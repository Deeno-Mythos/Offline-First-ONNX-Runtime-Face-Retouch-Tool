//! MediaPipe BlazeFace/478-point mesh and pretrained ModelScope skin retouching.
//! Model contracts and upstream provenance are documented in models/README.md.
use super::{AiStatus, Provider, model_path, session_with_profile};
use crate::{
    engine::{Segmentation, sample_channel},
    geometry::{FaceMesh, sample_rgba},
    nullstate::PortraitDemand,
};
use anyhow::{Result, bail, ensure};
use image::RgbaImage;
use ndarray::Array4;
use ort::{session::Session, value::TensorRef};
use rayon::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
};

type SessionKey = (PathBuf, Provider, u64, std::time::SystemTime);
struct SessionRecord {
    session: Session,
    verify_gpu: bool,
}
thread_local! {
    static SESSIONS: RefCell<HashMap<SessionKey, SessionRecord>> = RefCell::new(HashMap::new());
    static AUTO: RefCell<HashMap<SessionKey, (Provider, String)>> = RefCell::new(HashMap::new());
    static REQUESTED: RefCell<Option<Provider>> = const { RefCell::new(None) };
    static USED: RefCell<Vec<Provider>> = const { RefCell::new(Vec::new()) };
    static FALLBACKS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static MODEL_TRACE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static TRACING: Cell<bool> = const { Cell::new(false) };
}

/// Drain the current AI thread's recent logical model invocations for diagnostics.
pub fn take_model_run_trace() -> Vec<String> {
    TRACING.with(|tracing| tracing.set(true));
    MODEL_TRACE.with(|trace| std::mem::take(&mut *trace.borrow_mut()))
}

/// Resident model weights on the calling AI thread; disk result caches are separate.
pub fn resident_model_count() -> usize {
    let count = SESSIONS.with(|cache| cache.borrow().len());
    #[cfg(astra_burn_models)]
    let count = count + crate::burn_inference::resident_model_count();
    count
}

