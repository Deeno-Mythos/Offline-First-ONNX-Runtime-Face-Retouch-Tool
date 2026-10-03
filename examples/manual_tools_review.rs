//! Read-only saved-workspace diagnosis or reproducible real-portrait tool review.
use anyhow::{Result, ensure};
use hastur_retouch::{
    cleanup::{CloneStamp, PatchStroke},
    engine::{self, Edit, History, Stroke, Target},
    geometry::WarpStroke,
    interaction::View,
};
use image::RgbaImage;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

#[derive(Deserialize)]
struct Workspace {
    photos: Vec<SavedPhoto>,
    #[serde(default)]
    current: usize,
    #[serde(default)]
    compare: bool,
}
#[derive(Deserialize)]
struct SavedPhoto {
    path: Option<PathBuf>,
    edit: Edit,
    view: View,
}
fn changes(a: &RgbaImage, b: &RgbaImage) -> usize {
    a.pixels().zip(b.pixels()).filter(|(a, b)| a != b).count()
}
fn tool_edit(target: Target, center: [f32; 2]) -> Edit {
    let mut edit = Edit::default();
    match target {
        Target::Liquify => edit.warps.push(WarpStroke {
            center,
            delta: [0.018, 0.008],
            radius: 0.06,
            softness: 0.7,
            strength: 100.,
        }),
        Target::Heal => edit.strokes.push(Stroke {
            target,
            center,
            radius: 0.009,
            softness: 0.65,
            strength: 100.,
            erase: false,
        }),
        Target::Clone => edit.clones.push(CloneStamp {
            center,
            source: [center[0] - 0.024, center[1] + 0.022],
            radius: 0.012,
            softness: 0.7,
            strength: 100.,
        }),
        Target::Patch => edit.patches.push(PatchStroke {
            boundary: vec![
                [center[0] - 0.014, center[1] - 0.014],
                [center[0] + 0.014, center[1] - 0.014],
                [center[0] + 0.014, center[1] + 0.014],
                [center[0] - 0.014, center[1] + 0.014],
            ],
            offset: [0.025, 0.010],
            softness: 0.75,
            strength: 100.,
        }),
        _ => unreachable!(),
    }
    edit.ensure_stack();
    edit.active_layer_mut().unwrap().name = target.label().into();
    edit.sync_active_layer();
    edit
}
fn saved_review(path: &Path, edit: &Edit) -> Result<String> {
    Ok(format!(
        "(version:2,photos:[(path:Some({}),edit:{},rating:0)],compare:false)",
        ron::to_string(path)?,
        ron::to_string(edit)?
    ))
}
fn diagnose(path: &Path, root: &Path) -> Result<()> {
    let workspace: Workspace =
        ron::de::from_reader(std::io::BufReader::new(std::fs::File::open(path)?))?;
    println!(
        "Photos={} active={} compare={}",
        workspace.photos.len(),
        workspace.current,
        workspace.compare
    );
    for (index, p) in workspace.photos.iter().enumerate() {
        println!(
            "Photo {index}: zoom={} active={} locked={} heal={} liquify={} clone={} patch={}",
            p.view.scale,
            p.edit.stack.active,
            p.edit.layer_locked(),
            p.edit.strokes.len(),
            p.edit.warps.len(),
            p.edit.clones.len(),
            p.edit.patches.len()
        );
        for layer in &p.edit.stack.layers {
            println!(
                "  Layer {} {:?}: visible={} opacity={} locked={}",
                layer.id, layer.kind, layer.visible, layer.opacity, layer.locked
            );
        }
        if index != workspace.current {
            continue;
        }
        let source_path = p
            .path
            .clone()
            .unwrap_or_else(|| PathBuf::from("example-img/demo-portrait.png"));
        let photo = engine::load_photo(&source_path)?;
        let before = engine::render(&photo.preview, &p.edit, None);
        let mut edit = p.edit.clone();
        edit.warps.push(WarpStroke {
            center: [0.55, 0.5],
            delta: [0.04, 0.02],
            radius: 0.1,
            softness: 0.5,
            strength: 100.,
        });
        let after = engine::render(&photo.preview, &edit, None);
        println!("  Added warp changes {} pixels", changes(&before, &after));
        if let Some(layer) = edit.active_layer_mut() {
            layer.visible = true;
        }
        let revealed = engine::render(&photo.preview, &edit, None);
        println!(
            "  Revealing a cloned active layer changes {} pixels",
            changes(&before, &revealed)
        );
        revealed.save(root.join("revealed-layer-preview.png"))?;
    }
    Ok(())
}
fn main() -> Result<()> {
    let root = Path::new("output/manual-tools");
    std::fs::create_dir_all(root)?;
    if let Some(path) = std::env::args().nth(1) {
        return diagnose(Path::new(&path), root);
    }
    let mut report = String::from(
        "photo,tool,overview_changed_pixels,native_changed_pixels,overview_ms,native_ms\n",
    );
    for (name, path, center) in [
        ("demo", "example-img/demo-portrait.png", [0.62, 0.43]),
        (
            "original",
            "example-img/CTU DUMANJUG ORG 09.28.26_JAMESBRO-522.JPG",
            [0.54, 0.345],
        ),
    ] {
        let absolute = std::env::current_dir()?.join(path);
        let photo = engine::load_photo(&absolute)?;
        let original = photo.original.clone();
        photo
            .preview
            .save(root.join(format!("{name}-before.png")))?;
        let (w, h) = original.dimensions();
        let crop = engine::Crop {
            x: ((center[0] * w as f32) as u32)
                .saturating_sub(256)
                .min(w.saturating_sub(512)),
            y: ((center[1] * h as f32) as u32)
                .saturating_sub(256)
                .min(h.saturating_sub(512)),
            width: 512.min(w),
            height: 512.min(h),
        };
        let native_before =
            image::imageops::crop_imm(&*original, crop.x, crop.y, crop.width, crop.height)
                .to_image();
        native_before.save(root.join(format!("{name}-native-before.png")))?;
        let mut renderer = engine::Renderer::default();
        for (target, label) in [
            (Target::Liquify, "liquify"),
            (Target::Heal, "heal"),
            (Target::Clone, "clone"),
            (Target::Patch, "patch"),
        ] {
            let mut edit = tool_edit(target, center);
            let mut history = History::default();
            history.record(Edit::default());
            let started = Instant::now();
            let output = renderer.render(&photo.preview, &edit, None, None)?;
            let overview_ms = started.elapsed().as_secs_f64() * 1000.;
            let count = changes(&photo.preview, &output);
            ensure!(count > 0, "{name} {label} did not change pixels");
            ensure!(
                output == engine::render(&photo.preview, &edit, None),
                "Cached overview diverged"
            );
            output.save(root.join(format!("{name}-{label}.png")))?;
            let started = Instant::now();
            let native = renderer.render_region(&original, &edit, None, crop, None)?;
            let native_ms = started.elapsed().as_secs_f64() * 1000.;
            let native_count = changes(&native_before, &native);
            ensure!(native_count > 0, "Native {label} did not change pixels");
            native.save(root.join(format!("{name}-native-{label}.png")))?;
            let stored = edit.clone();
            ensure!(
                history.undo(&mut edit) && edit == Edit::default(),
                "Undo failed"
            );
            ensure!(history.redo(&mut edit) && edit == stored, "Redo failed");
            let restored: Edit = ron::from_str(&ron::to_string(&edit)?)?;
            ensure!(restored == edit, "Session instructions changed");
            ensure!(
                Arc::ptr_eq(&original, &photo.original),
                "Original was replaced"
            );
            ensure!(
                output.dimensions() == photo.preview.dimensions()
                    && output
                        .pixels()
                        .zip(photo.preview.pixels())
                        .all(|(a, b)| a[3] == b[3]),
                "Geometry/alpha changed"
            );
            std::fs::write(
                root.join(format!("{name}-{label}.ron")),
                saved_review(&absolute, &edit)?,
            )?;
            if name == "demo" && target == Target::Liquify {
                edit.active_layer_mut().unwrap().visible = false;
                ensure!(
                    engine::render(&photo.preview, &edit, None) == *photo.preview,
                    "Hidden layer should preserve original"
                );
                std::fs::write(
                    root.join("hidden-layer.ron"),
                    saved_review(&absolute, &edit)?,
                )?;
            }
            println!(
                "{name} {label}: {count} overview / {native_count} native pixels changed; {overview_ms:.2} / {native_ms:.2} ms"
            );
            report.push_str(&format!(
                "{name},{label},{count},{native_count},{overview_ms:.2},{native_ms:.2}\n"
            ));
        }
    }
    std::fs::write(root.join("validation.csv"), report)?;
    Ok(())
}
