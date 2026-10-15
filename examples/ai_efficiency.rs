//! Isolated-process cache timings and exact output fingerprints. No source edits.
use anyhow::{Result, ensure};
use hastur_retouch::{
    engine::{self, Edit},
    model::{self, Provider},
    nullstate::PortraitDemand,
};
use std::{path::PathBuf, time::Instant};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let output = PathBuf::from(args.get(1).expect("output directory"));
    let demand = match args.get(2).map(String::as_str) {
        Some("geometry") => PortraitDemand::GEOMETRY,
        Some("skin") => PortraitDemand {
            skin: true,
            ..PortraitDemand::GEOMETRY
        },
        _ => PortraitDemand::ALL,
    };
    let provider = if args.iter().any(|a| a == "--auto") {
        Provider::Auto
    } else {
        Provider::Cpu
    };
    let force = args.iter().any(|a| a == "--force");
    std::fs::create_dir_all(&output)?;
    let photo = engine::load_photo(&PathBuf::from("assets/demo-portrait.png"))?;
    model::take_model_run_trace();
    let start = Instant::now();
    let seg = if args.iter().any(|a| a == "--compat") {
        model::analyze_cached(&photo.preview, provider, force, |_| {})?
    } else {
        model::analyze_shared_demand_cached_with_status(
            &photo.preview,
            provider,
            force,
            demand,
            None,
            |_| {},
            |_| {},
        )?
    };
    let elapsed = start.elapsed().as_secs_f64() * 1000.;
    let trace = model::take_model_run_trace();
    ensure!(seg.prepared.contains(demand), "Incomplete cached stages");
    let mut hash = 0xcbf29ce484222325u64;
    for bytes in [
        &seg.skin[..],
        &seg.teeth[..],
        &seg.eyes[..],
        &seg.under_eyes[..],
        &seg.forehead[..],
        &seg.laugh_lines[..],
        &seg.contour[..],
        &seg.highlight[..],
        &seg.blemish[..],
        bytemuck::cast_slice(&seg.neural_blend[..]),
        bytemuck::cast_slice(&seg.repair_delta[..]),
    ] {
        for value in bytes {
            for byte in value.to_bits().to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
        }
    }
    std::fs::write(output.join("fingerprint.txt"), format!("{hash:016x}\n"))?;
    let mut edit = Edit::default();
    if demand.skin {
        edit.settings.smoothing = 65.;
        edit.settings.under_eyes = 65.;
    }
    if demand.blemishes {
        edit.settings.blemishes = 65.;
    }
    engine::render(&photo.preview, &edit, Some(&seg)).save(output.join("render.png"))?;
    let cache = std::env::var_os("HASTUR_AI_CACHE_DIR")
        .map(PathBuf::from)
        .expect("Use isolated HASTUR_AI_CACHE_DIR");
    let bytes = std::fs::read_dir(cache)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum::<u64>()
        })
        .unwrap_or(0);
    let report = format!(
        "analysis_ms={elapsed:.3}\ncache_bytes={bytes}\nresident_models={}\nmodel_runs={}\nstatus={}\nfingerprint={hash:016x}\n",
        model::resident_model_count(),
        trace.join(","),
        seg.status
    );
    print!("{report}");
    std::fs::write(output.join("results.txt"), report)?;
    Ok(())
}
