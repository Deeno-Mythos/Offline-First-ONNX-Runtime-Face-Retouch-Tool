//! Local ONNX portrait analysis and optional custom segmentation.
use crate::{engine::Segmentation, nullstate::PortraitDemand};
use anyhow::{Result, bail};
use image::RgbaImage;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    FaceParsing,
    Background,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Provider {
    Auto,
    Cpu,
    Burn,
    DirectMl,
    Cuda,
    CoreMl,
}
impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto · GPU when qualified",
            Self::Cpu => "CPU",
            Self::Burn => "Burn CPU",
            Self::DirectMl => "DirectML",
            Self::Cuda => "CUDA",
            Self::CoreMl => "CoreML",
        }
    }
}
pub const AVAILABLE: bool = cfg!(feature = "onnx");

/// Progress comes from real cache reads, model loading, inference and qualification.
#[derive(Clone, Debug)]
pub struct AiStatus {
    pub stage: String,
    pub detail: String,
    pub provider: Option<Provider>,
    pub progress: f32,
}
impl AiStatus {
    pub(crate) fn new(
        stage: &str,
        detail: impl Into<String>,
        provider: Option<Provider>,
        progress: f32,
    ) -> Self {
        Self {
            stage: stage.into(),
            detail: detail.into(),
            provider,
            progress,
        }
    }
}

pub fn analyze_cached(
    image: &RgbaImage,
    provider: Provider,
    force: bool,
    stage: impl FnMut(Segmentation),
) -> Result<Segmentation> {
    if !force && let Some(seg) = crate::ai_cache::load(image, provider) {
        return Ok(seg);
    }
    let seg = analyze_portrait(image, provider, stage)?;
    // A cache write failure must never turn a successfully analyzed photo into an error.
    let _ = crate::ai_cache::save(image, &seg, provider);
    Ok(seg)
}
/// Photo-owned images allow safe reuse without hashing their pixels on every request.
pub fn analyze_shared_cached(
    image: &std::sync::Arc<RgbaImage>,
    provider: Provider,
    force: bool,
    stage: impl FnMut(Segmentation),
) -> Result<Segmentation> {
    analyze_shared_cached_with_status(image, provider, force, stage, |_| {})
}
pub fn analyze_shared_cached_with_status(
    image: &std::sync::Arc<RgbaImage>,
    provider: Provider,
    force: bool,
    stage: impl FnMut(Segmentation),
    status: impl FnMut(AiStatus),
) -> Result<Segmentation> {
    analyze_shared_demand_cached_with_status(
        image,
        provider,
        force,
        PortraitDemand::ALL,
        None,
        stage,
        status,
    )
}

pub fn analyze_shared_demand_cached_with_status(
    image: &std::sync::Arc<RgbaImage>,
    provider: Provider,
    force: bool,
    demand: PortraitDemand,
    base: Option<&Segmentation>,
    stage: impl FnMut(Segmentation),
    mut status: impl FnMut(AiStatus),
) -> Result<Segmentation> {
    let demand = demand.normalized();
    if demand.is_empty() {
        return Ok(base.cloned().unwrap_or_default());
    }
    let base = base.filter(|seg| (seg.width, seg.height) == image.dimensions());
    if !force && let Some(seg) = base.filter(|seg| seg.prepared.contains(demand)) {
        status(AiStatus::new("Portrait maps ready", &seg.status, None, 1.0));
        return Ok(seg.clone());
    }
    status(AiStatus::new(
        "Checking portrait cache",
        "Reading local analysis",
        None,
        0.05,
    ));
    if !force && let Some(seg) = crate::ai_cache::load_shared(image, provider) {
        status(AiStatus::new(
            "Cached portrait ready",
            &seg.status,
            None,
            1.0,
        ));
        return Ok(seg);
    }
    let seg = analyze_portrait_demand_with_status(
        image,
        provider,
        demand,
        if force { None } else { base },
        stage,
        &mut status,
    )?;
    if seg.prepared.contains(PortraitDemand::ALL)
        && crate::ai_cache::save(image, &seg, provider).is_ok()
    {
        crate::ai_cache::associate(image, &seg, provider);
    }
    Ok(seg)
}

#[cfg(feature = "onnx")]
#[path = "portrait.rs"]
pub(crate) mod portrait;
#[cfg(feature = "onnx")]
pub use portrait::{
    analyze_portrait, analyze_portrait_demand_with_status, analyze_portrait_with_status,
    resident_model_count, take_model_run_trace,
};

