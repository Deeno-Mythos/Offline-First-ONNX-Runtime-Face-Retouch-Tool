//! Uncached provider timing and source-resolution output for hardware qualification.
use anyhow::{Result, ensure};
use hastur_retouch::{
    engine::{self, Edit},
    model::{self, Provider},
};
use std::{fs, io::Write, path::PathBuf, time::Instant};

#[cfg(windows)]
fn peak_mib() -> f64 {
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak: usize,
        working: usize,
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
            cb: u32,
        ) -> i32;
    }
    let mut c = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        faults: 0,
        peak: 0,
        working: 0,
        peak_paged: 0,
        paged: 0,
        peak_nonpaged: 0,
        nonpaged: 0,
        pagefile: 0,
        peak_pagefile: 0,
    };
    // SAFETY: the initialized C structure and byte length match PROCESS_MEMORY_COUNTERS.
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) } != 0 {
        c.peak as f64 / 1048576.0
    } else {
        0.0
    }
}
#[cfg(not(windows))]
fn peak_mib() -> f64 {
    0.0
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let provider = match args.next().as_deref() {
        Some("auto") => Provider::Auto,
        Some("cpu") => Provider::Cpu,
        Some("dml") => Provider::DirectMl,
        _ => anyhow::bail!("hardware_validation cpu|auto|dml INPUT OUTPUT [REPETITIONS]"),
    };
    let input = PathBuf::from(args.next().expect("INPUT"));
    let output = PathBuf::from(args.next().expect("OUTPUT"));
    let repeats = args
        .next()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(2)
        .max(1);
    fs::create_dir_all(&output)?;
    let photo = engine::load_photo(&input)?;
    let mut report = format!(
        "Provider request: {}\nInput: {}\nOriginal: {:?}; inference: {:?}\nBuild: {}\n",
        provider.label(),
        input.display(),
        photo.original.dimensions(),
        photo.preview.dimensions(),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    let mut last = None;
    for i in 0..repeats {
        let start = Instant::now();
        let seg = model::analyze_portrait_with_status(
            &photo.preview,
            provider,
            |_| {},
            |status| {
                println!(
                    "{:.3}s {}: {}",
                    start.elapsed().as_secs_f64(),
                    status.stage,
                    status.detail
                )
            },
        )?;
        ensure!(!seg.faces.is_empty(), "No confident face detected");
        let line = format!(
            "Run {}: {:.3}s; peak {:.2} MiB; {}\n",
            i + 1,
            start.elapsed().as_secs_f64(),
            peak_mib(),
            seg.status
        );
        print!("{line}");
        report.push_str(&line);
        last = Some(seg);
    }
    let seg = last.unwrap();
    fs::write(output.join("segmentation.ron"), ron::ser::to_string(&seg)?)?;
    let mut maps =
        std::io::BufWriter::with_capacity(1024 * 1024, fs::File::create(output.join("maps.bin"))?);
    for channel in [
        &seg.skin[..],
        &seg.under_eyes[..],
        &seg.eyes[..],
        &seg.blemish[..],
    ] {
        maps.write_all(&(channel.len() as u64).to_le_bytes())?;
        for &v in channel {
            maps.write_all(&v.to_le_bytes())?;
        }
    }
    for channel in [&seg.neural_blend[..], &seg.repair_delta[..]] {
        maps.write_all(&(channel.len() as u64 * 3).to_le_bytes())?;
        for p in channel {
            for &v in p {
                maps.write_all(&v.to_le_bytes())?;
            }
        }
    }
    maps.flush()?;
    let mut edit = Edit::default();
    edit.settings.apply_auto_retouch();
    engine::render(&photo.preview, &edit, Some(&seg)).save(output.join("auto-overview.png"))?;
    let native = engine::render(&photo.original, &edit, Some(&seg));
    ensure!(
        native.dimensions() == photo.original.dimensions(),
        "Native output changed dimensions"
    );
    native.save(output.join("auto-native.png"))?;
    report.push_str(&format!(
        "Native auto output: {:?}; contrast {:.1}; exposure {:.1}\n",
        native.dimensions(),
        edit.settings.contrast,
        edit.settings.exposure
    ));
    fs::write(output.join("results.txt"), report)?;
    Ok(())
}
