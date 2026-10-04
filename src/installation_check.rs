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

    crate::ai_cache::save(&photo.preview, &seg, provider)?;
    ensure!(
        crate::ai_cache::load(&photo.preview, provider).is_some(),
        "User AI cache could not be reloaded"
    );
    report.push_str(&format!(
        "Writable AI cache: {}\n",
        crate::ai_cache::directory().display()
    ));

    for tool in [Target::Liquify, Target::Heal, Target::Clone, Target::Patch] {
        let mut edit = Edit::default();
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
                boundary: vec![[0.56, 0.42], [0.60, 0.42], [0.60, 0.46], [0.56, 0.46]],
                offset: [-0.035, 0.015],
                softness: 0.75,
                strength: 100.,
            }),
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
                center: [0.58, 0.44],
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
