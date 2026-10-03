//! Reproducible end-to-end timings and pixel baselines. Run with --reference before optimization.
use anyhow::{Result, ensure};
use hastur_retouch::{
    cleanup::CloneStamp,
    engine::{self, Background, Edit, ExportOptions, ExportSize, Settings, Stroke, Target},
    geometry::WarpStroke,
    model::{self, Provider},
};
use std::{hint::black_box, path::Path, time::Instant};

fn timed<T>(name: &str, report: &mut String, run: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = run();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("{name}: {ms:.2} ms");
    report.push_str(&format!("{name},{ms:.3}\n"));
    result
}
fn main() -> Result<()> {
    let reference = std::env::args().any(|a| a == "--reference");
    let label = if reference { "before" } else { "after" };
    let root = Path::new("output/performance");
    std::fs::create_dir_all(root.join("reference"))?;
    let mut report = "operation,milliseconds\n".to_owned();
    let photo = timed("import-demo", &mut report, || {
        engine::load_photo(Path::new("assets/demo-portrait.png"))
    })?;
    let seg = timed("ai-cache-load", &mut report, || {
        model::analyze_shared_cached(&photo.preview, Provider::Cpu, false, |_| {})
    })?;
    if std::env::args().any(|a| a == "--ai-first") {
        let inferred = timed("ai-inference-before-render", &mut report, || {
            model::analyze_shared_cached(&photo.preview, Provider::Cpu, true, |_| {})
        })?;
        validate_ai(&seg, &inferred)?;
    }
    let mut natural = Edit {
        settings: engine::presets()[0].settings.clone(),
        ..Default::default()
    };
    let color = Edit {
        settings: Settings {
            exposure: 0.4,
            contrast: 20.0,
            warmth: 12.0,
            saturation: -12.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let detail = Edit {
        settings: Settings {
            under_eyes: 100.0,
            forehead: 100.0,
            laugh_lines: 100.0,
            contour: 75.0,
            face_highlight: 65.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let reshape = Edit {
        settings: Settings {
            eye_size: 80.0,
            nose_width: -50.0,
            lip_plumpness: 75.0,
            jawline: 60.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut heal = Edit::default();
    let mut clone = Edit::default();
    let mut liquify = reshape.clone();
    let mut mask = Edit {
        settings: Settings {
            background: Background::Blur,
            ..Default::default()
        },
        ..Default::default()
    };
    for i in 0..80 {
        let center = [0.2 + (i % 20) as f32 * 0.025, 0.4 + (i / 20) as f32 * 0.09];
        heal.strokes.push(Stroke {
            target: Target::Heal,
            center,
            radius: 0.012,
            erase: false,
            strength: 100.0,
            softness: 0.65,
        });
        clone.clones.push(CloneStamp {
            center,
            source: [center[0] + 0.05, center[1] - 0.06],
            radius: 0.015,
            softness: 0.65,
            strength: 100.0,
        });
        liquify.warps.push(WarpStroke {
            center,
            delta: [0.003, -0.001],
            radius: 0.025,
            softness: 0.7,
            strength: 80.0,
        });
        mask.strokes.push(Stroke {
            target: Target::Background,
            center,
            radius: 0.02,
            erase: i % 3 == 0,
            strength: 100.0,
            softness: 0.7,
        });
    }
    natural.settings.blemishes = 80.0;
    let variants = [
        ("neutral", Edit::default()),
        ("color", color),
        ("neural", natural.clone()),
        ("details", detail),
        ("reshape", reshape),
        ("heal", heal),
        ("clone", clone),
        ("liquify", liquify),
        ("background", mask),
    ];
    for (name, edit) in &variants {
        let result = timed(&format!("preview-{name}"), &mut report, || {
            engine::render(&photo.preview, edit, Some(&seg))
        });
        let path = root.join("reference").join(format!("{name}.png"));
        if reference {
            result.save(path)?;
        } else {
            let baseline = image::open(path)?.into_rgba8();
            ensure!(
                baseline.dimensions() == result.dimensions(),
                "Dimensions changed for {name}"
            );
            let max = result
                .as_raw()
                .iter()
                .zip(baseline.as_raw())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            let mae = result
                .as_raw()
                .iter()
                .zip(baseline.as_raw())
                .map(|(a, b)| a.abs_diff(*b) as f64)
                .sum::<f64>()
                / result.as_raw().len() as f64;
            println!("  pixel comparison: maximum {max}, MAE {mae:.6}");
            ensure!(
                max <= 1 && mae < 0.05,
                "Visible output regression in {name}: max {max}, MAE {mae}"
            );
        }
    }
    timed("overlay-normal", &mut report, || {
        black_box(engine::mask_overlay(
            &photo.preview,
            &variants[8].1,
            Some(&seg),
            Target::Background,
        ))
    });
    timed("overlay-reshaped", &mut report, || {
        black_box(engine::mask_overlay(
            &photo.preview,
            &variants[7].1,
            Some(&seg),
            Target::Skin,
        ))
    });
    let full = timed("resize-large-fixture", &mut report, || {
        image::imageops::resize(
            &*photo.original,
            2244,
            2804,
            image::imageops::FilterType::Lanczos3,
        )
    });
    let large = timed("import-large-fixture", &mut report, || {
        engine::photo_from_image("performance.png".into(), None, full)
    });
    timed("full-resolution-neural", &mut report, || {
        black_box(engine::render(&large.original, &natural, Some(&seg)))
    });
    for png in [true, false] {
        let path = timed(
            if png { "export-png" } else { "export-jpeg" },
            &mut report,
            || {
                engine::export_with_options(
                    &large,
                    &natural,
                    Some(&seg),
                    &root.join(label),
                    &ExportOptions {
                        size: ExportSize::Web,
                        png,
                        quality: 95,
                        center: [0.5; 2],
                        cancel: None,
                    },
                )
            },
        )?;
        if !reference {
            let extension = if png { "png" } else { "jpg" };
            let before = image::open(
                root.join("before")
                    .join(format!("performance_v1.{extension}")),
            )?
            .into_rgba8();
            let after = image::open(path)?.into_rgba8();
            ensure!(before.dimensions() == after.dimensions());
            let max = before
                .as_raw()
                .iter()
                .zip(after.as_raw())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            let mae = before
                .as_raw()
                .iter()
                .zip(after.as_raw())
                .map(|(a, b)| a.abs_diff(*b) as f64)
                .sum::<f64>()
                / before.as_raw().len() as f64;
            println!("  {extension} export comparison: maximum {max}, MAE {mae:.8}");
            // One input-byte change can spread across a JPEG DCT block after quantization.
            ensure!(
                max <= if png { 1 } else { 8 } && mae < 0.05,
                "Export pixel regression: max {max}, MAE {mae}"
            );
        }
    }
    let shared_seg = std::sync::Arc::new(seg.clone());
    println!(
        "Stored AI maps: {:.2} MiB",
        seg.map_bytes() as f64 / (1024. * 1024.)
    );
    std::fs::write(
        root.join("storage.txt"),
        format!("AI map bytes: {}\n", seg.map_bytes()),
    )?;
    for _ in 0..3 {
        timed("ai-cache-repeat", &mut report, || {
            black_box(model::analyze_shared_cached(
                &photo.preview,
                Provider::Cpu,
                false,
                |_| {},
            ))
        })?;
    }
    let mut renderer = engine::Renderer::default();
    for (name, edit) in [
        ("neural", &variants[2].1),
        ("heal", &variants[5].1),
        ("liquify", &variants[7].1),
        ("background", &variants[8].1),
    ] {
        renderer.render(&photo.preview, edit, Some(&shared_seg), None)?;
        let mut samples = Vec::with_capacity(5);
        let mut result = None;
        for _ in 0..5 {
            let start = Instant::now();
            result = Some(renderer.render(&photo.preview, edit, Some(&shared_seg), None)?);
            samples.push(start.elapsed().as_secs_f64() * 1000.);
        }
        samples.sort_by(f64::total_cmp);
        println!("hot-preview-{name}: {:.2} ms (median of five)", samples[2]);
        report.push_str(&format!("hot-preview-{name},{:.3}\n", samples[2]));
        let uncached = timed(&format!("paired-uncached-{name}"), &mut report, || {
            engine::render(&photo.preview, edit, Some(&seg))
        });
        ensure!(result.unwrap() == uncached, "Hot preview differs in {name}");
    }
    if std::env::args().any(|a| a == "--inference") {
        let inferred = timed("ai-inference-after-export", &mut report, || {
            model::analyze_shared_cached(&photo.preview, Provider::Cpu, true, |s| {
                println!("{}", s.status)
            })
        })?;
        validate_ai(&seg, &inferred)?;
        println!("Real ONNX outputs agree with the original analysis.");
        std::fs::write(root.join("inference.csv"), &report)?;
    }
    let start = Instant::now();
    let mut pixel_count = 0;
    for _ in 0..30 {
        let copy = seg.clone();
        pixel_count += copy.skin.len();
        black_box(copy);
    }
    report.push_str(&format!(
        "thirty-segmentation-clones,{:.3}\n",
        start.elapsed().as_secs_f64() * 1000.0
    ));
    black_box(pixel_count);
    let long_edit = variants[7].1.clone();
    let start = Instant::now();
    for _ in 0..10000 {
        black_box(long_edit.clone());
    }
    report.push_str(&format!(
        "ten-thousand-edit-clones,{:.3}\n",
        start.elapsed().as_secs_f64() * 1000.0
    ));
    std::fs::write(root.join(format!("{label}.csv")), report)?;
    Ok(())
}

fn validate_ai(seg: &engine::Segmentation, inferred: &engine::Segmentation) -> Result<()> {
    ensure!(
        inferred.faces.len() == seg.faces.len() && inferred.map_crop == seg.map_crop,
        "Changed face registration"
    );
    for (a, b) in [
        (&seg.skin, &inferred.skin),
        (&seg.teeth, &inferred.teeth),
        (&seg.eyes, &inferred.eyes),
        (&seg.under_eyes, &inferred.under_eyes),
        (&seg.forehead, &inferred.forehead),
        (&seg.laugh_lines, &inferred.laugh_lines),
        (&seg.contour, &inferred.contour),
        (&seg.highlight, &inferred.highlight),
        (&seg.blemish, &inferred.blemish),
    ] {
        ensure!(a.len() == b.len());
        let max = a
            .iter()
            .zip(b.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        ensure!(max < 0.0005, "AI mask changed: {max}");
    }
    for (a, b) in [
        (&seg.neural_blend, &inferred.neural_blend),
        (&seg.repair_delta, &inferred.repair_delta),
    ] {
        ensure!(a.len() == b.len());
        let max = a
            .iter()
            .flatten()
            .zip(b.iter().flatten())
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        ensure!(max < 0.0005, "AI output changed: {max}");
    }
    Ok(())
}