/// Call on the owning inference thread. Photo maps and qualification decisions remain usable.
pub fn release_idle_resources() {
    #[cfg(feature = "onnx")]
    portrait::release_idle_resources();
}

#[cfg(not(feature = "onnx"))]
pub fn take_model_run_trace() -> Vec<String> {
    Vec::new()
}
#[cfg(not(feature = "onnx"))]
pub fn resident_model_count() -> usize {
    0
}

#[cfg(not(feature = "onnx"))]
pub fn analyze_portrait_demand_with_status(
    image: &RgbaImage,
    provider: Provider,
    demand: PortraitDemand,
    base: Option<&Segmentation>,
    stage: impl FnMut(Segmentation),
    status: impl FnMut(AiStatus),
) -> Result<Segmentation> {
    if demand.is_empty() {
        return Ok(base.cloned().unwrap_or_default());
    }
    analyze_portrait_with_status(image, provider, stage, status)
}
#[cfg(not(feature = "onnx"))]
pub fn analyze_portrait(
    _: &RgbaImage,
    _: Provider,
    _: impl FnMut(Segmentation),
) -> Result<Segmentation> {
    bail!("This build has no ONNX support. Enable the onnx feature for portrait AI.")
}
#[cfg(not(feature = "onnx"))]
pub fn analyze_portrait_with_status(
    image: &RgbaImage,
    provider: Provider,
    stage: impl FnMut(Segmentation),
    _: impl FnMut(AiStatus),
) -> Result<Segmentation> {
    analyze_portrait(image, provider, stage)
}

pub fn model_path(name: &str) -> std::path::PathBuf {
    let dirs = [
        std::path::PathBuf::from("models"),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("models")))
            .unwrap_or_default(),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models"),
    ];
    dirs.iter()
        .map(|p| p.join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| dirs[0].join(name))
}

#[cfg(feature = "onnx")]
pub(crate) fn session(path: &Path, provider: Provider) -> Result<ort::session::Session> {
    session_with_profile(path, provider, None)
}
#[cfg(feature = "onnx")]
pub(crate) fn session_with_profile(
    path: &Path,
    provider: Provider,
    profile: Option<&Path>,
) -> Result<ort::session::Session> {
    use anyhow::Context;
    use ort::{ep, session::Session};
    let runtime = std::env::var_os("ORT_DYLIB_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            // ORT loads one runtime for the process. A DML build also supports CPU,
            // so use it from the outset to allow switching between GPU and CPU.
            let dml = model_path("directml/onnxruntime.dll");
            if cfg!(windows) && dml.is_file() && model_path("directml/DirectML.dll").is_file() {
                return dml;
            }
            model_path(if cfg!(windows) {
                "onnxruntime.dll"
            } else if cfg!(target_os = "macos") {
                "libonnxruntime.dylib"
            } else {
                "libonnxruntime.so"
            })
        });
    if !runtime.is_file() {
        bail!(
            "ONNX Runtime is missing: {}. Run scripts/setup-ai.ps1 or set ORT_DYLIB_PATH.",
            runtime.display()
        );
    }
    ort::init_from(&runtime)?
        .with_name("Hastur Retouch")
        .commit();
    let mut builder = Session::builder()?
        .with_intra_threads(4)
        .map_err(ort::Error::<()>::from)?
        // Sleeping between runs prevents five retained sessions from competing with
        // preview/export workers. Tiny denormal values are irrelevant at image precision.
        .with_config_entry("session.intra_op.allow_spinning", "0")
        .map_err(ort::Error::<()>::from)?
        .with_config_entry("session.inter_op.allow_spinning", "0")
        .map_err(ort::Error::<()>::from)?
        .with_config_entry("session.set_denormal_as_zero", "1")
        .map_err(ort::Error::<()>::from)?;
    builder = match provider {
        Provider::Auto | Provider::Cpu | Provider::Burn => builder,
        Provider::DirectMl => {
            builder = builder
                .with_parallel_execution(false)
                .map_err(ort::Error::<()>::from)?
                .with_memory_pattern(false)
                .map_err(ort::Error::<()>::from)?;
            // Vendor metacommands on older Intel drivers return incorrect face
            // tensors. Keep actual DirectML GPU kernels, but bypass that path.
            use ort::AsPointer;
            let keys = [c"device_id".as_ptr(), c"disable_metacommands".as_ptr()];
            let values = [c"0".as_ptr(), c"true".as_ptr()];
            // SAFETY: builder owns the live options; static C strings and arrays
            // stay valid for the synchronous v24 ORT call.
            unsafe {
                ort::Error::result_from_status(
                    (ort::api().SessionOptionsAppendExecutionProvider)(
                        builder.ptr_mut(),
                        c"DML".as_ptr(),
                        keys.as_ptr(),
                        values.as_ptr(),
                        keys.len(),
                    ),
                )?;
            }
            builder
        }
        Provider::Cuda => builder
            .with_execution_providers([ep::CUDA::default().build().error_on_failure()])
            .map_err(ort::Error::<()>::from)?,
        Provider::CoreMl => builder
            .with_execution_providers([ep::CoreML::default().build().error_on_failure()])
            .map_err(ort::Error::<()>::from)?,
    };
    if let Some(profile) = profile {
        builder = builder
            .with_profiling(profile)
            .map_err(ort::Error::<()>::from)?;
    }
    builder
        .commit_from_file(path)
        .with_context(|| format!("Cannot load {}", path.display()))
}

