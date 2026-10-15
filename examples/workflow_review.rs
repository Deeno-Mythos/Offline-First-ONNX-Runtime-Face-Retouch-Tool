//! Read-only workflow review using the local synthetic and bundled portraits.
use anyhow::{Result, ensure};
use hastur_retouch::{
    engine::{self, Adjustment, Crop, Edit, ExportSize, Segmentation, Stroke, Target},
    focus,
    model::{self, Provider},
    skin_tone::SkinTarget,
};
use image::RgbaImage;
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};

fn mae(a: &RgbaImage, b: &RgbaImage) -> f64 {
    a.as_raw()
        .iter()
        .zip(b.as_raw())
        .map(|(a, b)| a.abs_diff(*b) as f64)
        .sum::<f64>()
        / a.as_raw().len() as f64
}
fn check_region(
    image: &Arc<RgbaImage>,
    edit: &Edit,
    seg: &Arc<Segmentation>,
    full: &RgbaImage,
    crop: Crop,
) -> Result<()> {
    let region = engine::render_region(image, edit, Some(seg), crop, None)?;
    let expected =
        image::imageops::crop_imm(full, crop.x, crop.y, crop.width, crop.height).to_image();
    let max = region
        .as_raw()
        .iter()
        .zip(expected.as_raw())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    ensure!(max <= 1, "Native/full mismatch {max}");
    Ok(())
}
#[derive(Serialize)]
struct Session {
    version: u32,
    photos: Vec<SavedPhoto>,
    current: usize,
    compare: bool,
}
#[derive(Serialize)]
struct SavedPhoto {
    path: Option<PathBuf>,
    edit: Edit,
    rating: u8,
    selected: bool,
}
fn main() -> Result<()> {
    let root = std::env::current_dir()?;
    let output = root.join("output/workflow-upgrades/portrait-review");
    std::fs::create_dir_all(&output)?;
    let paths = [
        root.join("output/synthetic-fixtures/raw/portrait-01.png"),
        root.join("output/synthetic-fixtures/raw/portrait-02.png"),
        root.join("assets/demo-portrait.png"),
    ];
    let mut report = String::from(
        "Portrait validation: native original pixels; CPU ONNX; synthetic portraits plus bundled portrait.\n",
    );
    let mut pictures = Vec::new();
    let mut reference = None;
    for (index, path) in paths.iter().enumerate() {
        let dir = output.join(format!("portrait-{}", index + 1));
        std::fs::create_dir_all(&dir)?;
        let before = std::fs::read(path)?;
        let photo = engine::load_photo(path)?;
        let source = &photo.original;
        let seg = Arc::new(model::analyze_cached(
            &photo.preview,
            Provider::Cpu,
            true,
            |s| println!("{}: {}", index + 1, s.status),
        )?);
        ensure!(!seg.faces.is_empty(), "No face in portrait {}", index + 1);
        std::fs::write(dir.join("faces.ron"), ron::to_string(&seg.faces)?)?;
        let focused = focus::analyze(source, &seg.faces);
        let soft = focus::analyze(&image::imageops::blur(source.as_ref(), 4.), &seg.faces);
        let blurred = focus::analyze(&image::imageops::blur(source.as_ref(), 10.), &seg.faces);
        let scores = [
            focused.score.unwrap(),
            soft.score.unwrap(),
            blurred.score.unwrap(),
        ];
        ensure!(
            scores[0] > scores[1] && scores[1] > scores[2],
            "Focus ordering failed {scores:?}"
        );
        report.push_str(&format!(
            "Portrait {}: focus sharp/moderate/heavy {scores:?}; {} faces\n",
            index + 1,
            seg.faces.len()
        ));
        let original = engine::render(source, &Edit::default(), Some(&seg));
        ensure!(original == **source, "Neutral edits changed pixels");
        let face = &seg.faces[0];
        let crop = Crop {
            x: (face.bounds[0] * source.width() as f32) as u32,
            y: (face.bounds[1] * source.height() as f32) as u32,
            width: (face.bounds[2] * source.width() as f32) as u32,
            height: (face.bounds[3] * source.height() as f32) as u32,
        };
        image::imageops::crop_imm(source.as_ref(), crop.x, crop.y, crop.width, crop.height)
            .to_image()
            .save(dir.join("face-before.png"))?;
        let mut combined = Edit::default();
        for (name, a) in [
            ("skin", Adjustment::Smoothing),
            ("under-eyes", Adjustment::UnderEyes),
            ("forehead", Adjustment::Forehead),
            ("laugh-lines", Adjustment::LaughLines),
        ] {
            let mut previous = 0.;
            for strength in [50., 100.] {
                let mut edit = Edit::default();
                *edit.settings.value_mut(a) = strength;
                let full = engine::render(source, &edit, Some(&seg));
                let error = mae(&full, &original);
                ensure!(
                    error > previous && error > 0.,
                    "{name} strength has no increasing effect: {error}"
                );
                previous = error;
                check_region(source, &edit, &seg, &full, crop)?;
                image::imageops::crop_imm(&full, crop.x, crop.y, crop.width, crop.height)
                    .to_image()
                    .save(dir.join(format!("{name}-{}.png", strength as u32)))?;
                report.push_str(&format!(
                    "{name} {strength}: native MAE {error:.5}; region parity <=1 byte\n"
                ));
            }
            *combined.settings.value_mut(a) = if a == Adjustment::Smoothing { 40. } else { 65. };
        }
        if reference.is_none() {
            reference = Some(SkinTarget::from_face(
                "Portrait 1 source face".into(),
                source,
                &seg,
                0,
            )?);
        }
        combined.settings.skin_tone.target = reference.clone();
        combined.settings.skin_tone.strength = 65.;
        let match_only = Edit {
            settings: engine::Settings {
                skin_tone: combined.settings.skin_tone.clone(),
                ..Default::default()
            },
            ..Default::default()
        };
        let matched = engine::render(source, &match_only, Some(&seg));
        let mut lightness_error = 0.;
        let mut skin_samples = 0;
        for y in (0..source.height()).step_by(7) {
            for x in (0..source.width()).step_by(7) {
                let a = source.get_pixel(x, y);
                let b = matched.get_pixel(x, y);
                ensure!(
                    seg.width == source.width() && seg.height == source.height(),
                    "Review expects native-size maps"
                );
                let map = seg.map_crop.unwrap_or(Crop {
                    x: 0,
                    y: 0,
                    width: seg.width,
                    height: seg.height,
                });
                let mask = if x >= map.x
                    && y >= map.y
                    && x < map.x + map.width
                    && y < map.y + map.height
                {
                    seg.skin[((y - map.y) * map.width + x - map.x) as usize]
                } else {
                    0.
                };
                if mask == 0. {
                    ensure!(a == b, "Skin match leaked outside mask");
                }
                if mask > 0.8 {
                    let lab = |p: &image::Rgba<u8>| {
                        hastur_retouch::skin_tone::to_lab([
                            engine::linear_byte(p[0]),
                            engine::linear_byte(p[1]),
                            engine::linear_byte(p[2]),
                            1.,
                        ])
                    };
                    lightness_error += (lab(a)[0] - lab(b)[0]).abs() as f64;
                    skin_samples += 1;
                }
            }
        }
        ensure!(
            skin_samples > 20 && lightness_error / (skin_samples as f64) < 0.002,
            "Skin brightness changed excessively"
        );
        matched.save(dir.join("skin-match.png"))?;
        report.push_str(&format!(
            "Skin target: {} samples; mean Oklab lightness change {:.6}; outside mask identical\n",
            skin_samples,
            lightness_error / skin_samples as f64
        ));
        for target in [
            Target::Clothing,
            Target::Dodge,
            Target::Burn,
            Target::StrayHair,
        ] {
            let (center, radius) = match target {
                Target::Clothing => ([0.23, 0.68], 0.12),
                Target::Dodge | Target::Burn => ([0.43, 0.33], 0.07),
                _ => ([0.23, 0.26], 0.15),
            };
            let mut edit = Edit::default();
            let mut previous = 0.;
            for strength in [50., 100.] {
                edit.strokes.clear();
                edit.strokes.push(Stroke {
                    target,
                    center,
                    radius,
                    softness: 0.7,
                    strength,
                    erase: false,
                });
                let full = engine::render(source, &edit, Some(&seg));
                let error = mae(&full, &original);
                ensure!(
                    error > previous,
                    "{target:?} absent or weaker at {strength}: {error}"
                );
                previous = error;
                check_region(
                    source,
                    &edit,
                    &seg,
                    &full,
                    Crop {
                        x: 90,
                        y: 120,
                        width: 600,
                        height: 950,
                    },
                )?;
                full.save(dir.join(format!("{target:?}-{}.png", strength as u32)))?;
                report.push_str(&format!(
                    "{target:?} {strength}: native MAE {error:.6}; native/full parity\n"
                ));
            }
        }
        let full = engine::render(source, &combined, Some(&seg));
        full.save(dir.join("combined.png"))?;
        let exported = engine::export(
            &photo,
            &combined,
            Some(&seg),
            &dir,
            ExportSize::Original,
            true,
            95,
        )?;
        ensure!(
            image::open(exported)?.to_rgba8() == full,
            "PNG export differs from native render"
        );
        ensure!(std::fs::read(path)? == before, "Source file modified");
        pictures.push(SavedPhoto {
            path: Some(path.clone()),
            edit: combined,
            rating: 0,
            selected: true,
        });
    }
    // An upscaled stress fixture exercises >2.1MP tiled export and native viewport paths.
    let source = image::open(&paths[0])?.to_rgba8();
    let big = Arc::new(image::imageops::resize(
        &source,
        2244,
        2804,
        image::imageops::FilterType::Lanczos3,
    ));
    big.save(output.join("stress-upscaled.png"))?;
    let seg = Arc::new(Segmentation::default());
    let mut edit = Edit::default();
    for target in [Target::Clothing, Target::Dodge, Target::Burn] {
        edit.strokes.push(Stroke {
            target,
            center: [0.35, 0.68],
            radius: 0.1,
            softness: 0.7,
            strength: 80.,
            erase: false,
        });
    }
    let full = engine::render(&big, &edit, None);
    check_region(
        &big,
        &edit,
        &seg,
        &full,
        Crop {
            x: 420,
            y: 1540,
            width: 700,
            height: 550,
        },
    )?;
    report.push_str("6.3MP upscaled stress fixture: tiled/native parity <=1 byte. Source files unchanged; PNG exports identical.\n");
    std::fs::write(
        output.join("review-session.ron"),
        ron::ser::to_string_pretty(
            &Session {
                version: 5,
                photos: pictures,
                current: 0,
                compare: false,
            },
            Default::default(),
        )?,
    )?;
    std::fs::write(output.join("results.txt"), report)?;
    println!("Workflow review passed: {}", output.display());
    Ok(())
}
