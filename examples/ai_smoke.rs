use anyhow::Result;
use hastur_retouch::{
    engine::{self, Adjustment, Edit},
    model::{self, Provider},
};
fn main() -> Result<()> {
    let photo = engine::load_photo(std::path::Path::new("assets/demo-portrait.png"))?;
    let start = std::time::Instant::now();
    let provider = if std::env::args().any(|a| a == "--directml") {
        Provider::DirectMl
    } else if std::env::args().any(|a| a == "--burn") {
        Provider::Burn
    } else {
        Provider::Cpu
    };
    let seg = model::analyze_cached(&photo.preview, provider, true, |s| println!("{}", s.status))?;
    println!("{} in {:.2}s", seg.status, start.elapsed().as_secs_f32());
    anyhow::ensure!(!seg.faces.is_empty(), "Demo face detection failed");
    std::fs::create_dir_all("output/ai-validation")?;
    std::fs::write(
        "output/ai-validation/landmarks.ron",
        ron::ser::to_string_pretty(&seg.faces, Default::default())?,
    )?;
    let mut points = photo.preview.as_ref().clone();
    for f in &seg.faces {
        println!("Face confidence {:.4}, bounds {:?}", f.confidence, f.bounds);
        for p in &f.landmarks {
            let x = (p[0] * points.width() as f32) as i32;
            let y = (p[1] * points.height() as f32) as i32;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if x + dx >= 0
                        && y + dy >= 0
                        && x + dx < (points.width() as i32)
                        && y + dy < (points.height() as i32)
                    {
                        points.put_pixel(
                            (x + dx) as u32,
                            (y + dy) as u32,
                            image::Rgba([255, 220, 0, 255]),
                        );
                    }
                }
            }
        }
    }
    points.save("output/ai-validation/mesh.png")?;
    engine::mask_overlay(
        &photo.preview,
        &Edit::default(),
        Some(&seg),
        engine::Target::Skin,
    )
    .save("output/ai-validation/skin-mask.png")?;
    let mut report = format!(
        "{}\nInference time: {:.2}s\n",
        seg.status,
        start.elapsed().as_secs_f32()
    );
    let baseline = engine::render(&photo.preview, &Edit::default(), Some(&seg));
    baseline.save("output/ai-validation/original.png")?;
    for (name, adjustment) in [
        ("skin", Adjustment::Smoothing),
        ("blemishes", Adjustment::Blemishes),
        ("under-eyes", Adjustment::UnderEyes),
        ("forehead", Adjustment::Forehead),
        ("laugh-lines", Adjustment::LaughLines),
        ("contour", Adjustment::Contour),
        ("highlight", Adjustment::FaceHighlight),
        ("eye-size", Adjustment::EyeSize),
        ("nose-width", Adjustment::NoseWidth),
        ("lips", Adjustment::LipPlumpness),
        ("jawline", Adjustment::Jawline),
    ] {
        let mut previous = 0.0;
        for strength in [50.0, 100.0] {
            let mut edit = Edit::default();
            *edit.settings.value_mut(adjustment) = strength;
            let result = engine::render(&photo.preview, &edit, Some(&seg));
            let changed = result
                .pixels()
                .zip(baseline.pixels())
                .filter(|(a, b)| a != b)
                .count();
            let mae = result
                .as_raw()
                .iter()
                .zip(baseline.as_raw())
                .map(|(a, b)| (*a as f64 - *b as f64).abs())
                .sum::<f64>()
                / (result.width() * result.height() * 4) as f64;
            println!("{name} {strength}: {changed} pixels, MAE {mae:.4}");
            report.push_str(&format!(
                "{name} {strength}: {changed} changed pixels, MAE {mae:.4}\n"
            ));
            anyhow::ensure!(
                changed > 500 && mae > previous,
                "{name} effect absent or not increasing"
            );
            previous = mae;
            result.save(format!(
                "output/ai-validation/{name}-{}.png",
                strength as u32
            ))?;
        }
    }
    let edit = Edit {
        settings: engine::presets()[0].settings.clone(),
        ..Default::default()
    };
    engine::render(&photo.preview, &edit, Some(&seg)).save("output/ai-validation/natural.png")?;
    let full = image::imageops::resize(
        &*photo.original,
        photo.original.width() * 2,
        photo.original.height() * 2,
        image::imageops::FilterType::Lanczos3,
    );
    let full_photo = engine::photo_from_image("AI full resolution.png".into(), None, full);
    let before = full_photo.original.as_raw().clone();
    let mut export_edit = edit.clone();
    export_edit.settings.eye_size = 45.0;
    export_edit.settings.nose_width = -30.0;
    export_edit
        .warps
        .push(hastur_retouch::geometry::WarpStroke {
            center: [0.18, 0.72],
            delta: [0.008, 0.0],
            radius: 0.08,
            softness: 0.7,
            strength: 100.0,
        });
    export_edit
        .clones
        .push(hastur_retouch::cleanup::CloneStamp {
            center: [0.56, 0.82],
            source: [0.48, 0.82],
            radius: 0.015,
            softness: 0.7,
            strength: 100.0,
        });
    let path = engine::export_with_options(
        &full_photo,
        &export_edit,
        Some(&seg),
        std::path::Path::new("output/ai-validation"),
        &engine::ExportOptions {
            size: engine::ExportSize::Original,
            png: true,
            quality: 95,
            center: [0.5; 2],
            cancel: None,
        },
    )?;
    let exported = image::open(&path)?.into_rgba8();
    anyhow::ensure!(
        exported.dimensions() == full_photo.original.dimensions(),
        "AI export lost source resolution"
    );
    anyhow::ensure!(
        *full_photo.original.as_raw() == before,
        "AI/manual edits mutated original"
    );
    let expected = engine::render(&full_photo.original, &export_edit, Some(&seg));
    anyhow::ensure!(exported == expected, "AI export differs from renderer");
    report.push_str(&format!(
        "AI + liquify + clone export matched renderer at {}x{}; original remained byte-identical\n",
        exported.width(),
        exported.height()
    ));
    std::fs::write("output/ai-validation/results.txt", report)?;
    Ok(())
}
