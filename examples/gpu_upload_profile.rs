//! Synchronized GPU upload/conversion timing with actual texture readback verification.
use anyhow::{Result, ensure};
use hastur_retouch::{
    engine,
    gpu::diagnostics::{self, UploadProfiler},
};
use std::{path::Path, sync::Arc};
fn stats(mut values: Vec<f64>) -> (f64, f64) {
    values.sort_by(f64::total_cmp);
    (values[values.len() / 2], values[values.len() * 95 / 100])
}
fn main() -> Result<()> {
    let root = Path::new("output/render-profile");
    std::fs::create_dir_all(root)?;
    let validation = diagnostics::validate_upload()?;
    println!("{validation}");
    std::fs::write(root.join("gpu-validation.txt"), validation)?;
    let photo = engine::load_photo(Path::new(
        "example-img/CTU DUMANJUG ORG 09.28.26_JAMESBRO-522.JPG",
    ))?;
    let mut csv = String::from(
        "viewport,scenario,full_encode_ms,live_encode_ms,full_complete_ms,live_complete_ms,full_p95_ms,live_p95_ms,uploaded_percent\n",
    );
    for (w, h) in [(1536, 1024), (2048, 1536)] {
        for scenario in ["local", "unchanged", "global"] {
            let mut full = UploadProfiler::new(false)?;
            let mut live = UploadProfiler::new(true)?;
            let mut image =
                Arc::new(image::imageops::crop_imm(&*photo.original, 1400, 2200, w, h).to_image());
            full.update(&image)?;
            live.update(&image)?;
            let (mut fe, mut le, mut fc, mut lc) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut uploaded = 0;
            for i in 0..40u32 {
                let mut next = image.as_ref().clone();
                match scenario {
                    "local" => {
                        for y in h / 2..h / 2 + 91 {
                            for x in w / 2 + i * 3..w / 2 + i * 3 + 73 {
                                next.get_pixel_mut(x, y)[1] ^= 31;
                            }
                        }
                    }
                    "global" => {
                        for p in next.pixels_mut() {
                            p[0] ^= 1;
                        }
                    }
                    _ => {}
                }
                image = Arc::new(next);
                // Alternate measurement order to reduce thermal/order bias.
                let (f, l) = if i % 2 == 0 {
                    (full.update(&image)?, live.update(&image)?)
                } else {
                    let l = live.update(&image)?;
                    (full.update(&image)?, l)
                };
                fe.push(f.encode_ms);
                le.push(l.encode_ms);
                fc.push(f.complete_ms);
                lc.push(l.complete_ms);
                uploaded += l.pixels;
                if i % 10 == 9 {
                    ensure!(
                        full.read_linear()? == live.read_linear()?,
                        "{w}x{h}/{scenario}: GPU pixels changed"
                    );
                }
            }
            let (fe, _) = stats(fe);
            let (le, _) = stats(le);
            let (fc, fp) = stats(fc);
            let (lc, lp) = stats(lc);
            let percent = uploaded as f64 / (40. * w as f64 * h as f64) * 100.;
            println!(
                "{w}x{h} {scenario}: CPU encode {fe:.3}->{le:.3} ms; completed GPU update {fc:.3}->{lc:.3} ms (p95 {fp:.3}->{lp:.3}); uploaded {percent:.3}%; {}",
                live.adapter
            );
            csv.push_str(&format!(
                "{w}x{h},{scenario},{fe:.6},{le:.6},{fc:.6},{lc:.6},{fp:.6},{lp:.6},{percent:.3}\n"
            ));
        }
    }
    std::fs::write(root.join("gpu-upload.csv"), csv)?;
    Ok(())
}