/// Release unused weights on their owning worker without discarding GPU qualification.
pub(crate) fn release_idle_resources() {
    SESSIONS.with(|cache| cache.borrow_mut().clear());
    #[cfg(astra_burn_models)]
    crate::burn_inference::release_idle_resources();
}
type Output = (Vec<i64>, Vec<f32>);
fn model_stage(name: &str) -> (&'static str, f32) {
    match name {
        "face_detection_short_range.onnx" => ("Detecting faces", 0.15),
        "face_landmarker_Nx3x256x256.onnx" => ("Mapping facial landmarks", 0.3),
        "retouch_generator.onnx" => ("Preparing neural skin retouch", 0.5),
        "local_detection.onnx" => ("Detecting blemishes", 0.7),
        _ => ("Preparing blemish repair", 0.9),
    }
}
fn gpu_candidate() -> Provider {
    if cfg!(windows) {
        Provider::DirectMl
    } else if cfg!(target_os = "macos") {
        Provider::CoreMl
    } else {
        Provider::Cuda
    }
}
fn record_used(provider: Provider) {
    USED.with(|used| {
        let mut used = used.borrow_mut();
        if !used.contains(&provider) {
            used.push(provider);
        }
    });
}
fn backend_label(requested: Provider) -> String {
    if requested != Provider::Auto {
        return requested.label().into();
    }
    USED.with(|used| {
        let used = used.borrow();
        if used.len() > 1 {
            format!("{} GPU + CPU fallback", gpu_candidate().label())
        } else if used.first() == Some(&Provider::Cpu) {
            "CPU fallback".into()
        } else {
            format!("{} GPU", gpu_candidate().label())
        }
    })
}
fn output_agreement(cpu: &[Output], gpu: &[Output]) -> bool {
    cpu.len() == gpu.len()
        && cpu.iter().zip(gpu).all(|(a, b)| {
            a.0 == b.0
                && a.1.len() == b.1.len()
                && a.1.iter().zip(&b.1).all(|(&c, &g)| {
                    c.is_finite() && g.is_finite() && (c - g).abs() <= 0.005 + c.abs() * 0.003
                })
        })
}
fn profile_has_provider(profile: &str, provider: &str) -> bool {
    profile.split("\"provider\"").skip(1).any(|tail| {
        tail.trim_start()
            .strip_prefix(':')
            .and_then(|v| v.trim_start().strip_prefix('"'))
            .is_some_and(|v| {
                v.strip_prefix(provider)
                    .is_some_and(|rest| rest.starts_with('"'))
            })
    })
}
fn run_ort(name: &str, provider: Provider, inputs: &[Array4<f32>]) -> Result<Vec<Output>> {
    let start = std::time::Instant::now();
    let result = SESSIONS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let path = model_path(name);
        let meta = std::fs::metadata(&path)?;
        let key = (path, provider, meta.len(), meta.modified()?);
        cache.retain(|(path, _, len, modified), _| {
            path != &key.0 || (*len, *modified) == (key.2, key.3)
        });
        if !cache.contains_key(&key) {
            let verify_gpu = !matches!(provider, Provider::Cpu | Provider::Burn | Provider::Auto);
            let prefix = if verify_gpu {
                let dir = std::env::var_os("HASTUR_AI_PROFILE")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir);
                std::fs::create_dir_all(&dir)?;
                Some(dir.join(format!(
                    "hastur-{}-{}-{}",
                    std::process::id(),
                    provider.label(),
                    name
                )))
            } else {
                None
            };
            let session = session_with_profile(&key.0, provider, prefix.as_deref())?;
            cache.insert(
                key.clone(),
                SessionRecord {
                    session,
                    verify_gpu,
                },
            );
        }
        let record = cache.get_mut(&key).unwrap();
        let output = if inputs.len() == 2 {
            record.session.run(ort::inputs![
                TensorRef::from_array_view(&inputs[0])?,
                TensorRef::from_array_view(&inputs[1])?
            ])?
        } else {
            record
                .session
                .run(ort::inputs![TensorRef::from_array_view(&inputs[0])?])?
        };
        let result: Vec<Output> = output
            .iter()
            .map(|(_, tensor)| {
                let (shape, data) = tensor.try_extract_tensor::<f32>()?;
                ensure!(
                    data.iter().all(|v| v.is_finite()),
                    "Non-finite values from {name}"
                );
                Ok((shape.to_vec(), data.to_vec()))
            })
            .collect::<Result<_>>()?;
        drop(output);
        if record.verify_gpu {
            let profile = PathBuf::from(record.session.end_profiling()?);
            let data = std::fs::read_to_string(&profile)?;
            let ep = match provider {
                Provider::DirectMl => "DmlExecutionProvider",
                Provider::Cuda => "CUDAExecutionProvider",
                Provider::CoreMl => "CoreMLExecutionProvider",
                _ => unreachable!(),
            };
            let executed = profile_has_provider(&data, ep);
            if std::env::var_os("HASTUR_AI_PROFILE").is_none() {
                let _ = std::fs::remove_file(profile);
            }
            ensure!(executed, "{name} did not execute any {ep} GPU kernels");
            record.verify_gpu = false;
        }
        Ok(result)
    });
    if std::env::var_os("HASTUR_AI_TIMINGS").is_some()
        || std::env::var_os("ASTRA_AI_TIMINGS").is_some()
    {
        eprintln!(
            "{} {name}: {:.2} ms",
            provider.label(),
            start.elapsed().as_secs_f64() * 1000.
        );
    }
    result
}
fn run(
    name: &str,
    provider: Provider,
    inputs: &[Array4<f32>],
    status: &mut dyn FnMut(AiStatus),
) -> Result<Vec<Output>> {
    if TRACING.with(Cell::get) {
        MODEL_TRACE.with(|trace| {
            let mut trace = trace.borrow_mut();
            if trace.len() == 128 {
                trace.remove(0);
            }
            trace.push(name.into());
        });
    }
    let (stage, progress) = model_stage(name);
    status(AiStatus::new(
        stage,
        format!("Loading/running {name}"),
        Some(provider),
        progress,
    ));
    #[cfg(astra_burn_models)]
    if provider == Provider::Burn {
        record_used(provider);
        return crate::burn_inference::run(name, inputs);
    }
    #[cfg(not(astra_burn_models))]
    if provider == Provider::Burn {
        bail!("Burn models are not included in this build. Install AI models and rebuild.");
    }
    if provider != Provider::Auto {
        let result = run_ort(name, provider, inputs)?;
        record_used(provider);
        return Ok(result);
    }
    let path = model_path(name);
    let meta = std::fs::metadata(&path)?;
    let key = (path, Provider::Auto, meta.len(), meta.modified()?);
    let choice = AUTO.with(|cache| cache.borrow().get(&key).cloned());
    if let Some((selected, reason)) = choice {
        if selected == Provider::Cpu && !reason.is_empty() {
            FALLBACKS.with(|f| {
                let mut f = f.borrow_mut();
                if !f.contains(&reason) {
                    f.push(reason.clone());
                }
            });
        }
        match run_ort(name, selected, inputs) {
            Ok(output) => {
                record_used(selected);
                status(AiStatus::new(stage, reason, Some(selected), progress));
                return Ok(output);
            }
            Err(error) if selected != Provider::Cpu => {
                let reason = format!("{name}: GPU error; CPU fallback ({error:#})");
                AUTO.with(|cache| {
                    cache
                        .borrow_mut()
                        .insert(key.clone(), (Provider::Cpu, reason.clone()));
                });
                FALLBACKS.with(|f| f.borrow_mut().push(reason.clone()));
                status(AiStatus::new(
                    "Using CPU fallback",
                    reason,
                    Some(Provider::Cpu),
                    progress,
                ));
                SESSIONS.with(|cache| {
                    cache
                        .borrow_mut()
                        .retain(|(p, provider, _, _), _| p != &key.0 || *provider == Provider::Cpu)
                });
            }
            Err(error) => return Err(error),
        }
    } else {
        status(AiStatus::new(
            "Qualifying GPU output",
            format!("Comparing {name} with CPU on this portrait"),
            None,
            progress,
        ));
        let reference = run_ort(name, Provider::Cpu, inputs)?;
        let candidate = gpu_candidate();
        let gpu = run_ort(name, candidate, inputs);
        let (selected, reason, output) = match gpu {
            Ok(output) if output_agreement(&reference, &output) => (
                candidate,
                format!(
                    "{} GPU kernels verified; output agrees with CPU",
                    candidate.label()
                ),
                output,
            ),
            Ok(_) => (
                Provider::Cpu,
                format!("{name}: GPU output differs from CPU; CPU fallback"),
                reference,
            ),
            Err(error) => (
                Provider::Cpu,
                format!("{name}: CPU fallback ({error:#})"),
                reference,
            ),
        };
        AUTO.with(|cache| {
            cache
                .borrow_mut()
                .retain(|(p, _, len, time), _| p != &key.0 || (*len, *time) == (key.2, key.3));
            cache
                .borrow_mut()
                .insert(key.clone(), (selected, reason.clone()));
        });
        // Qualification's temporary reference session need not remain resident.
        SESSIONS.with(|cache| {
            cache
                .borrow_mut()
                .retain(|(p, provider, _, _), _| p != &key.0 || *provider == selected)
        });
        if selected == Provider::Cpu {
            FALLBACKS.with(|f| f.borrow_mut().push(reason.clone()));
        }
        record_used(selected);
        status(AiStatus::new(stage, reason, Some(selected), progress));
        return Ok(output);
    }
    let output = run_ort(name, Provider::Cpu, inputs)?;
    record_used(Provider::Cpu);
    Ok(output)
}
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x.clamp(-80.0, 80.0)).exp())
}
fn blob(image: &RgbaImage, size: u32, rect: [f32; 4], angle: f32, normalize: bool) -> Array4<f32> {
    let mut input = Array4::zeros((1, 3, size as usize, size as usize));
    let (cos, sin) = (angle.cos(), angle.sin());
    let plane = (size * size) as usize;
    let (red, rest) = input.as_slice_mut().unwrap().split_at_mut(plane);
    let (green, blue) = rest.split_at_mut(plane);
    red.par_chunks_mut(size as usize)
        .zip(green.par_chunks_mut(size as usize))
        .zip(blue.par_chunks_mut(size as usize))
        .enumerate()
        .for_each(|(y, ((red, green), blue))| {
            for x in 0..size {
                let dx = (x as f32 / size as f32 - 0.5) * rect[2];
                let dy = (y as f32 / size as f32 - 0.5) * rect[3];
                let sx = rect[0] + dx * cos - dy * sin;
                let sy = rect[1] + dx * sin + dy * cos;
                let p = if sx < 0.0
                    || sy < 0.0
                    || sx >= image.width() as f32
                    || sy >= image.height() as f32
                {
                    [0.0; 4]
                } else {
                    sample_rgba(image, sx, sy)
                };
                for (c, pixel) in p.iter().take(3).enumerate() {
                    let v = pixel / 255.0;
                    let value = if normalize { v * 2.0 - 1.0 } else { v };
                    match c {
                        0 => red[x as usize] = value,
                        1 => green[x as usize] = value,
                        _ => blue[x as usize] = value,
                    };
                }
            }
        });
    input
}

