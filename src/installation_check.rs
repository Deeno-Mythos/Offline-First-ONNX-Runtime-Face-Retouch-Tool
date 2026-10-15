//! Opt-in diagnostics on the installed binary using only the embedded demo.
use crate::{
    cleanup::{CloneStamp, PatchStroke},
    engine::{self, Edit, ExportSize, History, Stroke, Target},
    geometry::WarpStroke,
    layer_mask::{LayerMask, MaskStroke},
    model::{self, Provider},
    session::{self, SavedPhoto, Session},
};
use anyhow::{Context, Result, ensure};
use std::{fs, path::Path, time::Instant};

pub fn run(output: &Path, provider: Provider) -> Result<()> {
    fs::create_dir_all(output)?;
    let mut report = format!(
        "Hastur Retouch {}\nProvider requested: {}\n",
        env!("CARGO_PKG_VERSION"),
        provider.label()
    );
    let result = check(output, provider, &mut report);
    match &result {
        Ok(()) => report.push_str("RESULT: PASS\n"),
        Err(error) => report.push_str(&format!("RESULT: FAIL\n{error:#}\n")),
    }
    fs::write(output.join("installation-check.txt"), report)?;
    result
}

fn check(output: &Path, provider: Provider, report: &mut String) -> Result<()> {
    let executable = std::env::current_exe()?;
    let installed = executable
        .parent()
        .context("Executable directory missing")?;
    let models = installed.join("models");
    for stem in [
        "face_detection_short_range",
        "face_landmarker_Nx3x256x256",
        "retouch_generator",
        "local_detection",
        "local_inpainting",
    ] {
        let name = format!("{stem}.onnx");
        let path = model::model_path(&name);
        ensure!(
            path == models.join(&name),
            "Models must resolve inside the installation: {}",
            path.display()
        );
        ensure!(fs::metadata(&path)?.len() > 0, "Empty model: {name}");
        ensure!(
            fs::metadata(models.join("burn").join(format!("{stem}.bpk")))?.len() > 0,
            "Missing Burn pack: {stem}"
        );
    }
    for name in [
        "onnxruntime.dll",
        "onnxruntime_providers_shared.dll",
        "directml/onnxruntime.dll",
        "directml/onnxruntime_providers_shared.dll",
        "directml/DirectML.dll",
    ] {
        ensure!(
            models.join(name).is_file(),
            "Missing bundled runtime: {name}"
        );
    }
    report.push_str("All ONNX models, Burn packs and CPU/DirectML runtimes are bundled.\n");

    let photo = engine::photo_from_image(
        "Installation demo.png".into(),
        None,
        image::load_from_memory(include_bytes!("../assets/demo-portrait.png"))?.into_rgba8(),
    );
    let original = photo.original.as_ref();
    ensure!(
        engine::render(original, &Edit::default(), None) == *original,
        "Neutral render altered the source"
    );
    let start = Instant::now();
    let seg = model::analyze_portrait(&photo.preview, provider, |_| {})?;
    ensure!(
        !seg.faces.is_empty() && !seg.neural_blend.is_empty() && !seg.repair_delta.is_empty(),
        "Portrait pipeline did not produce complete maps"
    );
    report.push_str(&format!(
        "Portrait inference: {:.3}s; {}\n",
        start.elapsed().as_secs_f64(),
        seg.status
    ));
    let transform = crate::image_transform::ImageTransform {
        size: Some([photo.dimensions().0 / 2, photo.dimensions().1 / 2]),
        crop: Some(crate::image_transform::CropFrame {
            center: [0.5; 2],
            extent: [0.65, 0.7],
            angle: 17.,
        }),
        ..Default::default()
    };
    let working = photo.transformed(transform, None)?;
    let edit = Edit {
        transform,
        settings: engine::Settings {
            under_eyes: 40.,
            smoothing: 30.,
            ..Default::default()
        },
        ..Default::default()
    };
    let aligned = seg.transformed(transform, photo.dimensions());
    let expected = engine::render(&working.original, &edit, Some(&aligned));
    let export = engine::export(
        &photo,
        &edit,
        Some(&seg),
        &output.join("geometry"),
        ExportSize::Original,
        true,
        100,
    )?;
    ensure!(
        *engine::load_photo(&export)?.original == expected,
        "Resize/crop export does not match the working document with AI maps"
    );
    ensure!(
        photo.transformed(Default::default(), None)?.original == photo.original,
        "Reset image geometry altered the source"
    );
    report.push_str("Resize/crop: frame-only rotation, AI map alignment, document dimensions, lossless export and source preservation passed.\n");

    let (w, h) = photo.dimensions();
    let translated = crate::image_transform::ImageTransform {
        crop: Some(crate::image_transform::CropFrame {
            center: [0.5 - 10. / w as f32, 0.5 - 7. / h as f32],
            ..Default::default()
        }),
        ..Default::default()
    };
    let moved = photo.transformed(translated, None)?;
    ensure!(
        moved.dimensions() == (w, h)
            && moved.original.get_pixel(0, 0).0 == [0; 4]
            && moved.original.get_pixel(10, 7) == photo.original.get_pixel(0, 0),
        "Moving the full crop frame changed its size, lost transparency or resampled the photo"
    );
    let export = engine::export(
        &photo,
        &Edit {
            transform: translated,
            ..Default::default()
        },
        None,
        &output.join("frame-movement"),
        ExportSize::Original,
        true,
        100,
    )?;
    ensure!(
        *engine::load_photo(&export)?.original == *moved.original,
        "Moved crop export differs from the workspace"
    );
    report.push_str("Frame movement: full-size X/Y translation, transparent margins, stable dimensions, native pixels and PNG export passed.\n");

    crate::ai_cache::save(&photo.preview, &seg, provider)?;
    ensure!(
        crate::ai_cache::load(&photo.preview, provider).is_some(),
        "User AI cache could not be reloaded"
    );
    report.push_str(&format!(
        "Writable AI cache: {}\n",
        crate::ai_cache::directory().display()
    ));

    let focus = crate::focus::analyze(original, &seg.faces);
    ensure!(focus.score.is_some(), "Detected face has no focus score");
    report.push_str(&format!("Face focus: {}\n", focus.summary()));
    for tool in [
        Target::Liquify,
        Target::Heal,
        Target::Clone,
        Target::Patch,
        Target::Clothing,
        Target::Dodge,
        Target::Burn,
        Target::StrayHair,
    ] {
        let mut edit = Edit::default();
        let center = match tool {
            Target::Clothing => [0.35, 0.8],
            Target::StrayHair => [0.11, 0.47],
            _ => [0.58, 0.44],
        };
        match tool {
            Target::Liquify => edit.warps.push(WarpStroke {
                center: [0.55, 0.43],
                delta: [0.018, 0.008],
                radius: 0.06,
                softness: 0.7,
                strength: 100.,
            }),
            Target::Heal => edit.strokes.push(Stroke {
                target: tool,
                center: [0.58, 0.44],
                radius: 0.014,
                softness: 0.65,
                strength: 100.,
                erase: false,
            }),
            Target::Clone => edit.clones.push(CloneStamp {
                center: [0.58, 0.44],
                source: [0.55, 0.46],
                radius: 0.02,
                softness: 0.7,
                strength: 100.,
            }),
            Target::Patch => edit.patches.push(PatchStroke {
                feather: None,
                boundary: vec![[0.56, 0.42], [0.60, 0.42], [0.60, 0.46], [0.56, 0.46]],
                offset: [-0.035, 0.015],
                softness: 0.75,
                strength: 100.,
            }),
            Target::Clothing | Target::Dodge | Target::Burn | Target::StrayHair => {
                edit.strokes.push(Stroke {
                    target: tool,
                    center,
                    radius: 0.07,
                    softness: 0.7,
                    strength: 100.,
                    erase: false,
                })
            }
            _ => unreachable!(),
        }
        edit.ensure_stack();
        edit.sync_active_layer();
        let rendered = engine::render(original, &edit, None);
        ensure!(
            rendered != *original && rendered.dimensions() == original.dimensions(),
            "{} did not change native pixels",
            tool.label()
        );
        let mut history = History::default();
        history.record(Edit::default());
        ensure!(history.undo(&mut edit), "{} undo failed", tool.label());
        ensure!(
            engine::render(original, &edit, None) == *original,
            "{} undo changed source pixels",
            tool.label()
        );
        ensure!(history.redo(&mut edit), "{} redo failed", tool.label());
        ensure!(
            engine::render(original, &edit, None) == rendered,
            "{} redo differs",
            tool.label()
        );
        let layer = edit.active_layer_mut().context("Retouch layer missing")?;
        layer.mask = Some(LayerMask {
            base: 0,
            ..Default::default()
        });
        ensure!(
            engine::render(original, &edit, None) == *original,
            "Black layer mask failed"
        );
        edit.active_layer_mut()
            .unwrap()
            .mask
            .as_mut()
            .unwrap()
            .strokes
            .push(MaskStroke {
                center,
                radius: 0.15,
                softness: 0.5,
                strength: 100.,
                reveal: true,
            });
        let masked = engine::render(original, &edit, None);
        ensure!(masked != *original, "Reveal layer mask failed");
        let workspace = Session {
            version: session::VERSION,
            photos: vec![SavedPhoto {
                path: None,
                edit: edit.clone(),
                rating: 0,
                segmentation: None,
                view: Default::default(),
                crop_center: [0.5; 2],
                history,
                selected: true,
            }],
            current: 0,
            compare: false,
            split: 0.5,
            clean_shutdown: true,
        };
        let session_path = output.join(format!("{}-workspace.ron", tool.label()));
        session::save(&session_path, &workspace)?;
        let mut restored = session::read(&session_path)?;
        let saved = &mut restored.photos[0];
        ensure!(
            engine::render(original, &saved.edit, None) == masked,
            "Saved layer pixels differ"
        );
        ensure!(
            saved.history.undo(&mut saved.edit) && saved.history.redo(&mut saved.edit),
            "Saved history failed"
        );
        // The mask was added after recording the tool gesture, so redo must restore it.
        ensure!(
            engine::render(original, &saved.edit, None) == masked,
            "Restored redo lost the mask"
        );
        let export = engine::export(
            &photo,
            &edit,
            None,
            &output.join(tool.label()),
            ExportSize::Original,
            true,
            95,
        )?;
        ensure!(
            *engine::load_photo(&export)?.original == masked,
            "Lossless native export differs"
        );
        report.push_str(&format!(
            "{}: native render, undo/redo, layer mask, save/reopen and PNG export passed.\n",
            tool.label()
        ));
    }
    let recovery = output.join("recovery-check.ron");
    let empty = Session {
        version: session::VERSION,
        photos: vec![],
        current: 0,
        compare: false,
        split: 0.5,
        clean_shutdown: false,
    };
    session::save_recovery(&recovery, &empty, 1)?;
    session::save_recovery(&recovery, &empty, 2)?;
    fs::write(&recovery, b"interrupted write")?;
    ensure!(
        session::read_recovery(&recovery)?.version == session::VERSION,
        "Previous-generation recovery failed"
    );
    ensure!(photo.original.as_ref() == original, "Source pixels mutated");
    report.push_str("Interrupted recovery falls back to the previous complete generation.\n");
    Ok(())
}
