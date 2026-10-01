"""Reproduce the verified local portrait models. No network access is used by the app."""
from pathlib import Path
import hashlib
import subprocess
import sys
import time
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
MODELS = ROOT / "models"
MODELS.mkdir(exist_ok=True)

def fetch(name, url, digest=None, expected_size=None):
    path = MODELS / name
    def valid():
        if not path.is_file(): return False
        if expected_size is not None and path.stat().st_size != expected_size: return False
        return digest is None or hashlib.file_digest(path.open("rb"), "sha256").hexdigest() == digest
    if valid(): return path
    for attempt in range(4):
        start = path.stat().st_size if path.exists() else 0
        req = urllib.request.Request(url, headers={"User-Agent": "Astra-Retouch/0.1", "Range": f"bytes={start}-"})
        try:
            with urllib.request.urlopen(req, timeout=120) as response:
                resume = response.status == 206 and response.headers.get("Content-Range", "").startswith(f"bytes {start}-")
                with path.open("ab" if resume else "wb") as out:
                    while data := response.read(1024 * 1024): out.write(data)
            if valid():
                print(f"Verified {name}", flush=True)
                return path
            # A partial transfer can resume; a complete but incorrect file must restart.
            if expected_size is None or path.stat().st_size >= expected_size: path.unlink(missing_ok=True)
        except Exception as error:
            print(f"Retrying {name}: {error}", flush=True)
        time.sleep(1 + attempt)
    raise RuntimeError(f"Could not verify {name}")

def main():
    mesh = "https://github.com/yakhyo/mediapipe-face-mesh-onnx/releases/download/weights/"
    fetch("face_detection_short_range.onnx", mesh + "face_detection_short_range.onnx", "2f2689b040becf555706d2cb978d2f0e3296ea82413734fba9a856c66c5f2b17", 470549)
    fetch("face_landmarker_Nx3x256x256.onnx", mesh + "face_landmarker_Nx3x256x256.onnx", "111795f8703cdeb6d0c68a9f3cc966a0f23f8786bb00f4577a11f461fc4276ac", 4864717)
    source = "https://media.githubusercontent.com/media/aoguai/skin-retouching-onnxruntime/master/"
    fetch("pytorch_model.pt", source + "pytorch_model.pt", "760d0c0b95c4e0a692165677fe2b82dbdf8fbbd4a3780ca966599c627b082993", 53646276)
    fetch("joint_20210926.pth", source + "joint_20210926.pth", "c9e1388b19d9334c90db08308c86de208f535920ee381ae8a9cb63484a78351a", 227618450)
    runtime = fetch("onnxruntime-win-x64-1.24.4.zip", "https://github.com/microsoft/onnxruntime/releases/download/v1.24.4/onnxruntime-win-x64-1.24.4.zip", "d2319fddfb6ea4db99ccc4b60c85c517bcd855721f5daa6a06d40d7cb2ee2357", 74442783)
    with zipfile.ZipFile(runtime) as archive:
        for name in archive.namelist():
            if Path(name).name in {"onnxruntime.dll", "onnxruntime_providers_shared.dll", "LICENSE", "ThirdPartyNotices.txt"}:
                (MODELS / Path(name).name).write_bytes(archive.read(name))
    venv = ROOT / ".ai-tools"
    python = venv / "Scripts" / "python.exe"
    if not python.exists(): subprocess.run([sys.executable, "-m", "venv", str(venv)], check=True)
    subprocess.run([str(python), "-m", "pip", "install", "torch==2.8.0", "--index-url", "https://download.pytorch.org/whl/cpu"], check=True)
    subprocess.run([str(python), "-m", "pip", "install", "onnx==1.19.0", "numpy", "pillow"], check=True)
    subprocess.run([str(python), str(ROOT / "scripts/third_party/export_skin_retouching_onnx.py"), "--model-dir", str(MODELS), "--skip-face", "--opset", "11"], check=True)
    print("AI models ready. The next Cargo build generates Burn model packs; ONNX Runtime remains available for the CPU and GPU providers.")

if __name__ == "__main__": main()
