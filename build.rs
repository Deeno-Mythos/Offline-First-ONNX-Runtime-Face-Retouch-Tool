#[cfg(feature = "onnx")]
use std::{env, fs, path::PathBuf};

#[cfg(feature = "onnx")]
const MODELS: [&str; 5] = [
    "face_detection_short_range",
    "face_landmarker_Nx3x256x256",
    "retouch_generator",
    "local_detection",
    "local_inpainting",
];

fn main() {
    println!("cargo:rustc-check-cfg=cfg(astra_burn_models)");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_ONNX");

    #[cfg(feature = "onnx")]
    generate_models();
}

#[cfg(feature = "onnx")]
fn generate_models() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let model_dir = root.join("models");
    for model in MODELS {
        let path = model_dir.join(format!("{model}.onnx"));
        println!("cargo:rerun-if-changed={}", path.display());
        if !path.is_file() {
            println!(
                "cargo:warning=Burn model conversion skipped because {} is missing; CPU inference will use ONNX Runtime until scripts/setup-ai.ps1 is run.",
                path.display()
            );
            return;
        }
    }

    let mut model_gen = burn_onnx::ModelGen::new();
    for model in MODELS {
        model_gen.input(model_dir.join(format!("{model}.onnx")).to_str().unwrap());
    }
    model_gen
        .out_dir("astra_burn")
        .load_strategy(burn_onnx::LoadStrategy::File)
        .run_from_script();

    // Burn's File loading strategy keeps large weights out of the executable. Ship the
    // generated packs beside the source models, where the app already looks for AI assets.
    let generated_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("astra_burn");
    let runtime_dir = model_dir.join("burn");
    fs::create_dir_all(&runtime_dir).expect("create Burn model directory");
    for model in MODELS {
        let pack = format!("{model}.bpk");
        fs::copy(generated_dir.join(&pack), runtime_dir.join(pack))
            .expect("copy generated Burn model pack");
    }

    println!("cargo:rustc-cfg=astra_burn_models");
}
