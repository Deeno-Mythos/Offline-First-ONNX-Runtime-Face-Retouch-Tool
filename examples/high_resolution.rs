//! Render-only large-image and cached native-viewport measurements. No inference runs here.
use anyhow::{Result, ensure};
use astra_retouch::engine::{
    self, Crop, Edit, ExportOptions, ExportSize, Renderer, Segmentation, Stroke, Target,
};
use std::{fs, path::PathBuf, sync::Arc, time::Instant};

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
    // SAFETY: this struct has the Windows PROCESS_MEMORY_COUNTERS layout and size.
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
fn segmentation(path: &std::path::Path) -> Result<Segmentation> {
    let mut seg: Segmentation =
        ron::de::from_str(&fs::read_to_string(path.join("segmentation.ron"))?)?;
    let bytes = fs::read(path.join("maps.bin"))?;
    let mut position = 0;
    for channel in 0..6 {
        let count = u64::from_le_bytes(bytes[position..position + 8].try_into()?) as usize;
        position += 8;
        let data = &bytes[position..position + count * 4];
        position += count * 4;
        if channel >= 4 {
            ensure!(count.is_multiple_of(3), "Malformed RGB analysis map");
            let values = data
                .as_chunks::<12>()
                .0
                .iter()
                .map(|p| {
                    std::array::from_fn(|c| {
                        f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap())
                    })
                })
                .collect::<Vec<[f32; 3]>>();
            if channel == 4 {
                seg.neural_blend = values.into();
            } else {
                seg.repair_delta = values.into();
            }
        }
    }
    ensure!(position == bytes.len(), "Unexpected analysis trailing data");
    Ok(seg)
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(args.next().expect("INPUT"));
    let analysis = PathBuf::from(args.next().expect("ANALYSIS_DIR"));
    let output = PathBuf::from(args.next().expect("OUTPUT_DIR"));
    let target_megapixels = args
        .next()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    fs::create_dir_all(&output)?;
    let source = engine::load_photo(&input)?;
    let source_dimensions = source.original.dimensions();
    let photo = if target_megapixels > 0.0 {
        let factor = (target_megapixels * 1e6
            / (source_dimensions.0 as f64 * source_dimensions.1 as f64))
            .sqrt();
        let dimensions = (
            (source_dimensions.0 as f64 * factor).round() as u32,
            (source_dimensions.1 as f64 * factor).round() as u32,
        );
        let upscaled = image::imageops::resize(
            &*source.original,
            dimensions.0,
            dimensions.1,
            image::imageops::FilterType::Lanczos3,
        );
        drop(source);
        engine::photo_from_image(
            format!("synthetic-{}MP.png", target_megapixels as u32),
            None,
            upscaled,
        )
    } else {
        source
    };
    let seg = Arc::new(segmentation(&analysis)?);
    ensure!(
        !seg.neural_blend.is_empty() && !seg.repair_delta.is_empty(),
        "Complete precomputed AI maps required; no inference is run by this benchmark"
    );
    let (w, h) = photo.original.dimensions();
    let mut edit = Edit::default();
    edit.settings.apply_auto_retouch();
    edit.settings.contrast = 8.0;
    edit.settings.exposure = 0.1;
    edit.settings.sharpening = 15.0;
    edit.strokes.push(Stroke {
        target: Target::Heal,
        center: [0.43, 0.33],
        radius: 0.004,
        erase: false,
        softness: 0.7,
    });
    edit.clones.push(astra_retouch::cleanup::CloneStamp {
        center: [0.46, 0.44],
        source: [0.48, 0.43],
        radius: 0.005,
        softness: 0.7,
        strength: 80.0,
    });
    edit.patches.push(astra_retouch::cleanup::PatchStroke {
        boundary: vec![[0.39, 0.36], [0.40, 0.36], [0.40, 0.38], [0.39, 0.38]],
        offset: [0.025, 0.0],
        softness: 0.6,
        strength: 70.0,
    });
    let baseline_peak = peak_working_set();
    let mut report = format!(
        "Input: {}\nActual source: {:?}\nBenchmark dimensions: {:?} ({:.3}MP)\n{}\nPrecomputed AI: {} (no runtime inference)\nBaseline peak working set: {:.2} MiB\n",
        input.display(),
        source_dimensions,
        (w, h),
        w as f64 * h as f64 / 1e6,
        if target_megapixels > 0.0 {
            "Deterministic Lanczos3 upscale exercises memory/throughput; it does not add photographic detail."
        } else {
            "Original photograph, original resolution."
        },
        analysis.display(),
        baseline_peak as f64 / 1048576.0
    );
    let mut renderer = Renderer::default();
    let face = seg.faces.first().expect("face").bounds;
    let cx = ((face[0] + face[2] * 0.5) * w as f32) as u32;
    let cy = ((face[1] + face[3] * 0.45) * h as f32) as u32;
    let crop = Crop {
        x: cx.saturating_sub(640).min(w.saturating_sub(1280)),
        y: cy.saturating_sub(450).min(h.saturating_sub(900)),
        width: 1280.min(w),
        height: 900.min(h),
    };
    let start = Instant::now();
    let native = renderer.render_region(&photo.original, &edit, Some(&seg), crop, None)?;
    ensure!(native.dimensions() == (crop.width, crop.height));
    let cold = start.elapsed().as_secs_f64() * 1000.0;
    native.save(output.join("native-viewport.png"))?;
    report.push_str(&format!(
        "Native viewport {:?}: cold {:.2}ms; peak {:.2} MiB\n",
        (crop.width, crop.height),
        cold,
        peak_working_set() as f64 / 1048576.0
    ));
    let mut warm = vec![];
    for _ in 0..5 {
        let start = Instant::now();
        let result = renderer.render_region(&photo.original, &edit, Some(&seg), crop, None)?;
        warm.push(start.elapsed().as_secs_f64() * 1000.0);
        ensure!(result == native, "Repeated native cache output changed");
    }
    warm.sort_by(f64::total_cmp);
    report.push_str(&format!(
        "Unchanged native viewport: warm median {:.2}ms; all5 byte-identical; peak {:.2} MiB\n",
        warm[2],
        peak_working_set() as f64 / 1048576.0
    ));
    let mut slider = vec![];
    for strength in [25.0, 35.0, 45.0, 55.0, 65.0] {
        edit.settings.under_eyes = strength;
        let start = Instant::now();
        let result = renderer.render_region(&photo.original, &edit, Some(&seg), crop, None)?;
        slider.push(start.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(result);
    }
    slider.sort_by(f64::total_cmp);
    report.push_str(&format!(
        "Native under-eye slider changes: median {:.2}ms; peak {:.2} MiB\n",
        slider[2],
        peak_working_set() as f64 / 1048576.0
    ));
    edit.settings.under_eyes = 45.0;
    drop(renderer);
    let start = Instant::now();
    let rendered = engine::render_cancellable(&photo.original, &edit, Some(&seg), None)?;
    ensure!(rendered.dimensions() == (w, h));
    report.push_str(&format!(
        "Complete tiled render: {:.3}s; peak {:.2} MiB; dimensions {:?}\n",
        start.elapsed().as_secs_f64(),
        peak_working_set() as f64 / 1048576.0,
        rendered.dimensions()
    ));
    let exported_region =
        image::imageops::crop_imm(&rendered, crop.x, crop.y, crop.width, crop.height).to_image();
    let maximum_error = native
        .as_raw()
        .iter()
        .zip(exported_region.as_raw())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    ensure!(
        maximum_error <= 1,
        "Native viewport differs from complete export by {maximum_error} bytes"
    );
    report.push_str(&format!(
        "Native viewport versus complete render: max {maximum_error}-byte channel difference (limit 1)\n"
    ));
    drop(exported_region);
    drop(rendered);
    let start = Instant::now();
    let exported = engine::export_with_options(
        &photo,
        &edit,
        Some(&seg),
        &output,
        &ExportOptions {
            size: ExportSize::Original,
            png: false,
            quality: 90,
            center: [0.5; 2],
            cancel: None,
        },
    )?;
    let dimensions = image::image_dimensions(&exported)?;
    ensure!(dimensions == (w, h), "Export resolution changed");
    report.push_str(&format!("Complete JPG export including rerender: {:.3}s; peak {:.2} MiB; dimensions {:?}; {} bytes; {}\n",start.elapsed().as_secs_f64(),peak_working_set() as f64/1048576.0,dimensions,fs::metadata(&exported)?.len(),exported.display()));
    report.push_str(&format!("Historical untiled linear/filter/mask/cleanup buffers could exceed {:.2} MiB at64bytes/pixel, before originals/output/runtime. This is a buffer estimate, not a measured speedup or direct baseline.\n",w as f64*h as f64*64.0/1048576.0));
    print!("{report}");
    fs::write(output.join("results.txt"), report)?;
    Ok(())
}
