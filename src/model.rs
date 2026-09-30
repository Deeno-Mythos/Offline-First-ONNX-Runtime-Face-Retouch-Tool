//! Optional, user-supplied ONNX segmentation. No models or network downloads at startup.
use crate::engine::Segmentation;
use anyhow::{Result, bail};
use image::RgbaImage;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    FaceParsing,
    Background,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Cpu,
    DirectMl,
    Cuda,
    CoreMl,
}
impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::DirectMl => "DirectML",
            Self::Cuda => "CUDA",
            Self::CoreMl => "CoreML",
        }
    }
}
pub const AVAILABLE: bool = cfg!(feature = "onnx");

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
    use anyhow::Context;
    use ort::{ep, session::Session, value::TensorRef};
    // Check the shared library explicitly so missing runtime is a recoverable UI error.
    let runtime = std::env::var_os("ORT_DYLIB_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(if cfg!(windows) {
                "models/onnxruntime.dll"
            } else if cfg!(target_os = "macos") {
                "models/libonnxruntime.dylib"
            } else {
                "models/libonnxruntime.so"
            })
        });
    if !runtime.is_file() {
        bail!(
            "ONNX Runtime not found. Set ORT_DYLIB_PATH to its shared library or place it in models/."
        );
    }
    ort::init_from(&runtime)?
        .with_name("Astra Retouch")
        .commit();
    let mut builder = Session::builder()?
        .with_intra_threads(2)
        .map_err(ort::Error::<()>::from)?;
    builder = match provider {
        Provider::Cpu => builder,
        Provider::DirectMl => builder
            .with_parallel_execution(false)
            .map_err(ort::Error::<()>::from)?
            .with_memory_pattern(false)
            .map_err(ort::Error::<()>::from)?
            .with_execution_providers([ep::DirectML::default().build().error_on_failure()])
            .map_err(ort::Error::<()>::from)?,
        Provider::Cuda => builder
            .with_execution_providers([ep::CUDA::default().build().error_on_failure()])
            .map_err(ort::Error::<()>::from)?,
        Provider::CoreMl => builder
            .with_execution_providers([ep::CoreML::default().build().error_on_failure()])
            .map_err(ort::Error::<()>::from)?,
    };
    let mut session = builder
        .commit_from_file(path)
        .context("Cannot load segmentation model")?;
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
        ..Default::default()
    };
    if kind == Kind::FaceParsing {
        if channels != 19 {
            bail!("Face parsing requires a 19-class CelebAMask-HQ BiSeNet model");
        }
        seg.skin = vec![0.0; n];
        seg.teeth = vec![0.0; n];
        seg.eyes = vec![0.0; n];
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