#[derive(Clone)]
struct Detection {
    score: f32,
    values: [f32; 16],
}
fn iou(a: &Detection, b: &Detection) -> f32 {
    let a = &a.values;
    let b = &b.values;
    let left = (a[0] - a[2] * 0.5).max(b[0] - b[2] * 0.5);
    let right = (a[0] + a[2] * 0.5).min(b[0] + b[2] * 0.5);
    let top = (a[1] - a[3] * 0.5).max(b[1] - b[3] * 0.5);
    let bottom = (a[1] + a[3] * 0.5).min(b[1] + b[3] * 0.5);
    let area = (right - left).max(0.0) * (bottom - top).max(0.0);
    area / (a[2] * a[3] + b[2] * b[3] - area).max(0.00001)
}
fn nms(mut detections: Vec<Detection>) -> Vec<Detection> {
    detections.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut output = vec![];
    while !detections.is_empty() {
        let mut top = detections[0].clone();
        let mut sum = 0.0;
        let mut values = [0.0; 16];
        detections.retain(|d| {
            if iou(d, &top) > 0.3 {
                sum += d.score;
                for (c, v) in values.iter_mut().enumerate() {
                    *v += d.values[c] * d.score;
                }
                false
            } else {
                true
            }
        });
        for (c, v) in top.values.iter_mut().enumerate() {
            *v = values[c] / sum.max(0.00001);
        }
        output.push(top);
    }
    output
}
fn detect(
    image: &RgbaImage,
    provider: Provider,
    tile: [f32; 4],
    status: &mut dyn FnMut(AiStatus),
) -> Result<Vec<Detection>> {
    let side = tile[2].max(tile[3]);
    let rect = [tile[0] + tile[2] * 0.5, tile[1] + tile[3] * 0.5, side, side];
    let out = run(
        "face_detection_short_range.onnx",
        provider,
        &[blob(image, 128, rect, 0.0, true)],
        status,
    )?;
    ensure!(
        out.len() == 2 && out[0].1.len() == 896 * 16 && out[1].1.len() == 896,
        "Unexpected BlazeFace output contract"
    );
    let mut anchors = Vec::with_capacity(896);
    for (cells, repeats) in [(16, 2), (8, 6)] {
        for y in 0..cells {
            for x in 0..cells {
                for _ in 0..repeats {
                    anchors.push([
                        (x as f32 + 0.5) / cells as f32,
                        (y as f32 + 0.5) / cells as f32,
                    ]);
                }
            }
        }
    }
    let mut found = vec![];
    for (i, anchor) in anchors.iter().enumerate() {
        let score = sigmoid(out[1].1[i]);
        if score < 0.6 {
            continue;
        }
        let reg = &out[0].1[i * 16..(i + 1) * 16];
        let mut v = [0.0; 16];
        for c in 0..2 {
            v[c] = rect[c] + (reg[c] / 128.0 + anchor[c] - 0.5) * side;
            v[c + 2] = reg[c + 2] / 128.0 * side;
        }
        for k in 0..6 {
            for c in 0..2 {
                v[4 + 2 * k + c] = rect[c] + (reg[4 + 2 * k + c] / 128.0 + anchor[c] - 0.5) * side;
            }
        }
        if v[2] > 3.0 && v[3] > 3.0 {
            found.push(Detection { score, values: v });
        }
    }
    Ok(nms(found))
}
fn mesh(
    image: &RgbaImage,
    d: &Detection,
    provider: Provider,
    status: &mut dyn FnMut(AiStatus),
) -> Result<Option<FaceMesh>> {
    let v = d.values;
    let side = v[2].max(v[3]) * 1.5;
    let angle = (v[7] - v[5]).atan2(v[6] - v[4]);
    let out = run(
        "face_landmarker_Nx3x256x256.onnx",
        provider,
        &[blob(image, 256, [v[0], v[1], side, side], angle, false)],
        status,
    )?;
    ensure!(
        out.len() == 2 && out[0].1.len() == 478 * 3 && !out[1].1.is_empty(),
        "Unexpected 478-point face mesh output"
    );
    let confidence = sigmoid(out[1].1[0]).min(d.score);
    if confidence < 0.65 {
        return Ok(None);
    }
    let (cos, sin) = (angle.cos(), angle.sin());
    let landmarks = out[0]
        .1
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| {
            let (dx, dy) = ((p[0] - 128.0) * side / 256.0, (p[1] - 128.0) * side / 256.0);
            [
                (v[0] + dx * cos - dy * sin) / image.width() as f32,
                (v[1] + dx * sin + dy * cos) / image.height() as f32,
                p[2] * side / 256.0 / image.width().min(image.height()) as f32,
            ]
        })
        .collect();
    Ok(Some(FaceMesh {
        landmarks,
        confidence,
        bounds: [
            (v[0] - v[2] * 0.5) / image.width() as f32,
            (v[1] - v[3] * 0.5) / image.height() as f32,
            v[2] / image.width() as f32,
            v[3] / image.height() as f32,
        ],
    }))
}

