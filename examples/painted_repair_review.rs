//! Native-pixel, no-inference review of explicitly painted defects.
use anyhow::{Result, ensure};
use hastur_retouch::engine::{self, Crop, Edit, Renderer, Stroke, Target};
use image::RgbaImage;
use std::{path::PathBuf, time::Instant};

fn main() -> Result<()> {
    let out = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output/manual-repair/after"));
    std::fs::create_dir_all(&out)?;
    let mut report = String::new();
    for (name, path, target, points, radius, crop) in [
        (
            "sleeve",
            "output/synthetic-fixtures/raw/portrait-01.png",
            Target::Clothing,
            vec![[145., 890.], [180., 896.], [220., 902.], [260., 905.]],
            40.,
            Crop {
                x: 120,
                y: 810,
                width: 200,
                height: 200,
            },
        ),
        (
            "clothing",
            "output/synthetic-fixtures/raw/portrait-01.png",
            Target::Clothing,
            vec![[390., 1190.], [416., 1164.], [440., 1136.], [461., 1106.]],
            48.,
            Crop {
                x: 330,
                y: 1050,
                width: 200,
                height: 200,
            },
        ),
        (
            "hair",
            "assets/demo-portrait.png",
            Target::StrayHair,
            vec![[106., 439.], [114., 447.], [123., 455.], [133., 461.]],
            7.,
            Crop {
                x: 90,
                y: 420,
                width: 90,
                height: 100,
            },
        ),
    ] {
        let source = engine::load_photo(&PathBuf::from(path))?.original;
        let mut edit = Edit::default();
        let short = source.width().min(source.height()) as f32;
        let mut last = None;
        for point in points {
            let uv = [
                point[0] / source.width() as f32,
                point[1] / source.height() as f32,
            ];
            let mut stamps = if last.is_none() { vec![uv] } else { vec![] };
            stamps.extend(hastur_retouch::interaction::stamps(
                last,
                uv,
                [source.width(), source.height()],
                radius / short,
            ));
            for center in stamps {
                edit.strokes.push(Stroke {
                    target,
                    center,
                    radius: radius / short,
                    softness: 0.5,
                    erase: false,
                    strength: 100.,
                });
            }
            last = Some(uv);
        }
        let native = |image: &RgbaImage| {
            image::imageops::crop_imm(image, crop.x, crop.y, crop.width, crop.height).to_image()
        };
        native(&source).save(out.join(format!("{name}-before.png")))?;
        let mut renderer = Renderer::default();
        for strength in [50., 100.] {
            let mut edited = edit.clone();
            if target == Target::Clothing {
                edited.settings.clothing_wrinkles = strength;
            } else {
                edited.settings.healing = strength;
            }
            let start = Instant::now();
            let image = renderer.render(&source, &edited, None, None)?;
            let ms = start.elapsed().as_secs_f64() * 1000.;
            let expected = native(&image);
            let tile = renderer.render_region(&source, &edited, None, crop, None)?;
            ensure!(tile == expected, "native/full pixels differ for {name}");
            expected.save(out.join(format!("{name}-{strength:.0}.png")))?;
            image.save(out.join(format!("{name}-full-{strength:.0}.png")))?;
            let changed = image
                .pixels()
                .zip(source.pixels())
                .filter(|(a, b)| a != b)
                .count();
            let original = native(&source);
            let mae = expected
                .as_raw()
                .iter()
                .zip(original.as_raw())
                .map(|(a, b)| a.abs_diff(*b) as f64)
                .sum::<f64>()
                / expected.as_raw().len() as f64;
            report.push_str(&format!("{name} {strength}: {ms:.2} ms, {changed} changed pixels, crop MAE {mae:.4}; native/full identical\n"));
        }
        ensure!(
            edit.portrait_demand().is_empty(),
            "manual repair requested AI"
        );
    }
    print!("{report}");
    std::fs::write(out.join("results.txt"), report)?;
    Ok(())
}
