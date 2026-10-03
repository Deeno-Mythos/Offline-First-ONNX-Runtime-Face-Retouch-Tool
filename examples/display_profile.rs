//! CPU stages after a native render, plus scalar/AVX2 upload planning candidates.
use anyhow::{Result, ensure};
use hastur_retouch::{engine, gpu::diagnostics};
use std::{hint::black_box, path::Path, time::Instant};
fn measure(mut f: impl FnMut(), repeats: usize) -> (f64, f64) {
    f();
    let mut samples = Vec::new();
    for _ in 0..repeats {
        let start = Instant::now();
        f();
        samples.push(start.elapsed().as_secs_f64() * 1000.);
    }
    samples.sort_by(f64::total_cmp);
    (samples[repeats / 2], samples[repeats * 95 / 100])
}
fn main() -> Result<()> {
    let root = Path::new("output/render-profile");
    std::fs::create_dir_all(root)?;
    let photo = engine::load_photo(Path::new(
        "example-img/CTU DUMANJUG ORG 09.28.26_JAMESBRO-522.JPG",
    ))?;
    let mut csv = String::from("viewport,scenario,stage,median_ms,p95_ms,changed_percent\n");
    println!("AVX2 available: {}", diagnostics::simd_available());
    for (w, h) in [(1536, 1024), (2048, 1536)] {
        let a = image::imageops::crop_imm(&*photo.original, 1400, 2200, w, h).to_image();
        let mut local = a.clone();
        for y in h / 2..h / 2 + 91 {
            for x in w / 2..w / 2 + 73 {
                local.get_pixel_mut(x, y)[1] ^= 31;
            }
        }
        let all = image::RgbaImage::from_fn(w, h, |x, y| {
            let mut p = *a.get_pixel(x, y);
            p[0] ^= 1;
            p
        });
        for (scenario, b) in [("local", &local), ("unchanged", &a), ("global", &all)] {
            let region = diagnostics::change_bounds(&a, b, false);
            ensure!(region == diagnostics::change_bounds(&a, b, true));
            let percent = region.map_or(0., |r| {
                r.width as f64 * r.height as f64 / (w as f64 * h as f64) * 100.
            });
            for (name, scan) in [("scalar_scan", false), ("simd_scan", true)] {
                let (m, p) = measure(
                    || {
                        black_box(diagnostics::change_bounds(
                            black_box(&a),
                            black_box(b),
                            scan,
                        ));
                    },
                    80,
                );
                println!(
                    "{w}x{h} {scenario} {name}: {m:.3} ms (p95 {p:.3}), changed {percent:.3}%"
                );
                csv.push_str(&format!(
                    "{w}x{h},{scenario},{name},{m:.6},{p:.6},{percent:.3}\n"
                ));
            }
        }
        let (m, p) = measure(
            || {
                black_box(image::imageops::thumbnail(black_box(&local), 320, 320));
            },
            32,
        );
        println!("{w}x{h} fallback_thumbnail: {m:.3} ms (p95 {p:.3})");
        csv.push_str(&format!(
            "{w}x{h},local,fallback_thumbnail,{m:.6},{p:.6},0\n"
        ));
        let thumb = image::imageops::thumbnail(&local, 320, 320);
        let (m, p) = measure(
            || {
                black_box(eframe::egui::ColorImage::from_rgba_unmultiplied(
                    [thumb.width() as usize, thumb.height() as usize],
                    black_box(thumb.as_raw()),
                ));
            },
            80,
        );
        csv.push_str(&format!(
            "{w}x{h},local,fallback_color_pack,{m:.6},{p:.6},0\n"
        ));
        let (m, p) = measure(
            || {
                black_box(local.clone());
            },
            80,
        );
        csv.push_str(&format!("{w}x{h},local,frame_copy,{m:.6},{p:.6},0\n"));
    }
    let after = std::env::args().any(|a| a == "--after");
    std::fs::write(
        root.join(if after {
            "display-after.csv"
        } else {
            "display-baseline.csv"
        }),
        csv,
    )?;
    Ok(())
}