const OVAL: &[usize] = &[
    10, 338, 297, 332, 284, 251, 389, 356, 454, 323, 361, 288, 397, 365, 379, 378, 400, 377, 152,
    148, 176, 149, 150, 136, 172, 58, 132, 93, 234, 127, 162, 21, 54, 103, 67, 109,
];
const EYE_R: &[usize] = &[
    33, 7, 163, 144, 145, 153, 154, 155, 133, 173, 157, 158, 159, 160, 161, 160, 246,
];
const EYE_L: &[usize] = &[
    263, 249, 390, 373, 374, 380, 381, 382, 362, 398, 384, 385, 386, 387, 388, 466,
];
const LIPS: &[usize] = &[
    61, 146, 91, 181, 84, 17, 314, 405, 321, 375, 291, 409, 270, 269, 267, 0, 37, 39, 40, 185,
];
const MOUTH: &[usize] = &[
    78, 95, 88, 178, 87, 14, 317, 402, 318, 324, 308, 415, 310, 311, 312, 13, 82, 81, 80, 191,
];
const BROW_R: &[usize] = &[70, 63, 105, 66, 107, 55, 65, 52, 53, 46];
const BROW_L: &[usize] = &[300, 293, 334, 296, 336, 285, 295, 282, 283, 276];
fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let t =
        ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]).max(0.0000001);
    (p[0] - a[0] - d[0] * t.clamp(0.0, 1.0)).hypot(p[1] - a[1] - d[1] * t.clamp(0.0, 1.0))
}
fn polygon(p: [f32; 2], points: &[[f32; 2]], feather: f32) -> f32 {
    let mut inside = false;
    let mut distance = f32::MAX;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        distance = distance.min(segment_distance(p, a, b));
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
    }
    let signed = if inside { distance } else { -distance };
    (0.5 + signed / feather.max(0.00001)).clamp(0.0, 1.0)
}
fn ellipse(p: [f32; 2], c: [f32; 2], r: [f32; 2]) -> f32 {
    let d = ((p[0] - c[0]) / r[0].max(0.0001)).hypot((p[1] - c[1]) / r[1].max(0.0001));
    (1.0 - d * d).clamp(0.0, 1.0).powi(2)
}

#[derive(Clone, Copy)]
pub(crate) struct EyeRegion {
    pub(crate) lower: [f32; 2],
    pub(crate) axis: [f32; 2],
    pub(crate) down: [f32; 2],
    pub(crate) width: f32,
}

fn eye_region(a: [f32; 2], b: [f32; 2], lower: [f32; 2], chin: [f32; 2]) -> Option<EyeRegion> {
    let delta = [b[0] - a[0], b[1] - a[1]];
    let width = delta[0].hypot(delta[1]);
    if width < 1.0 {
        return None;
    }
    let axis = [delta[0] / width, delta[1] / width];
    let mut down = [-axis[1], axis[0]];
    let center = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    if (chin[0] - center[0]) * down[0] + (chin[1] - center[1]) * down[1] < 0.0 {
        down = [-down[0], -down[1]];
    }
    Some(EyeRegion {
        lower,
        axis,
        down,
        width,
    })
}

