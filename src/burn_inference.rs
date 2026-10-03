//! Burn CPU inference adapters for the bundled portrait models.
//!
//! The ONNX graphs are converted to Rust during the build by `burn-onnx`; the
//! generated modules load Burnpack weights from `models/burn/` at runtime.

use anyhow::{Context, Result, bail, ensure};
use burn::tensor::{Device, Tensor, TensorData};
use ndarray::Array4;
use std::{cell::RefCell, collections::HashMap, path::PathBuf};

#[allow(dead_code)]
mod face_detection {
    include!(concat!(
        env!("OUT_DIR"),
        "/astra_burn/face_detection_short_range.rs"
    ));
}
#[allow(dead_code)]
mod face_landmarker {
    include!(concat!(
        env!("OUT_DIR"),
        "/astra_burn/face_landmarker_Nx3x256x256.rs"
    ));
}
#[allow(dead_code)]
mod retouch_generator {
    include!(concat!(env!("OUT_DIR"), "/astra_burn/retouch_generator.rs"));
}
#[allow(dead_code)]
mod local_detection {
    include!(concat!(env!("OUT_DIR"), "/astra_burn/local_detection.rs"));
}
#[allow(dead_code)]
mod local_inpainting {
    include!(concat!(env!("OUT_DIR"), "/astra_burn/local_inpainting.rs"));
}

type Output = (Vec<i64>, Vec<f32>);
type ModelKey = (PathBuf, u64, std::time::SystemTime);

enum Model {
    FaceDetection(Box<face_detection::Model>),
    FaceLandmarker(Box<face_landmarker::Model>),
    RetouchGenerator(Box<retouch_generator::Model>),
    LocalDetection(Box<local_detection::Model>),
    LocalInpainting(Box<local_inpainting::Model>),
}

thread_local! {
    // The app's AI work is serialized per worker. Thread-local model caches avoid
    // re-reading 270+ MB of weights for each tile without cross-thread lock contention.
    static MODELS: RefCell<HashMap<ModelKey, Model>> = RefCell::new(HashMap::new());
}

pub(crate) fn release_idle_resources() {
    MODELS.with(|cache| cache.borrow_mut().clear());
}

pub(crate) fn resident_model_count() -> usize {
    MODELS.with(|cache| cache.borrow().len())
}

fn model_path(name: &str) -> PathBuf {
    let stem = name.strip_suffix(".onnx").unwrap_or(name);
    crate::model::model_path(&format!("burn/{stem}.bpk"))
}

fn load_model(name: &str, path: &PathBuf) -> Result<Model> {
    let device = Device::cpu();
    let model = match name {
        "face_detection_short_range.onnx" => {
            Model::FaceDetection(Box::new(face_detection::Model::from_file(path, &device)))
        }
        "face_landmarker_Nx3x256x256.onnx" => {
            Model::FaceLandmarker(Box::new(face_landmarker::Model::from_file(path, &device)))
        }
        "retouch_generator.onnx" => {
            Model::RetouchGenerator(Box::new(retouch_generator::Model::from_file(path, &device)))
        }
        "local_detection.onnx" => {
            Model::LocalDetection(Box::new(local_detection::Model::from_file(path, &device)))
        }
        "local_inpainting.onnx" => {
            Model::LocalInpainting(Box::new(local_inpainting::Model::from_file(path, &device)))
        }
        _ => bail!("No bundled Burn model for {name}"),
    };
    Ok(model)
}

fn input_tensor(input: &Array4<f32>) -> Result<Tensor<4>> {
    let shape = input.shape();
    let data = input
        .as_slice()
        .context("Burn input array is not contiguous")?
        .to_vec();
    Ok(Tensor::from_data(
        TensorData::new(data, [shape[0], shape[1], shape[2], shape[3]]),
        &Device::cpu(),
    ))
}

fn output<const D: usize>(tensor: Tensor<D>) -> Result<Output> {
    let data = tensor.into_data();
    let shape = data.shape.dims::<D>().iter().map(|&v| v as i64).collect();
    let values = data.try_into_vec_as::<f32>()?;
    ensure!(
        values.iter().all(|v| v.is_finite()),
        "Burn produced non-finite output"
    );
    Ok((shape, values))
}

pub(crate) fn run(name: &str, inputs: &[Array4<f32>]) -> Result<Vec<Output>> {
    ensure!(
        matches!(inputs.len(), 1 | 2),
        "Unexpected input count for {name}"
    );

    let start = std::time::Instant::now();
    let path = model_path(name);
    let metadata = std::fs::metadata(&path).with_context(|| {
        format!(
            "Burn model pack {} is missing. Run scripts/setup-ai.ps1.",
            path.display()
        )
    })?;
    let key = (path, metadata.len(), metadata.modified()?);
    let result = MODELS.with(|cache| -> Result<Vec<Output>> {
        let mut cache = cache.borrow_mut();
        cache.retain(|(path, len, modified), _| {
            path != &key.0 || (*len, *modified) == (key.1, key.2)
        });
        if !cache.contains_key(&key) {
            cache.insert(key.clone(), load_model(name, &key.0)?);
        }
        let model = cache.get(&key).expect("Burn model inserted");
        let image = input_tensor(&inputs[0])?;
        let outputs = match model {
            Model::FaceDetection(model) => {
                let (boxes, scores) = model.forward(image);
                vec![output(boxes)?, output(scores)?]
            }
            Model::FaceLandmarker(model) => {
                let (landmarks, presence) = model.forward(image);
                vec![output(landmarks)?, output(presence)?]
            }
            Model::RetouchGenerator(model) => vec![output(model.forward(image))?],
            Model::LocalDetection(model) => vec![output(model.forward(image))?],
            Model::LocalInpainting(model) => {
                let mask = input_tensor(&inputs[1])?;
                vec![output(model.forward(image, mask))?]
            }
        };
        Ok(outputs)
    })?;
    if std::env::var_os("HASTUR_AI_TIMINGS").is_some()
        || std::env::var_os("ASTRA_AI_TIMINGS").is_some()
    {
        eprintln!(
            "Burn {name}: {:.2} ms",
            start.elapsed().as_secs_f64() * 1000.
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burn_generated_face_models_run_and_return_finite_outputs() {
        let detection = run(
            "face_detection_short_range.onnx",
            &[Array4::zeros((1, 3, 128, 128))],
        )
        .unwrap();
        assert_eq!(detection.len(), 2);
        assert_eq!(detection[0].1.len(), 896 * 16);
        assert_eq!(detection[1].1.len(), 896);

        let landmarks = run(
            "face_landmarker_Nx3x256x256.onnx",
            &[Array4::zeros((1, 3, 256, 256))],
        )
        .unwrap();
        assert_eq!(landmarks.len(), 2);
        assert_eq!(landmarks[0].1.len(), 478 * 3);
        assert!(!landmarks[1].1.is_empty());
        assert!(
            detection
                .iter()
                .chain(landmarks.iter())
                .flat_map(|output| output.1.iter())
                .all(|value| value.is_finite())
        );
    }

    #[test]
    fn burn_generated_retouch_model_runs_and_returns_finite_output() {
        let retouch = run("retouch_generator.onnx", &[Array4::zeros((1, 3, 512, 512))]).unwrap();
        assert_eq!(retouch.len(), 1);
        assert_eq!(retouch[0].0, [1, 3, 512, 512]);
        assert!(retouch[0].1.iter().all(|value| value.is_finite()));
    }
}