#[cfg(not(feature = "onnx"))]
pub fn infer(_: &Path, _: Kind, _: Provider, _: &RgbaImage) -> Result<Segmentation> {
    bail!(
        "This build has no ONNX support. Build with cargo run --features onnx, and install ONNX Runtime plus a compatible model."
    );
}

#[cfg(feature = "onnx")]
pub fn infer(
    path: &Path,
    kind: Kind,
    provider: Provider,
    image: &RgbaImage,
) -> Result<Segmentation> {
    if provider == Provider::Burn {
        bail!(
            "Burn inference currently supports the bundled portrait models only. Choose CPU for a custom ONNX mask model."
        );
    }
    // Imported mask models have different output contracts. Auto keeps their
    // established CPU behavior; explicit GPU providers remain available.
    let provider = if provider == Provider::Auto {
        Provider::Cpu
    } else {
        provider
    };
    use ort::value::TensorRef;
    let mut session = session(path, provider)?;
    let size = if kind == Kind::FaceParsing { 512 } else { 1024 };
    let resized = image::imageops::resize(image, size, size, image::imageops::FilterType::Triangle);
    let mut input = ndarray::Array4::<f32>::zeros((1, 3, size as usize, size as usize));
    for (x, y, p) in resized.enumerate_pixels() {
        for c in 0..3 {
            let v = p[c] as f32 / 255.0;
            input[[0, c, y as usize, x as usize]] = if kind == Kind::FaceParsing {
                (v - [0.485, 0.456, 0.406][c]) / [0.229, 0.224, 0.225][c]
            } else {
                v - 0.5
            };
        }
    }
    let outputs = session.run(ort::inputs![TensorRef::from_array_view(&input)?])?;
    let (shape, data) = outputs[0].try_extract_tensor::<f32>()?;
    if shape.len() != 4 || shape[0] != 1 {
        bail!("Expected float32 NCHW output [1, channels, height, width]");
    }
    let channels = shape[1] as usize;
    let h = shape[2] as u32;
    let w = shape[3] as u32;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 16_000_000 {
        bail!("Invalid segmentation output size");
    }
    let n = (w * h) as usize;
    let mut seg = Segmentation {
        width: w,
        height: h,
        status: format!("{} · custom ONNX mask ready", provider.label()),
        ..Default::default()
    };
    if kind == Kind::FaceParsing {
        if channels != 19 {
            bail!("Face parsing requires a 19-class CelebAMask-HQ BiSeNet model");
        }
        seg.skin = vec![0.0; n].into();
        seg.teeth = vec![0.0; n].into();
        seg.eyes = vec![0.0; n].into();
        for i in 0..n {
            let class = (0..channels)
                .max_by(|&a, &b| data[a * n + i].total_cmp(&data[b * n + i]))
                .unwrap_or(0);
            if matches!(class, 1 | 7 | 8 | 10 | 14) {
                seg.skin[i] = 1.0;
            }
            if matches!(class, 4 | 5) {
                seg.eyes[i] = 1.0;
            }
            // Class 11 is the mouth cavity, not teeth; teeth are always painted manually.
        }
    } else {
        if channels != 1 {
            bail!("Background removal requires a single-channel foreground probability model");
        }
        if data
            .iter()
            .any(|v| !v.is_finite() || *v < -0.01 || *v > 1.01)
        {
            bail!("Background output must contain foreground probabilities in [0, 1], not logits");
        }
        seg.background = data[..n].iter().map(|&v| 1.0 - v.clamp(0.0, 1.0)).collect();
    }
    Ok(seg)
}