pub(crate) fn eye_regions(face: &FaceMesh, w: u32, h: u32) -> Vec<EyeRegion> {
    let point = |i: usize| {
        [
            face.landmarks[i][0] * w as f32,
            face.landmarks[i][1] * h as f32,
        ]
    };
    let chin = point(152);
    [(33, 133, 145), (362, 263, 374)]
        .into_iter()
        .filter_map(|(a, b, lower)| eye_region(point(a), point(b), point(lower), chin))
        .collect()
}

fn under_eye_weight(p: [f32; 2], eye: EyeRegion) -> f32 {
    let dx = p[0] - eye.lower[0] - eye.down[0] * eye.width * 0.42;
    let dy = p[1] - eye.lower[1] - eye.down[1] * eye.width * 0.42;
    let along = dx * eye.axis[0] + dy * eye.axis[1];
    let below = dx * eye.down[0] + dy * eye.down[1];
    let d = (along / (eye.width * 0.64)).hypot(below / (eye.width * 0.46));
    (1.0 - d * d).clamp(0.0, 1.0).powi(2)
}

fn hair_exclusion(image: &RgbaImage, x: u32, y: u32, radius: i32) -> f32 {
    let pixel = image.get_pixel(x, y).0;
    let [r, g, b] = [pixel[0], pixel[1], pixel[2]].map(|v| v as f32 / 255.0);
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let spread = r.max(g).max(b) - r.min(g).min(b);
    let warm_skin = ((r - b - 0.025) / 0.13).clamp(0.0, 1.0);
    let dark_neutral = ((0.4 - luminance) / 0.32).clamp(0.0, 1.0)
        * ((0.21 - spread) / 0.16).clamp(0.0, 1.0)
        * (1.0 - warm_skin * 0.9);

    let (mut surrounding, mut count) = (0.0, 0);
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx == 0 && dy == 0 {
                continue;
            }
            let sx = (x as i32 + dx).clamp(0, image.width() as i32 - 1) as u32;
            let sy = (y as i32 + dy).clamp(0, image.height() as i32 - 1) as u32;
            let neighbor = image.get_pixel(sx, sy).0;
            surrounding += (0.2126 * neighbor[0] as f32
                + 0.7152 * neighbor[1] as f32
                + 0.0722 * neighbor[2] as f32)
                / 255.0;
            count += 1;
        }
    }
    let local_shadow =
        ((surrounding / count.max(1) as f32 - luminance - 0.07) / 0.24).clamp(0.0, 1.0) * 0.9;
    1.0 - (1.0 - dark_neutral) * (1.0 - local_shadow)
}

