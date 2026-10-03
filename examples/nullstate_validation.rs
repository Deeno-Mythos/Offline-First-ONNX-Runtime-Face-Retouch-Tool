//! Real model traces and output agreement for the NullState lifecycle.
use anyhow::{Result, ensure};
use hastur_retouch::{
    engine::{self, Edit, Segmentation},
    model::{self, Provider},
    nullstate::PortraitDemand,
};
use image::RgbaImage;
use std::{
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

struct Validation {
    image: Arc<RgbaImage>,
    provider: Provider,
    report: String,
}
impl Validation {
    fn phase(
        &mut self,
        name: &str,
        demand: PortraitDemand,
        force: bool,
        base: Option<&Segmentation>,
        allowed: &[&str],
    ) -> Result<Segmentation> {
        model::take_model_run_trace();
        let start = Instant::now();
        let seg = model::analyze_shared_demand_cached_with_status(
            &self.image,
            self.provider,
            force,
            demand,
            base,
            |_| {},
            |_| {},
        )?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        let trace = model::take_model_run_trace();
        ensure!(
            trace.iter().all(|name| allowed.contains(&name.as_str())),
            "{name} ran unexpected models: {trace:?}"
        );
        for expected in allowed {
            ensure!(
                trace.iter().any(|name| name == expected),
                "{name} did not run expected model {expected}: {trace:?}"
            );
        }
        let line = format!(
            "| {name} | {elapsed:.3} | {} | {} |\n",
            model::resident_model_count(),
            if trace.is_empty() {
                "none".into()
            } else {
                trace.join(", ")
            }
        );
        print!("{line}");
        self.report.push_str(&line);
        Ok(seg)
    }
}

fn compare(a: &Segmentation, b: &Segmentation) -> Result<()> {
    ensure!(
        a.prepared == b.prepared
            && a.map_crop == b.map_crop
            && (a.width, a.height) == (b.width, b.height)
            && a.faces == b.faces,
        "Readiness/crop mismatch"
    );
    ensure!(
        a.skin == b.skin
            && a.teeth == b.teeth
            && a.eyes == b.eyes
            && a.under_eyes == b.under_eyes
            && a.forehead == b.forehead
            && a.laugh_lines == b.laugh_lines
            && a.contour == b.contour
            && a.highlight == b.highlight
            && a.blemish == b.blemish
            && a.neural_blend == b.neural_blend
            && a.repair_delta == b.repair_delta,
        "Incremental maps differ from full analysis"
    );
    Ok(())
}

fn same_registered<T: Copy + PartialEq>(
    a: &Segmentation,
    av: &[T],
    b: &Segmentation,
    bv: &[T],
    neutral: T,
) -> bool {
    let value = |seg: &Segmentation, data: &[T], i: usize| {
        if data.len() == (seg.width * seg.height) as usize {
            return data[i];
        }
        if let Some(crop) = seg.map_crop {
            let x = i as u32 % seg.width;
            let y = i as u32 / seg.width;
            if x >= crop.x && x < crop.x + crop.width && y >= crop.y && y < crop.y + crop.height {
                return data[((y - crop.y) * crop.width + x - crop.x) as usize];
            }
        }
        neutral
    };
    (a.width, a.height) == (b.width, b.height)
        && (0..(a.width * a.height) as usize).all(|i| value(a, av, i) == value(b, bv, i))
}

fn main() -> Result<()> {
    let provider = if std::env::args().any(|arg| arg == "--auto") {
        Provider::Auto
    } else {
        Provider::Cpu
    };
    let photo = engine::load_photo(std::path::Path::new("assets/demo-portrait.png"))?;
    let mut image = photo.preview.as_ref().clone();
    // A corner fingerprint avoids existing complete caches masking the staged calls.
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .to_le_bytes();
    for (x, byte) in stamp.into_iter().enumerate() {
        image.get_pixel_mut(x as u32, 0)[0] = byte;
    }
    model::release_idle_resources();
    let mut validation = Validation {
        image: Arc::new(image),
        provider,
        report: format!(
            "# NullState verification\n\nProvider: {}. Image: {:?}. Timings include first-use initialization/qualification where applicable.\n\n| Phase | Milliseconds | Resident models | Logical model calls |\n| --- | ---: | ---: | --- |\n",
            provider.label(),
            photo.preview.dimensions()
        ),
    };
    validation.phase("Dormant", PortraitDemand::NONE, false, None, &[])?;
    ensure!(
        model::resident_model_count() == 0,
        "Dormant request loaded weights"
    );
    let geometry = validation.phase(
        "Face geometry",
        PortraitDemand::GEOMETRY,
        true,
        None,
        &[
            "face_detection_short_range.onnx",
            "face_landmarker_Nx3x256x256.onnx",
        ],
    )?;
    ensure!(!geometry.faces.is_empty(), "No face detected in demo");
    let skin = validation.phase(
        "Add neural skin",
        PortraitDemand {
            geometry: true,
            skin: true,
            blemishes: false,
        },
        false,
        Some(&geometry),
        &["retouch_generator.onnx"],
    )?;
    let complete = validation.phase(
        "Add blemish repair",
        PortraitDemand::ALL,
        false,
        Some(&skin),
        &["local_detection.onnx", "local_inpainting.onnx"],
    )?;
    validation.phase(
        "Reuse completed maps",
        PortraitDemand::ALL,
        false,
        Some(&complete),
        &[],
    )?;
    let full = validation.phase(
        "Fresh full analysis",
        PortraitDemand::ALL,
        true,
        None,
        &[
            "face_detection_short_range.onnx",
            "face_landmarker_Nx3x256x256.onnx",
            "retouch_generator.onnx",
            "local_detection.onnx",
            "local_inpainting.onnx",
        ],
    )?;
    compare(&complete, &full)?;
    // Custom masks must not change the model-space eye/hair protection merely
    // because neural stages are requested after facial geometry.
    let mut custom = geometry.clone();
    custom.custom_skin = true;
    custom.skin = vec![1.0; (custom.width * custom.height) as usize].into();
    model::take_model_run_trace();
    let custom_ready = model::analyze_portrait_demand_with_status(
        &validation.image,
        provider,
        PortraitDemand::ALL,
        Some(&custom),
        |_| {},
        |_| {},
    )?;
    ensure!(
        same_registered(
            &custom_ready,
            &custom_ready.neural_blend,
            &full,
            &full.neural_blend,
            [0.5; 3]
        ) && same_registered(
            &custom_ready,
            &custom_ready.repair_delta,
            &full,
            &full.repair_delta,
            [0.0; 3]
        ) && same_registered(
            &custom_ready,
            &custom_ready.blemish,
            &full,
            &full.blemish,
            0.0
        ),
        "Custom mask changed model-space repair"
    );
    ensure!(
        custom_ready.skin.len() == (custom.width * custom.height) as usize
            || custom_ready.map_crop.is_some(),
        "Custom mask registration was lost"
    );
    ensure!(custom_ready.custom_skin, "Custom mask ownership was lost");
    ensure!(
        model::take_model_run_trace().iter().all(|name| {
            [
                "retouch_generator.onnx",
                "local_detection.onnx",
                "local_inpainting.onnx",
            ]
            .contains(&name.as_str())
        }),
        "Custom mask upgrade redetected faces"
    );
    let mut edit = Edit::default();
    edit.settings.apply_auto_retouch();
    let staged = engine::render(&validation.image, &edit, Some(&complete));
    let fresh = engine::render(&validation.image, &edit, Some(&full));
    ensure!(staged == fresh, "Incremental rendered pixels differ");
    model::release_idle_resources();
    ensure!(
        model::resident_model_count() == 0,
        "Idle release retained model weights"
    );
    validation.phase(
        "Retained maps after idle",
        PortraitDemand::ALL,
        false,
        Some(&complete),
        &[],
    )?;
    ensure!(
        model::resident_model_count() == 0,
        "Reusing maps reloaded weights"
    );
    let cached = validation.phase(
        "Full cache after idle",
        PortraitDemand::ALL,
        false,
        None,
        &[],
    )?;
    compare(&complete, &cached)?;
    let reloaded = validation.phase(
        "Reload after idle",
        PortraitDemand::ALL,
        true,
        None,
        &[
            "face_detection_short_range.onnx",
            "face_landmarker_Nx3x256x256.onnx",
            "retouch_generator.onnx",
            "local_detection.onnx",
            "local_inpainting.onnx",
        ],
    )?;
    compare(&complete, &reloaded)?;
    validation.report.push_str("\nAll assertions passed. Incremental/full/reloaded maps agree exactly; incremental and full renders have identical pixels. Custom masks preserve model-space face protection during incremental repair. Idle release leaves zero resident models while maps/cache remain usable.\n");
    std::fs::create_dir_all("output/nullstate-validation")?;
    let name = if provider == Provider::Auto {
        "auto"
    } else {
        "cpu"
    };
    std::fs::write(
        format!("output/nullstate-validation/{name}.md"),
        &validation.report,
    )?;
    staged.save(format!("output/nullstate-validation/{name}-retouched.png"))?;
    println!("All assertions passed; output/nullstate-validation/{name}.md");
    Ok(())
}
