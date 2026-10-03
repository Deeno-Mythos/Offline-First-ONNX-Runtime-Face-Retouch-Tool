//! Offline paired-data preparation using the application's import and face pipeline.
use anyhow::{Context, Result, ensure};
use hastur_retouch::{engine, model, nullstate::PortraitDemand};
use std::{fs, path::PathBuf};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(
        args.next()
            .context("training_faces INPUT_DIRECTORY OUTPUT_DIRECTORY")?,
    );
    let output = PathBuf::from(args.next().context("Missing output directory")?);
    let source_root = root.canonicalize()?;
    let absolute_output = std::path::absolute(&output)?;
    let mut ancestor = absolute_output.as_path();
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .context("Output has no existing ancestor")?;
    }
    let tail = absolute_output.strip_prefix(ancestor)?;
    ensure!(
        !tail
            .components()
            .any(|part| part == std::path::Component::ParentDir),
        "Output must have a resolved parent"
    );
    let resolved_output = ancestor.canonicalize()?.join(tail);
    ensure!(
        !resolved_output.starts_with(&source_root),
        "Use an output directory outside the read-only dataset"
    );
    fs::create_dir_all(&output)?;
    let mut folders: Vec<_> = fs::read_dir(root)?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .collect();
    folders.sort_by_key(|e| e.file_name());
    for folder in folders {
        let name = folder.file_name().to_string_lossy().into_owned();
        let id = name
            .split(" - ")
            .nth(1)
            .context("Expected an anonymous numeric subject ID")?;
        ensure!(id.chars().all(|c| c.is_ascii_digit()), "Invalid subject ID");
        for (label, path) in [
            ("original", folder.path()),
            ("edited", folder.path().join("EDITED")),
        ] {
            let files: Vec<_> = fs::read_dir(&path)?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("jpg"))
                })
                .collect();
            ensure!(
                files.len() == 1,
                "Subject {id} {label}: expected exactly one JPEG, got {}",
                files.len()
            );
            let stem = output.join(format!("subject-{id}-{label}"));
            let skin_path = output.join(format!("subject-{id}-{label}-skin.png"));
            let metadata = fs::metadata(&files[0])?;
            let source_stamp = format!(
                "app-import-v1\n{}\n{}\n{:?}",
                files[0].canonicalize()?.display(),
                metadata.len(),
                metadata.modified()?
            );
            let stamp_path = stem.with_extension("source");
            if stem.with_extension("png").is_file()
                && stem.with_extension("ron").is_file()
                && (label != "original" || skin_path.is_file())
                && fs::read_to_string(&stamp_path).ok().as_deref() == Some(&source_stamp)
            {
                continue;
            }
            let photo = engine::load_photo(&files[0])?;
            let seg = model::analyze_portrait_demand_with_status(
                &photo.preview,
                model::Provider::Cpu,
                PortraitDemand {
                    geometry: true,
                    ..Default::default()
                },
                None,
                |_| {},
                |_| {},
            )?;
            ensure!(
                seg.faces.len() == 1,
                "Subject {id} {label}: expected one face, got {}",
                seg.faces.len()
            );
            fs::write(
                stem.with_extension("ron"),
                ron::ser::to_string_pretty(&seg.faces, Default::default())?,
            )?;
            photo.preview.save(stem.with_extension("png"))?;
            if label == "original" {
                let mut mask = image::GrayImage::new(seg.width, seg.height);
                let crop = seg.map_crop.unwrap_or(engine::Crop {
                    x: 0,
                    y: 0,
                    width: seg.width,
                    height: seg.height,
                });
                for (i, value) in seg.skin.iter().enumerate() {
                    mask.put_pixel(
                        crop.x + i as u32 % crop.width,
                        crop.y + i as u32 / crop.width,
                        image::Luma([(value.clamp(0.0, 1.0) * 255.0).round() as u8]),
                    );
                }
                mask.save(skin_path)?;
            }
            fs::write(stamp_path, source_stamp)?;
            println!(
                "Subject {id} {label}: {:?}, face confidence {:.3}",
                photo.original.dimensions(),
                seg.faces[0].confidence
            );
        }
    }
    Ok(())
}