fn masks(image: &RgbaImage, faces: Vec<FaceMesh>) -> Segmentation {
    let (w, h) = image.dimensions();
    let n = (w * h) as usize;
    let mut seg = Segmentation {
        width: w,
        height: h,
        faces,
        skin: vec![0.0; n].into(),
        teeth: vec![0.0; n].into(),
        eyes: vec![0.0; n].into(),
        under_eyes: vec![0.0; n].into(),
        forehead: vec![0.0; n].into(),
        laugh_lines: vec![0.0; n].into(),
        contour: vec![0.0; n].into(),
        highlight: vec![0.0; n].into(),
        ..Default::default()
    };
    for face in &seg.faces {
        let point = |i: usize| [face.landmarks[i][0], face.landmarks[i][1]];
        let polygon_points =
            |indices: &[usize]| indices.iter().map(|&i| point(i)).collect::<Vec<_>>();
        let oval = polygon_points(OVAL);
        let exclusions = [EYE_R, EYE_L, LIPS, BROW_R, BROW_L].map(polygon_points);
        let mouth = polygon_points(MOUTH);
        let under_eye_regions = eye_regions(face, w, h);
        let fw = (point(454)[0] - point(234)[0]).abs();
        let fh = (point(152)[1] - point(10)[1]).abs();
        let hair_radius = (fw * w as f32 * 0.006).round().clamp(1.0, 8.0) as i32;
        let feather = fw * 0.045;
        let x0 = (oval.iter().map(|p| p[0]).fold(1.0, f32::min) * w as f32)
            .floor()
            .max(0.0) as u32;
        let x1 = (oval.iter().map(|p| p[0]).fold(0.0, f32::max) * w as f32)
            .ceil()
            .min(w as f32) as u32;
        let y0 = (oval.iter().map(|p| p[1]).fold(1.0, f32::min) * h as f32)
            .floor()
            .max(0.0) as u32;
        let y1 = (oval.iter().map(|p| p[1]).fold(0.0, f32::max) * h as f32)
            .ceil()
            .min(h as f32) as u32;
        let face_masks: Vec<[f32; 8]> = (0..y1.saturating_sub(y0) * x1.saturating_sub(x0))
            .into_par_iter()
            .map(|i| {
                let (x, y) = (x0 + i % (x1 - x0), y0 + i / (x1 - x0));
                let mut result = [0.0; 8];
                let uv = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
                let base = polygon(uv, &oval, feather);
                let excluded = exclusions
                    .iter()
                    .map(|p| polygon(uv, p, feather * 0.75))
                    .fold(0.0, f32::max);
                let hair = hair_exclusion(image, x, y, hair_radius);
                let skin = base * (1.0 - excluded) * (1.0 - hair);
                result[0] = skin;
                let eyes = exclusions[..2]
                    .iter()
                    .map(|p| polygon(uv, p, feather * 0.3))
                    .fold(0.0, f32::max);
                result[1] = eyes;
                let pixel = image.get_pixel(x, y);
                let min = *pixel.0[..3].iter().min().unwrap() as f32 / 255.0;
                let max = *pixel.0[..3].iter().max().unwrap() as f32 / 255.0;
                result[2] = polygon(uv, &mouth, feather * 0.25)
                    * ((min - 0.3) / 0.3).clamp(0.0, 1.0)
                    * (1.0 - (max - min) / 0.25).clamp(0.0, 1.0);
                let pixel_uv = [uv[0] * w as f32, uv[1] * h as f32];
                let bag = under_eye_regions
                    .iter()
                    .map(|eye| under_eye_weight(pixel_uv, *eye))
                    .fold(0.0, f32::max);
                result[3] = bag * skin;
                let brow = (point(105)[1] + point(334)[1]) * 0.5;
                result[4] = ellipse(
                    uv,
                    [
                        (point(10)[0] + point(9)[0]) * 0.5,
                        (point(10)[1] + brow) * 0.5,
                    ],
                    [fw * 0.38, (brow - point(10)[1]).abs() * 0.6],
                ) * skin;
                let mut laugh = 0.0f32;
                for (nose, lip, chin) in [(98, 61, 136), (327, 291, 365)] {
                    let a = point(nose);
                    let b = point(lip);
                    let end = point(chin);
                    let middle = [b[0] + (end[0] - b[0]) * 0.2, b[1] + fh * 0.03];
                    let dist = segment_distance(uv, a, middle);
                    laugh = laugh.max((1.0 - dist / (fw * 0.075)).clamp(0.0, 1.0).powi(2));
                }
                result[5] = laugh * skin;
                let mut contour = 0.0f32;
                let mut highlight = 0.0f32;
                for (cheek, high) in [(132, 50), (361, 280)] {
                    let c = point(cheek);
                    contour = contour.max(ellipse(
                        uv,
                        [c[0], c[1] + fh * 0.06],
                        [fw * 0.16, fh * 0.15],
                    ));
                    highlight = highlight.max(ellipse(uv, point(high), [fw * 0.17, fh * 0.1]));
                }
                contour = contour
                    .max(ellipse(uv, point(172), [fw * 0.2, fh * 0.1]))
                    .max(ellipse(uv, point(397), [fw * 0.2, fh * 0.1]));
                highlight = highlight
                    .max(ellipse(uv, point(6), [fw * 0.055, fh * 0.17]))
                    .max(ellipse(uv, point(151), [fw * 0.2, fh * 0.11]));
                result[6] = contour * skin;
                result[7] = highlight * skin;
                result
            })
            .collect();
        for (map, channel) in [
            (&mut seg.skin[..], 0),
            (&mut seg.eyes[..], 1),
            (&mut seg.teeth[..], 2),
            (&mut seg.under_eyes[..], 3),
            (&mut seg.forehead[..], 4),
            (&mut seg.laugh_lines[..], 5),
            (&mut seg.contour[..], 6),
            (&mut seg.highlight[..], 7),
        ] {
            map.par_chunks_mut(w as usize)
                .enumerate()
                .skip(y0 as usize)
                .take(y1.saturating_sub(y0) as usize)
                .for_each(|(y, row)| {
                    for x in x0..x1 {
                        let v =
                            face_masks[((y as u32 - y0) * (x1 - x0) + x - x0) as usize][channel];
                        row[x as usize] = row[x as usize].max(v);
                    }
                });
        }
    }
    seg
}

