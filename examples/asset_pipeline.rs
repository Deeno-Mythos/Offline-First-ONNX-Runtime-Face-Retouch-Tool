//! Read-only staging/residency audit on a real portrait batch.
use anyhow::{Context, Result, ensure};
use hastur_retouch::assets;
use std::{path::PathBuf, sync::Arc, time::Instant};
fn main() -> Result<()> {
    let root = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("Pass a portrait folder")?,
    );
    let mut paths = assets::folder_files(&root)?;
    for entry in std::fs::read_dir(&root)? {
        let path = entry?.path();
        if path.is_dir() {
            paths.extend(assets::folder_files(&path)?);
        }
    }
    paths.sort();
    paths.truncate(24);
    ensure!(!paths.is_empty(), "No supported portraits found");
    let start = Instant::now();
    let photos: Vec<_> = paths
        .iter()
        .map(|p| assets::stage(p))
        .collect::<Result<_>>()?;
    let cold = start.elapsed();
    let start = Instant::now();
    for path in &paths {
        let _ = assets::stage(path)?;
    }
    let warm = start.elapsed();
    ensure!(
        photos
            .iter()
            .all(|p| !p.resident && p.preview.width().max(p.preview.height()) <= 512),
        "Staging retained original pixels"
    );
    let staged: usize = photos.iter().map(|p| p.original.as_raw().len()).sum();
    let full: u64 = photos
        .iter()
        .map(|p| u64::from(p.dimensions().0) * u64::from(p.dimensions().1) * 4)
        .sum();
    let start = Instant::now();
    let mut active = photos[0].open_native()?;
    let load = start.elapsed();
    ensure!(
        active.original.dimensions() == photos[0].dimensions(),
        "Native dimensions changed"
    );
    let weak = Arc::downgrade(&active.original);
    active.release_pixels();
    ensure!(
        weak.upgrade().is_none(),
        "Native source still retained after eviction"
    );
    let out = PathBuf::from("output/asset-pipeline");
    std::fs::create_dir_all(&out)?;
    let mut sheet = image::RgbaImage::new(6 * 192, photos.len().div_ceil(6) as u32 * 256);
    for (i, p) in photos.iter().enumerate() {
        let thumb = image::imageops::thumbnail(p.thumbnail.as_ref(), 192, 256);
        image::imageops::overlay(
            &mut sheet,
            &thumb,
            (i % 6 * 192) as i64,
            (i / 6 * 256) as i64,
        );
    }
    sheet.save(out.join("thumbnails.png"))?;
    let report = format!(
        "Photos: {}\nStaging pass: {:?}\nCached staging: {:?}\nActive native load: {:?}\nRetained staged RGBA bytes: {}\nEquivalent all-native RGBA bytes: {}\nReduction: {:.2}%\nOriginal dimensions: {:?}\nNative Arc released: true\nMeasurement: pixel allocations, not total process RSS\n",
        photos.len(),
        cold,
        warm,
        load,
        staged,
        full,
        100. * (1. - staged as f64 / full as f64),
        photos[0].dimensions()
    );
    std::fs::write(out.join("report.txt"), &report)?;
    print!("{report}");
    Ok(())
}
