//! Validate only the changed skin model and its geometry dependencies after training.
use anyhow::{Context, Result, ensure};
use hastur_retouch::{
    engine::{self, Edit},
    model::{self, Provider},
    nullstate::PortraitDemand,
};
use std::{fs, path::PathBuf, time::Instant};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let provider = match args.next().as_deref() {
        Some("cpu") => Provider::Cpu,
        Some("burn") => Provider::Burn,
        Some("auto") => Provider::Auto,
        _ => anyhow::bail!("paired_model_validation cpu|burn|auto INPUT OUTPUT"),
    };
    let input = PathBuf::from(args.next().context("Missing input")?);
    let output = PathBuf::from(args.next().context("Missing output")?);
    fs::create_dir_all(&output)?;
    let photo = engine::load_photo(&input)?;
    let start = Instant::now();
    let seg = model::analyze_portrait_demand_with_status(
        &photo.preview,
        provider,
        PortraitDemand {
            geometry: true,
            skin: true,
            blemishes: false,
        },
        None,
        |_| {},
        |status| println!("{}: {}", status.stage, status.detail),
    )?;
    ensure!(seg.faces.len() == 1, "Expected one validation face");
    let inference_seconds = start.elapsed().as_secs_f64();
    ensure!(
        seg.prepared.skin && !seg.prepared.blemishes,
        "Unexpected model demand"
    );
    ensure!(
        seg.neural_blend.iter().flatten().all(|v| v.is_finite()),
        "Nonfinite skin output"
    );
    let mut edit = Edit::default();
    edit.settings.smoothing = 70.0;
    edit.settings.under_eyes = 60.0;
    let overview = engine::render(&photo.preview, &edit, Some(&seg));
    let native = engine::render(&photo.original, &edit, Some(&seg));
    ensure!(
        native.dimensions() == photo.original.dimensions(),
        "Changed native dimensions"
    );
    ensure!(
        native
            .pixels()
            .zip(photo.original.pixels())
            .all(|(a, b)| a[3] == b[3]),
        "Changed alpha"
    );
    ensure!(native != *photo.original, "Skin controls had no effect");
    ensure!(
        edit.settings.contrast == 0.0 && edit.settings.exposure == 0.0,
        "Unexpected global tone changes"
    );
    overview.save(output.join("retouch-overview.png"))?;
    native.save(output.join("retouch-native.png"))?;
    let report = format!(
        "{}\nInference: {inference_seconds:.3}s\nNative source/output: {:?}\nSkin 70 / under-eyes 60; contrast 0 / exposure 0; unchanged alpha\n",
        seg.status,
        native.dimensions()
    );
    fs::write(output.join("results.txt"), &report)?;
    println!("{report}");
    Ok(())
}
