//! Isolated-process release AI timing, peak working set, and native-resolution crops.
use anyhow::{Result, ensure};
use astra_retouch::{
    engine::{self, Edit},
    model::{self, Provider},
};
use image::RgbaImage;
use std::{fs, io::Write, path::PathBuf, time::Instant};

#[cfg(windows)]
fn peak_working_set() -> usize {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    let mut counters = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        ..Default::default()
    };
    // SAFETY: a correctly sized writable struct and current-process pseudo-handle are supplied.
    unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<Counters>() as u32,
        );
    }
    counters.peak_working_set
}
#[cfg(not(windows))]
fn peak_working_set() -> usize {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<usize>().ok())
        })
        .unwrap_or(0)
        * 1024
}
fn crop(image: &RgbaImage, b: [f32; 4], margin: f32) -> RgbaImage {
    let w = image.width() as f32;
    let h = image.height() as f32;
    let x = ((b[0] - b[2] * margin) * w).floor().clamp(0.0, w - 1.0) as u32;
    let y = ((b[1] - b[3] * margin) * h).floor().clamp(0.0, h - 1.0) as u32;
    let right = ((b[0] + b[2] * (1.0 + margin)) * w)
        .ceil()
        .clamp((x + 1) as f32, w) as u32;
    let bottom = ((b[1] + b[3] * (1.0 + margin)) * h)
        .ceil()
        .clamp((y + 1) as f32, h) as u32;
    image::imageops::crop_imm(image, x, y, right - x, bottom - y).to_image()
}
fn difference(a: &RgbaImage, b: &RgbaImage) -> (usize, f64, u8) {
    let changed = a.pixels().zip(b.pixels()).filter(|(a, b)| a != b).count();
    let (total, max) =
        a.as_raw()
            .iter()
            .zip(b.as_raw())
            .fold((0_u64, 0_u8), |(total, max), (a, b)| {
                let delta = a.abs_diff(*b);
                (total + delta as u64, max.max(delta))
            });
    (changed, total as f64 / a.as_raw().len() as f64, max)
}
fn registered_mask(seg: &engine::Segmentation, data: &[f32], u: f32, v: f32) -> f32 {
    if seg.width == 0 || seg.height == 0 {
        return 0.0;
    }
    let crop = if data.len() == (seg.width * seg.height) as usize {
        engine::Crop {
            x: 0,
            y: 0,
            width: seg.width,
            height: seg.height,
        }
    } else if let Some(crop) = seg.map_crop {
        crop
    } else {
        return 0.0;
    };
    let x = (u * seg.width as f32 - 0.5).clamp(0.0, (seg.width - 1) as f32);
    let y = (v * seg.height as f32 - 0.5).clamp(0.0, (seg.height - 1) as f32);
    let (ix, iy) = (x as u32, y as u32);
    let (tx, ty) = (x - ix as f32, y - iy as f32);
    let at = |px: u32, py: u32| {
        let (px, py) = (px.min(seg.width - 1), py.min(seg.height - 1));
        if px >= crop.x && py >= crop.y && px < crop.x + crop.width && py < crop.y + crop.height {
            data.get(((py - crop.y) * crop.width + px - crop.x) as usize)
                .copied()
                .unwrap_or(0.0)
        } else {
            0.0
        }
    };
    (at(ix, iy) * (1.0 - tx) + at(ix + 1, iy) * tx) * (1.0 - ty)
        + (at(ix, iy + 1) * (1.0 - tx) + at(ix + 1, iy + 1) * tx) * ty
}
fn detail_crops(
    image: &RgbaImage,
    face: &astra_retouch::geometry::FaceMesh,
    output: &std::path::Path,
    name: &str,
) -> Result<()> {
    let [x, y, w, h] = face.bounds;
    for (label, index) in [("left-eye", 133), ("right-eye", 362)] {
        if let Some(p) = face.landmarks.get(index) {
            let dw = (w * 0.4).min(800.0 / image.width() as f32);
            let dh = (h * 0.34).min(480.0 / image.height() as f32);
            crop(image, [p[0] - dw * 0.5, p[1] - dh * 0.22, dw, dh], 0.0)
                .save(output.join(format!("{label}-{name}-native.png")))?;
        }
    }
    let dw = (w * 0.34).min(640.0 / image.width() as f32);
    let dh = (h * 0.66).min(900.0 / image.height() as f32);
    crop(image, [x + w - dw * 0.4, y + h * 0.14, dw, dh], 0.0)
        .save(output.join(format!("hair-edge-{name}-native.png")))?;
    if matches!(name, "original" | "flyaway-100") {
        // This is the entire detector halo, not just the inside of one hair edge.
        // Keep source pixels unresized so exterior repairs and internal reflections can be reviewed.
        crop(image, [x - w * 0.65, y - h * 0.85, w * 2.3, h * 1.7], 0.0)
            .save(output.join(format!("head-halo-{name}-native.png")))?;
    }
    Ok(())
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let provider = match args.next().as_deref() {
        Some("burn") => Provider::Burn,
        Some("cpu") => Provider::Cpu,
        _ => {
            anyhow::bail!("usage: runtime_validation cpu|burn INPUT OUTPUT [REPETITIONS] [native]")
        }
    };
    let input = PathBuf::from(args.next().expect("input"));
    let output = PathBuf::from(args.next().expect("output"));
    let repeats = args
        .next()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(2);
    let native = args.next().as_deref() == Some("native");
    fs::create_dir_all(&output)?;
    let photo = engine::load_photo(&input)?;
    let baseline_peak = peak_working_set();
    let mut report = format!(
        "Input: {}\nBackend: {}\nBuild: {}\nNative source: {:?}\nInference overview: {:?}\nBaseline peak working set: {:.2} MiB\n",
        input.display(),
        provider.label(),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        photo.original.dimensions(),
        photo.preview.dimensions(),
        baseline_peak as f64 / 1048576.0
    );
    let mut latest = None;
    for repetition in 0..repeats {
        let start = Instant::now();
        let seg = model::analyze_portrait(&photo.preview, provider, |s| {
            println!(
                "{} {:.2}s {}",
                provider.label(),
                start.elapsed().as_secs_f64(),
                s.status
            );
        })?;
        ensure!(!seg.faces.is_empty(), "No face detected");
        let elapsed = start.elapsed().as_secs_f64();
        println!(
            "run {}: {:.3}s peak {:.2} MiB",
            repetition + 1,
            elapsed,
            peak_working_set() as f64 / 1048576.0
        );
        report.push_str(&format!(
            "Run {} ({} sessions, uncached AI): {:.3}s; peak working set {:.2} MiB; {} faces\n",
            repetition + 1,
            if repetition == 0 { "cold" } else { "warm" },
            elapsed,
            peak_working_set() as f64 / 1048576.0,
            seg.faces.len()
        ));
        latest = Some(seg);
    }
    let seg = latest.expect("at least one run");
    fs::write(output.join("segmentation.ron"), ron::ser::to_string(&seg)?)?;
    fs::write(
        output.join("landmarks.ron"),
        ron::ser::to_string_pretty(&seg.faces, Default::default())?,
    )?;
    // Raw maps permit independent comparisons, including actual neural model output.
    let mut maps = fs::File::create(output.join("maps.bin"))?;
    for channel in [
        &seg.skin[..],
        &seg.under_eyes[..],
        &seg.eyes[..],
        &seg.blemish[..],
    ] {
        maps.write_all(&(channel.len() as u64).to_le_bytes())?;
        for &value in channel {
            maps.write_all(&value.to_le_bytes())?;
        }
    }
    for channel in [&seg.neural_blend[..], &seg.repair_delta[..]] {
        maps.write_all(&(channel.len() as u64 * 3).to_le_bytes())?;
        for p in channel {
            for &value in p {
                maps.write_all(&value.to_le_bytes())?;
            }
        }
    }
    let mut automatic = Edit::default();
    automatic.settings.apply_auto_retouch();
    let overview = engine::render(&photo.preview, &automatic, Some(&seg));
    overview.save(output.join("auto-overview.png"))?;
    report.push_str(&format!(
        "Auto-retouch contrast/exposure: {:.1}/{:.1}\n",
        automatic.settings.contrast, automatic.settings.exposure
    ));
    if native {
        for (i, face) in seg.faces.iter().enumerate() {
            crop(&photo.original, face.bounds, 0.20)
                .save(output.join(format!("face-{i}-original-native.png")))?;
            if i == 0 {
                detail_crops(&photo.original, face, &output, "original")?;
            }
        }
        let mut cases = vec![("auto", automatic)];
        for strength in [50.0, 100.0] {
            let mut edit = Edit::default();
            edit.settings.under_eyes = strength;
            cases.push((
                if strength == 50.0 {
                    "under-eyes-50"
                } else {
                    "under-eyes-100"
                },
                edit,
            ));
        }
        let mut smooth = Edit::default();
        smooth.settings.smoothing = 100.0;
        cases.push(("skin-100", smooth));
        let mut hair = Edit::default();
        hair.settings.flyaway_hairs = 100.0;
        cases.push(("flyaway-100", hair));
        let mut previous_eye_mae = 0.0;
        for (name, edit) in cases {
            let start = Instant::now();
            let rendered = engine::render(&photo.original, &edit, Some(&seg));
            let (changed, mae, max) = difference(&rendered, &photo.original);
            ensure!(
                rendered.dimensions() == photo.original.dimensions(),
                "Native render changed dimensions"
            );
            report.push_str(&format!("Native {name}: {:.3}s; changed {changed} pixels; image MAE {mae:.6}; max channel difference {max}\n",start.elapsed().as_secs_f64()));
            if name.starts_with("under-eyes") {
                ensure!(mae > previous_eye_mae, "Under-eye strength not monotonic");
                previous_eye_mae = mae;
                let (w, h) = photo.original.dimensions();
                let outside_mask = rendered
                    .pixels()
                    .zip(photo.original.pixels())
                    .enumerate()
                    .filter(|(i, (a, b))| {
                        a != b
                            && registered_mask(
                                &seg,
                                &seg.under_eyes,
                                ((*i as u32 % w) as f32 + 0.5) / w as f32,
                                ((*i as u32 / w) as f32 + 0.5) / h as f32,
                            ) < 0.49 / 255.0
                    })
                    .count();
                report.push_str(&format!("Native {name}: {outside_mask} changed pixels outside quantized under-eye mask\n"));
                ensure!(
                    outside_mask == 0,
                    "Under-eye retouch changed unrelated pixels outside its mask ({outside_mask})"
                );
                let mut target_pixels = 0_usize;
                let mut target_error = 0_u64;
                let mut target_luma = 0.0_f64;
                for (i, (after, before)) in
                    rendered.pixels().zip(photo.original.pixels()).enumerate()
                {
                    if registered_mask(
                        &seg,
                        &seg.under_eyes,
                        ((i as u32 % w) as f32 + 0.5) / w as f32,
                        ((i as u32 / w) as f32 + 0.5) / h as f32,
                    ) >= 0.2
                    {
                        target_pixels += 1;
                        for c in 0..3 {
                            target_error += after[c].abs_diff(before[c]) as u64;
                        }
                        target_luma += (after[0] as f64 - before[0] as f64) * 0.2126
                            + (after[1] as f64 - before[1] as f64) * 0.7152
                            + (after[2] as f64 - before[2] as f64) * 0.0722;
                    }
                }
                report.push_str(&format!("Native {name}: {target_pixels} confident under-eye pixels; target RGB MAE {:.6} bytes; target mean brightness change {:+.6} bytes\n",target_error as f64/(target_pixels.max(1)*3) as f64,target_luma/target_pixels.max(1) as f64));
            }
            if name != "flyaway-100" {
                let (w, h) = photo.original.dimensions();
                let exterior_changes = rendered
                    .pixels()
                    .zip(photo.original.pixels())
                    .enumerate()
                    .filter(|(i, (a, b))| {
                        if a == b {
                            return false;
                        }
                        let u = (*i as u32 % w) as f32 / w as f32;
                        let v = (*i as u32 / w) as f32 / h as f32;
                        !seg.faces.iter().any(|face| {
                            let [x, y, fw, fh] = face.bounds;
                            u >= x - fw * 0.3
                                && u <= x + fw * 1.3
                                && v >= y - fh * 0.3
                                && v <= y + fh * 1.3
                        })
                    })
                    .count();
                report.push_str(&format!("Native {name}: {exterior_changes} changed pixels outside expanded face bounds\n"));
                ensure!(
                    exterior_changes == 0,
                    "Facial retouch changed unrelated exterior pixels ({exterior_changes})"
                );
            }
            for (i, face) in seg.faces.iter().enumerate() {
                crop(&rendered, face.bounds, 0.20)
                    .save(output.join(format!("face-{i}-{name}-native.png")))?;
                if i == 0 {
                    detail_crops(&rendered, face, &output, name)?;
                }
            }
        }
    }
    report.push_str(&format!(
        "Final peak working set: {:.2} MiB\n",
        peak_working_set() as f64 / 1048576.0
    ));
    print!("{report}");
    fs::write(output.join("results.txt"), report)?;
    Ok(())
}
