//! Whole viewport latency, warmed growing strokes, with full-frame pixel parity.
use anyhow::{Result, ensure};
use hastur_retouch::{
    cleanup::{CloneStamp, PatchStroke},
    engine::{self, Crop, Edit, Renderer, Stroke, Target},
    geometry::WarpStroke,
    gpu::diagnostics::UploadProfiler,
};
use std::{path::Path, sync::Arc, time::Instant};

fn append(edit: &mut Edit, tool: &str, index: usize) {
    let center = [
        0.42 + (index % 16) as f32 * 0.006,
        0.40 + (index / 16) as f32 * 0.008,
    ];
    match tool {
        "liquify" => edit.warps.push(WarpStroke {
            center,
            delta: [0.0008, -0.0004],
            radius: 0.012,
            softness: 0.7,
            strength: 85.,
        }),
        "heal" => edit.strokes.push(Stroke {
            target: Target::Heal,
            center,
            radius: 0.008,
            softness: 0.7,
            erase: false,
            strength: 100.,
        }),
        "clone" => edit.clones.push(CloneStamp {
            center,
            source: [center[0] + 0.04, center[1] - 0.04],
            radius: 0.012,
            softness: 0.7,
            strength: 90.,
        }),
        "patch" => {
            let patch = PatchStroke {
                boundary: vec![[0.49, 0.41], [0.51, 0.41], [0.51, 0.44], [0.49, 0.44]],
                offset: [-0.045 + index as f32 * 0.001, 0.035],
                softness: 0.7,
                strength: 90.,
            };
            if edit.patches.is_empty() {
                edit.patches.push(patch);
            } else {
                *edit.patches.last_mut().unwrap() = patch;
            }
        }
        _ => unreachable!(),
    }
}
fn stats(mut timings: Vec<f64>) -> (f64, f64) {
    timings.sort_by(f64::total_cmp);
    (
        timings[timings.len() / 2],
        timings[timings.len() * 95 / 100],
    )
}
fn main() -> Result<()> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build()?;
    pool.install(run)
}
fn run() -> Result<()> {
    let gpu_profile = std::env::args().any(|arg| arg == "--gpu-profile");
    let root = Path::new(if gpu_profile {
        "output/render-profile/manual"
    } else {
        "output/live-viewport"
    });
    std::fs::create_dir_all(root)?;
    let photo = engine::load_photo(Path::new(
        "example-img/CTU DUMANJUG ORG 09.28.26_JAMESBRO-522.JPG",
    ))?;
    let source = &photo.original;
    let mut report = String::from(
        "viewport,tool,full_median_ms,full_p95_ms,incremental_median_ms,incremental_p95_ms,redrawn_percent,max_rgb_difference\n",
    );
    let mut gpu_report = String::from(
        "viewport,tool,full_encode_ms,live_encode_ms,full_complete_ms,live_complete_ms,full_p95_ms,live_p95_ms,uploaded_percent,max_gpu_difference\n",
    );
    for (width, height) in [(1536, 1024), (2048, 1536)] {
        let crop = Crop {
            x: source.width() * 38 / 100,
            y: source.height() * 38 / 100,
            width,
            height,
        };
        for tool in ["liquify", "heal", "clone", "patch"] {
            let mut edit = Edit::default();
            for i in 0..48 {
                append(&mut edit, tool, i);
            }
            let mut full = Renderer::default();
            let mut live = Renderer::default();
            ensure!(
                full.render_region(source, &edit, None, crop, None)?
                    == live.render_viewport(source, &edit, None, crop, None)?
            );
            let mut gpu = if gpu_profile {
                let mut full_gpu = UploadProfiler::new(false)?;
                let mut live_gpu = UploadProfiler::new(true)?;
                let initial = Arc::new(live.render_viewport(source, &edit, None, crop, None)?);
                full_gpu.update(&initial)?;
                live_gpu.update(&initial)?;
                Some((full_gpu, live_gpu))
            } else {
                None
            };
            let (mut fe, mut le, mut fc, mut lc) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut uploaded = 0;
            let mut before = Vec::new();
            let mut after = Vec::new();
            let mut total_area = 0u64;
            let mut max_diff = 0;
            for i in 48..64 {
                append(&mut edit, tool, i);
                let start = Instant::now();
                let expected = full.render_region(source, &edit, None, crop, None)?;
                before.push(start.elapsed().as_secs_f64() * 1000.);
                let start = Instant::now();
                let actual = live.render_viewport(source, &edit, None, crop, None)?;
                after.push(start.elapsed().as_secs_f64() * 1000.);
                if let Some(dirty) = live.viewport_damage() {
                    total_area += dirty.width as u64 * dirty.height as u64;
                }
                let difference = actual
                    .as_raw()
                    .iter()
                    .zip(expected.as_raw())
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                max_diff = max_diff.max(difference);
                ensure!(
                    difference <= 1,
                    "{width}x{height}-{tool}: full/native difference {difference}"
                );
                if let Some((full_gpu, live_gpu)) = &mut gpu {
                    let image = Arc::new(actual.clone());
                    let (f, l) = if i % 2 == 0 {
                        (full_gpu.update(&image)?, live_gpu.update(&image)?)
                    } else {
                        let l = live_gpu.update(&image)?;
                        (full_gpu.update(&image)?, l)
                    };
                    fe.push(f.encode_ms);
                    le.push(l.encode_ms);
                    fc.push(f.complete_ms);
                    lc.push(l.complete_ms);
                    uploaded += l.pixels;
                    if i % 4 == 3 {
                        ensure!(
                            full_gpu.read_linear()? == live_gpu.read_linear()?,
                            "{width}x{height}-{tool}: GPU partial upload changed pixels"
                        );
                    }
                }
                if i == 63 {
                    actual.save(root.join(format!("{width}x{height}-{tool}.png")))?;
                }
            }
            let (b, b95) = stats(before);
            let (a, a95) = stats(after);
            let percent = total_area as f64 / 16. / (width as f64 * height as f64) * 100.;
            println!(
                "{width}x{height} {tool}: full {b:.2} ms (p95 {b95:.2}), live {a:.2} ms (p95 {a95:.2}), redrawn {percent:.2}%, max pixel difference {max_diff}"
            );
            report.push_str(&format!(
                "{width}x{height},{tool},{b:.3},{b95:.3},{a:.3},{a95:.3},{percent:.3},{max_diff}\n"
            ));
            if let Some((_, live_gpu)) = gpu {
                let (fe, _) = stats(fe);
                let (le, _) = stats(le);
                let (fc, fp) = stats(fc);
                let (lc, lp) = stats(lc);
                let percent = uploaded as f64 / (16. * width as f64 * height as f64) * 100.;
                println!(
                    "  GPU encode {fe:.3}->{le:.3} ms; completed update {fc:.3}->{lc:.3} ms (p95 {fp:.3}->{lp:.3}); uploaded {percent:.3}%; {}",
                    live_gpu.adapter
                );
                gpu_report.push_str(&format!(
                    "{width}x{height},{tool},{fe:.6},{le:.6},{fc:.6},{lc:.6},{fp:.6},{lp:.6},{percent:.3},0\n"
                ));
            }
        }
    }
    std::fs::write(root.join("timings.csv"), report)?;
    if gpu_profile {
        std::fs::write(root.join("gpu-upload.csv"), gpu_report)?;
    }
    Ok(())
}
