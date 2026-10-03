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
    embed_windows_icon();

    #[cfg(feature = "onnx")]
    generate_models();
}

/// Embed the crown in the PE resource table as well as the runtime window icon.
/// This uses the SDK already required by native Windows builds, without adding a
/// resource crate or tying the output to a package/binary name.
fn embed_windows_icon() {
    use std::{env, path::PathBuf, process::Command};

    println!("cargo:rerun-if-changed=assets/branding/icon.rc");
    println!("cargo:rerun-if-changed=assets/branding/hastur.ico");
    for key in [
        "RC",
        "WINDRES",
        "PATH",
        "WindowsSdkDir",
        "WindowsSdkBinPath",
        "WindowsSdkVerBinPath",
        "WindowsSDKVersion",
        "ProgramFiles(x86)",
        "ProgramFiles",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let script = root.join("assets/branding/icon.rc");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let resource = out.join(if msvc {
        "hastur-icon.res"
    } else {
        "hastur-icon.o"
    });
    let mut command = if msvc {
        let compiler = find_resource_compiler().expect(
            "Hastur Retouch requires Windows SDK rc.exe to embed its icon; install the Windows SDK or set RC to the resource compiler path",
        );
        let mut command = Command::new(compiler);
        command
            .arg("/nologo")
            .arg("/fo")
            .arg(&resource)
            .arg(&script);
        command
    } else {
        let compiler = env::var_os("WINDRES").unwrap_or_else(|| {
            let prefix = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
                Ok("x86") => "i686",
                Ok("aarch64") => "aarch64",
                _ => "x86_64",
            };
            let named = format!("{prefix}-w64-mingw32-windres");
            if find_on_path(&named).is_some() {
                named.into()
            } else {
                "windres".into()
            }
        });
        let mut command = Command::new(compiler);
        command
            .arg("--input")
            .arg(&script)
            .arg("--output")
            .arg(&resource)
            .arg("--output-format=coff");
        command
    };
    let result = command
        .current_dir(&root)
        .output()
        .expect("start Windows icon resource compiler");
    assert!(
        result.status.success(),
        "Windows icon resource compilation failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        resource.is_file(),
        "Windows resource compiler did not produce the icon resource"
    );
    println!("cargo:rustc-link-arg-bins={}", resource.display());
}

fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|directory| {
            let path = directory.join(name);
            if path.is_file() {
                return Some(path);
            }
            let exe = directory.join(format!("{name}.exe"));
            exe.is_file().then_some(exe)
        })
    })
}

fn find_resource_compiler() -> Option<std::path::PathBuf> {
    use std::{env, fs, path::PathBuf, process::Command};

    if let Some(explicit) = env::var_os("RC") {
        return Some(PathBuf::from(explicit));
    }
    if let Some(path) = find_on_path("rc").or_else(|| find_on_path("llvm-rc")) {
        return Some(path);
    }
    let host_arch = match env::var("HOST").unwrap_or_default().split('-').next() {
        Some("aarch64") => "arm64",
        Some("i686") => "x86",
        _ => "x64",
    };
    let mut roots = Vec::new();
    for key in ["WindowsSdkVerBinPath", "WindowsSdkBinPath"] {
        if let Some(path) = env::var_os(key) {
            roots.push(PathBuf::from(path));
        }
    }
    if let Some(path) = env::var_os("WindowsSdkDir") {
        roots.push(PathBuf::from(path).join("bin"));
    }
    for key in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(path) = env::var_os(key) {
            roots.push(PathBuf::from(path).join("Windows Kits/10/bin"));
        }
    }
    // Discover a Windows SDK installed outside Program Files as well.
    if let Ok(result) = Command::new("reg.exe")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Windows Kits\Installed Roots",
            "/v",
            "KitsRoot10",
        ])
        .output()
    {
        let text = String::from_utf8_lossy(&result.stdout);
        if let Some(value) = text
            .lines()
            .find_map(|line| line.split_once("REG_SZ").map(|(_, value)| value.trim()))
        {
            roots.push(PathBuf::from(value).join("bin"));
        }
    }
    for root in roots {
        for path in [root.join(host_arch).join("rc.exe"), root.join("rc.exe")] {
            if path.is_file() {
                return Some(path);
            }
        }
        if let Ok(entries) = fs::read_dir(&root) {
            let mut versions: Vec<_> = entries
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let version = name
                        .split('.')
                        .map(str::parse::<u32>)
                        .collect::<Result<Vec<_>, _>>()
                        .ok()?;
                    Some((version, entry.path()))
                })
                .collect();
            versions.sort_by(|a, b| b.0.cmp(&a.0));
            for (_, directory) in versions {
                let path = directory.join(host_arch).join("rc.exe");
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
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