fn neural(
    image: &RgbaImage,
    seg: &mut Segmentation,
    provider: Provider,
    demand: PortraitDemand,
    status: &mut dyn FnMut(AiStatus),
) -> Result<()> {
    let n = (seg.width * seg.height) as usize;
    // User masks control the final blend. Model-space lesion repair always uses
    // the original face protection, as it does in a fresh full analysis.
    let repair_skin = if demand.blemishes && seg.custom_skin {
        masks(image, seg.faces.clone()).skin
    } else {
        seg.skin.clone()
    };
    if demand.skin {
        seg.neural_blend = vec![[0.5; 3]; n].into();
    }
    if demand.blemishes {
        seg.repair_delta = vec![[0.0; 3]; n].into();
        seg.blemish = vec![0.0; n].into();
    }
    for face in &seg.faces {
        let w = image.width() as f32;
        let h = image.height() as f32;
        let b = face.bounds;
        let side = (b[2] * w).max(b[3] * h) * 1.5;
        let (cx, cy) = ((b[0] + b[2] * 0.5) * w, (b[1] + b[3] * 0.5) * h);
        let (left, top) = ((cx - side * 0.5).max(0.0), (cy - side * 0.5).max(0.0));
        let (right, bottom) = ((cx + side * 0.5).min(w), (cy + side * 0.5).min(h));
        let rect = [
            (left + right) * 0.5,
            (top + bottom) * 0.5,
            right - left,
            bottom - top,
        ];
        let generator = if demand.skin {
            let output = run(
                "retouch_generator.onnx",
                provider,
                &[blob(image, 512, rect, 0.0, true)],
                status,
            )?;
            ensure!(
                output[0].0 == [1, 3, 512, 512],
                "Unexpected neural blend output"
            );
            Some(output)
        } else {
            None
        };
        let repair = if demand.blemishes {
            let detection = run(
                "local_detection.onnx",
                provider,
                &[blob(image, 768, rect, 0.0, true)],
                status,
            )?;
            ensure!(
                detection[0].0 == [1, 1, 768, 768],
                "Unexpected blemish detection output"
            );
            let lesion: Vec<f32> = detection[0]
                .1
                .iter()
                .map(|&v| {
                    let p = sigmoid(v);
                    if p >= 0.5 {
                        1.0
                    } else if p >= 0.35 {
                        p
                    } else {
                        0.0
                    }
                })
                .collect();
            let mut input = blob(image, 576, rect, 0.0, true);
            let mut valid = Array4::ones((1, 1, 576, 576));
            for y in 0..576 {
                for x in 0..576 {
                    let u = (x as f32 + 0.5) / 576.0;
                    let v = (y as f32 + 0.5) / 576.0;
                    let skin = sample_channel(
                        &repair_skin,
                        seg.width,
                        seg.height,
                        (left + u * rect[2]) / w,
                        (top + v * rect[3]) / h,
                    );
                    let missing = sample_channel(&lesion, 768, 768, u, v) * skin;
                    valid[[0, 0, y, x]] = 1.0 - missing;
                    for c in 0..3 {
                        input[[0, c, y, x]] *= 1.0 - missing;
                    }
                }
            }
            let inpaint = run("local_inpainting.onnx", provider, &[input, valid], status)?;
            ensure!(
                inpaint[0].0 == [1, 3, 576, 576],
                "Unexpected blemish inpainting output"
            );
            Some((lesion, inpaint))
        } else {
            None
        };
        for y in top.floor() as u32..bottom.ceil().min(h) as u32 {
            for x in left.floor() as u32..right.ceil().min(w) as u32 {
                let i = (y * seg.width + x) as usize;
                let u = (x as f32 + 0.5 - left) / rect[2];
                let v = (y as f32 + 0.5 - top) / rect[3];
                let border = (u.min(1.0 - u).min(v.min(1.0 - v)) / 0.045).clamp(0.0, 1.0);
                let missing = repair.as_ref().map(|(lesion, _)| {
                    sample_channel(lesion, 768, 768, u, v) * repair_skin[i] * border
                });
                if let Some(missing) = missing {
                    seg.blemish[i] = seg.blemish[i].max(missing);
                }
                for c in 0..3 {
                    if let Some(generator) = &generator {
                        let blend = sample_channel(
                            &generator[0].1[c * 512 * 512..(c + 1) * 512 * 512],
                            512,
                            512,
                            u,
                            v,
                        )
                        .clamp(0.0, 1.0);
                        seg.neural_blend[i][c] = 0.5 + (blend - 0.5) * border;
                    }
                    if let Some((_, inpaint)) = &repair {
                        let replacement = (sample_channel(
                            &inpaint[0].1[c * 576 * 576..(c + 1) * 576 * 576],
                            576,
                            576,
                            u,
                            v,
                        ) + 1.0)
                            * 0.5;
                        seg.repair_delta[i][c] = (replacement
                            - image.get_pixel(x, y)[c] as f32 / 255.0)
                            * missing.unwrap_or(0.0);
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn analyze_portrait(
    image: &RgbaImage,
    provider: Provider,
    stage: impl FnMut(Segmentation),
) -> Result<Segmentation> {
    analyze_portrait_with_status(image, provider, stage, |_| {})
}
pub fn analyze_portrait_with_status(
    image: &RgbaImage,
    provider: Provider,
    stage: impl FnMut(Segmentation),
    status: impl FnMut(AiStatus),
) -> Result<Segmentation> {
    analyze_portrait_demand_with_status(image, provider, PortraitDemand::ALL, None, stage, status)
}

/// Expand stored rectangles by exact native-pixel copies, preserving existing maps.
fn dense_maps(base: &Segmentation) -> Segmentation {
    let Some(crop) = base.map_crop else {
        return base.clone();
    };
    fn expand<T: Copy>(
        map: &crate::shared::SharedVec<T>,
        base: &Segmentation,
        crop: crate::engine::Crop,
        neutral: T,
    ) -> crate::shared::SharedVec<T> {
        let n = (base.width * base.height) as usize;
        if map.is_empty() || map.len() == n {
            return map.clone();
        }
        let mut data = vec![neutral; n];
        if map.len() == (crop.width * crop.height) as usize {
            for y in 0..crop.height {
                let from = (y * crop.width) as usize;
                let to = ((crop.y + y) * base.width + crop.x) as usize;
                data[to..to + crop.width as usize]
                    .copy_from_slice(&map[from..from + crop.width as usize]);
            }
        }
        data.into()
    }
    let mut seg = base.clone();
    for map in [
        &mut seg.skin,
        &mut seg.teeth,
        &mut seg.eyes,
        &mut seg.background,
        &mut seg.under_eyes,
        &mut seg.forehead,
        &mut seg.laugh_lines,
        &mut seg.contour,
        &mut seg.highlight,
        &mut seg.blemish,
    ] {
        *map = expand(map, base, crop, 0.0);
    }
    seg.neural_blend = expand(&seg.neural_blend, base, crop, [0.5; 3]);
    seg.repair_delta = expand(&seg.repair_delta, base, crop, [0.0; 3]);
    seg.map_crop = None;
    seg
}

pub fn analyze_portrait_demand_with_status(
    image: &RgbaImage,
    provider: Provider,
    demand: PortraitDemand,
    base: Option<&Segmentation>,
    mut stage: impl FnMut(Segmentation),
    mut status: impl FnMut(AiStatus),
) -> Result<Segmentation> {
    let demand = demand.normalized();
    if demand.is_empty() {
        return Ok(base.cloned().unwrap_or_default());
    }
    let base =
        base.filter(|seg| (seg.width, seg.height) == image.dimensions() && seg.prepared.geometry);
    if let Some(seg) = base.filter(|seg| seg.prepared.contains(demand)) {
        return Ok(seg.clone());
    }
    REQUESTED.with(|requested| {
        if *requested.borrow() != Some(provider) {
            release_idle_resources();
            *requested.borrow_mut() = Some(provider);
        }
    });
    USED.with(|used| used.borrow_mut().clear());
    FALLBACKS.with(|f| f.borrow_mut().clear());
    if image.width() == 0 || image.height() == 0 {
        bail!("Cannot analyze an empty image");
    }
    let mut seg = if let Some(base) = base {
        dense_maps(base)
    } else {
        let (w, h) = (image.width() as f32, image.height() as f32);
        let mut detections = detect(image, provider, [0.0, 0.0, w, h], &mut status)?;
        // Overlapping crops make short-range BlazeFace useful for smaller faces in group portraits.
        for y in [0.0, h * 0.4] {
            for x in [0.0, w * 0.4] {
                detections.extend(detect(
                    image,
                    provider,
                    [x, y, w * 0.6, h * 0.6],
                    &mut status,
                )?);
            }
        }
        let detections = nms(detections);
        let mut faces = vec![];
        for d in detections {
            if let Some(face) = mesh(image, &d, provider, &mut status)? {
                faces.push(face);
            }
        }
        let mut seg = masks(image, faces);
        seg.prepared = PortraitDemand::GEOMETRY;
        seg
    };
    if seg.faces.is_empty() {
        seg.prepared = PortraitDemand::ALL;
        seg.status = format!(
            "{} · no confident face found; use manual tools",
            backend_label(provider)
        );
        let fallback = FALLBACKS.with(|f| f.borrow().join("; "));
        if !fallback.is_empty() {
            seg.status.push_str(&format!(" · {fallback}"));
        }
        status(AiStatus::new(
            "Portrait analysis finished",
            &seg.status,
            None,
            1.0,
        ));
        seg.compact_generated_maps();
        return Ok(seg);
    }
    seg.status = format!(
        "{} · {} face(s), 478 landmarks · preparing neural retouch",
        backend_label(provider),
        seg.faces.len()
    );
    let missing = demand.without(seg.prepared);
    if missing.skin || missing.blemishes {
        let mut partial = seg.clone();
        partial.compact_generated_maps();
        stage(partial);
        neural(image, &mut seg, provider, missing, &mut status)?;
    }
    seg.prepared = seg.prepared.union(missing);
    seg.status = format!(
        "{} · {} face(s) · mesh{}{} ready",
        backend_label(provider),
        seg.faces.len(),
        if seg.prepared.skin {
            ", neural skin"
        } else {
            ""
        },
        if seg.prepared.blemishes {
            " & blemish repair"
        } else {
            ""
        },
    );
    let fallback = FALLBACKS.with(|f| f.borrow().join("; "));
    if !fallback.is_empty() {
        seg.status.push_str(&format!(" · {fallback}"));
    }
    status(AiStatus::new("Portrait AI ready", &seg.status, None, 1.0));
    seg.compact_generated_maps();
    Ok(seg)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qualification_rejects_wrong_shapes_nonfinite_and_large_output_changes() {
        let cpu = vec![(vec![1, 3], vec![0.2, -100.0, 250.0])];
        let close = vec![(vec![1, 3], vec![0.20001, -100.001, 250.001])];
        assert!(output_agreement(&cpu, &close));
        for output in [
            vec![(vec![3, 1], close[0].1.clone())],
            vec![(vec![1, 3], vec![0.2, f32::NAN, 250.0])],
            vec![(vec![1, 3], vec![0.4, -100.0, 250.0])],
        ] {
            assert!(!output_agreement(&cpu, &output));
        }
    }
    #[test]
    fn gpu_claim_requires_profile_kernel_provider_field() {
        assert!(profile_has_provider(
            r#"[{"args":{"provider" : "DmlExecutionProvider"}}]"#,
            "DmlExecutionProvider"
        ));
        assert!(!profile_has_provider(
            r#"[{"name":"DmlExecutionProvider","args":{"provider":"CPUExecutionProvider"}}]"#,
            "DmlExecutionProvider"
        ));
    }
    #[test]
    fn weighted_nms_preserves_multiple_faces_and_blends_duplicate_boxes() {
        let mut values = [0.0; 16];
        values[..4].copy_from_slice(&[20.0, 20.0, 10.0, 10.0]);
        let a = Detection { score: 0.9, values };
        let mut b = a.clone();
        b.score = 0.6;
        b.values[0] = 21.0;
        let mut c = a.clone();
        c.values[0] = 80.0;
        let out = nms(vec![a, b, c]);
        assert_eq!(out.len(), 2);
        assert!((out[0].values[0] - 20.4).abs() < 0.001);
    }
}
