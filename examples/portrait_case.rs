//! Real-photo under-eye regression images and lightweight history timings.
use anyhow::{Result, ensure};
use hastur_retouch::{
    engine::{self, Edit, History, Target},
    model::{self, Provider},
};
use std::{hint::black_box, path::PathBuf, time::Instant};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "assets/demo-portrait.png".into()),
    );
    let output = PathBuf::from(args.next().unwrap_or_else(|| "output/portrait-case".into()));
    std::fs::create_dir_all(&output)?;
    let photo = engine::load_photo(&input)?;
    let seg = model::analyze_shared_cached(&photo.preview, Provider::Cpu, false, |s| {
        println!("{}", s.status)
    })?;
    ensure!(
        !seg.faces.is_empty(),
        "No face detected in {}",
        input.display()
    );
    std::fs::write(
        output.join("landmarks.ron"),
        ron::ser::to_string_pretty(&seg.faces, Default::default())?,
    )?;
    photo.preview.save(output.join("original.png"))?;
    engine::mask_overlay(
        &photo.preview,
        &Edit::default(),
        Some(&seg),
        Target::UnderEyes,
    )
    .save(output.join("under-eye-mask.png"))?;
    engine::mask_overlay(&photo.preview, &Edit::default(), Some(&seg), Target::Skin)
        .save(output.join("skin-mask.png"))?;
    engine::mask_overlay(&photo.preview, &Edit::default(), Some(&seg), Target::Eyes)
        .save(output.join("eye-mask.png"))?;
    let mut report = format!(
        "Input: {}\nOriginal: {:?}, preview: {:?}, faces: {}\n",
        input.display(),
        photo.original.dimensions(),
        photo.preview.dimensions(),
        seg.faces.len()
    );
    let b = seg.faces[0].bounds;
    let x = ((b[0] - b[2] * 0.1).max(0.) * photo.preview.width() as f32).floor() as u32;
    let y = ((b[1] - b[3] * 0.1).max(0.) * photo.preview.height() as f32).floor() as u32;
    let width = (b[2] * 1.2 * photo.preview.width() as f32).ceil() as u32;
    let height = (b[3] * 1.2 * photo.preview.height() as f32).ceil() as u32;
    let crop = |image: &image::RgbaImage| {
        image::imageops::crop_imm(
            image,
            x,
            y,
            width.min(image.width() - x),
            height.min(image.height() - y),
        )
        .to_image()
    };
    crop(&photo.preview).save(output.join("face-original.png"))?;
    let mut full_smoothing = Edit::default();
    full_smoothing.settings.smoothing = 100.0;
    let smoothed = engine::render(&photo.preview, &full_smoothing, Some(&seg));
    smoothed.save(output.join("skin-100.png"))?;
    crop(&smoothed).save(output.join("face-skin-100.png"))?;
    let mut auto_retouch = Edit::default();
    auto_retouch.settings.apply_auto_retouch();
    let automatic = engine::render(&photo.preview, &auto_retouch, Some(&seg));
    automatic.save(output.join("auto-retouch.png"))?;
    crop(&automatic).save(output.join("face-auto-retouch.png"))?;
    let auto_changed = automatic
        .pixels()
        .zip(photo.preview.pixels())
        .filter(|(a, b)| a != b)
        .count();
    report.push_str(&format!(
        "Auto Retouch: changed {auto_changed} pixels; contrast remains {:.1}; exposure remains {:.2}\n",
        auto_retouch.settings.contrast, auto_retouch.settings.exposure
    ));
    let mut under_eye_expected = Vec::with_capacity(3);
    for strength in [25., 50., 100.] {
        let mut edit = Edit::default();
        edit.settings.under_eyes = strength;
        let start = Instant::now();
        let rendered = engine::render(&photo.preview, &edit, Some(&seg));
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        let changed = rendered
            .pixels()
            .zip(photo.preview.pixels())
            .filter(|(a, b)| a != b)
            .count();
        let mae = rendered
            .as_raw()
            .iter()
            .zip(photo.preview.as_raw())
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .sum::<f64>()
            / rendered.as_raw().len() as f64;
        report.push_str(&format!(
            "Under-eyes {strength}: {changed} pixels, MAE {mae:.6}, render {elapsed:.2} ms\n"
        ));
        rendered.save(output.join(format!("under-eyes-{}.png", strength as u32)))?;
        crop(&rendered).save(output.join(format!("face-{}.png", strength as u32)))?;
        under_eye_expected.push(rendered);
    }
    let shared_seg = std::sync::Arc::new(seg.clone());
    let mut renderer = engine::Renderer::default();
    let mut cached_times = Vec::with_capacity(3);
    for (index, strength) in [25., 50., 100.].into_iter().enumerate() {
        let mut edit = Edit::default();
        edit.settings.under_eyes = strength;
        let start = Instant::now();
        let cached = renderer.render(&photo.preview, &edit, Some(&shared_seg), None)?;
        ensure!(
            cached == under_eye_expected[index],
            "Cached under-eye output differs"
        );
        black_box(cached);
        cached_times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    report.push_str(&format!(
        "Cached under-eye slider renders at 25/50/100: {:.2}/{:.2}/{:.2} ms\n",
        cached_times[0], cached_times[1], cached_times[2]
    ));
    let mut current = Edit::default();
    let mut history = History::default();
    for i in 0..100 {
        history.record(current.clone());
        current.settings.under_eyes = i as f32;
    }
    let start = Instant::now();
    for _ in 0..1000 {
        for _ in 0..100 {
            ensure!(history.undo(&mut current));
        }
        for _ in 0..100 {
            ensure!(history.redo(&mut current));
        }
    }
    report.push_str(&format!(
        "200,000 history state transitions: {:.2} ms\n",
        start.elapsed().as_secs_f64() * 1000.
    ));
    print!("{report}");
    std::fs::write(output.join("results.txt"), report)?;
    Ok(())
}
