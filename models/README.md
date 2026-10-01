# Local portrait models

The app uses real pretrained ONNX weights. It never downloads models or uploads photos at startup. `scripts/setup-ai.ps1` reproduces the verified CPU setup, including checksummed downloads and the isolated conversion environment. ONNX is enabled by default; manual editing is also available in builds made with `--no-default-features`.

The default `CPU` provider runs inference through ONNX Runtime. When all five ONNX files are present during a build, `burn-onnx` also converts them into generated Rust models and writes `.bpk` weights under `models/burn/`. The `Burn CPU` provider runs those bundled portrait models with Burn's CubeCL CPU backend. It is experimental and opt-in: on the current reference machine, the bundled local blemish detector took about 104 seconds in a debug build versus 0.82 seconds through ONNX Runtime. Custom imported ONNX mask models continue to use ONNX Runtime. Burn's generated code and weights are local build artifacts and are not checked into source control.

| Model | Source and license | Contract |
| --- | --- | --- |
| `face_detection_short_range.onnx` | [Yakhyo MediaPipe conversion](https://github.com/yakhyo/mediapipe-face-mesh-onnx), Google MediaPipe architecture/weights, Apache 2.0 | RGB `[1,3,128,128]` in `[-1,1]`; 896 box regressors and score logits. Letterboxing, weighted NMS, overlapping detection crops. |
| `face_landmarker_Nx3x256x256.onnx` | Same source, Apache 2.0 | Rotated square RGB crops `[1,3,256,256]` in `[0,1]`; 478 crop-pixel XYZ landmarks and presence logit. ROI side is 1.5 times the larger box dimension. |
| `retouch_generator.onnx` | [Alibaba / ModelScope skin retouching](https://www.modelscope.cn/models/iic/cv_unet_skin_retouching_torch), Apache 2.0; locally fine-tuned from the original checkpoint | RGB `[1,3,512,512]` in `[-1,1]`; learned RGB blend layer. Applied as `(1−2m)I²+2mI`, with a neutral blend of 0.5. The local output head was fine-tuned for 64 steps on synthetic under-eye and blemish pairs from one portrait. Held-out synthetic repair MAE improved 1.90%; clean-output MAE shift was 0.00059. This is a single-image personalization result, not a general training benchmark. |
| `local_detection.onnx` | Same Alibaba pretrained joint checkpoint, Apache 2.0 | RGB `[1,3,768,768]` in `[-1,1]`; blemish mask logits, sigmoid and 0.35/0.5 confidence thresholds. |
| `local_inpainting.onnx` | Same checkpoint, Apache 2.0 | Masked RGB `[1,3,576,576]` in `[-1,1]` and validity `[1,1,576,576]`; normalized repaired RGB. Skin/feature protection restricts repaired regions. |

The retouch and local models were converted from verified checkpoints using the architecture/export code from [aoguai/skin-retouching-onnxruntime](https://github.com/aoguai/skin-retouching-onnxruntime). That Apache-licensed exporter is preserved in `scripts/third_party/export_skin_retouching_onnx.py`; original copyright headers are retained. See `LICENSE-modelscope.txt` and `LICENSE-mediapipe.txt`.

`scripts/train_personalized_retouch.py` writes its gated ONNX candidate and validation artifacts under `output/model-training/personalized-retouch/`; it does not replace the active ONNX or Burn assets automatically.

The engine preserves source-resolution texture: it registers model blend layers and repair deltas to the portrait rather than replacing the entire face with a small neural output. Eye, eyebrow and lip contours are excluded from skin smoothing. Teeth are gated by mouth geometry and pixel color; a mouth cavity alone does not indicate teeth. Fine hairs and complex scars can require manual healing or clone cleanup.

ONNX Runtime 1.24.4 uses the v24 C API. Its MIT license and third-party notices are included as `LICENSE` and `ThirdPartyNotices.txt`. CPU is the validated default. An optional DirectML runtime was tested with this computer's Intel UHD driver: it returned no confident face on the known demo, so it is not selected automatically. GPU provider choices require a compatible runtime and working driver. Changing the provider never changes the name of the actual provider reported by a completed inference.

Automatic neural maps are cached in `.astra-cache/` beside `models/`. The key includes image pixels, dimensions, model metadata and a pipeline version. Invalid caches are ignored. Cached output is labeled as cached and retains its original provider label. The cache is capped at 512 MiB. Reanalyze portrait bypasses it. Sessions retain sliders, local tools and face metadata; generated neural maps are reloaded from cache or recomputed.

Verified SHA-256 values:

```text
face_detection_short_range.onnx  2f2689b040becf555706d2cb978d2f0e3296ea82413734fba9a856c66c5f2b17
face_landmarker_Nx3x256x256.onnx  111795f8703cdeb6d0c68a9f3cc966a0f23f8786bb00f4577a11f461fc4276ac
pytorch_model.pt                760d0c0b95c4e0a692165677fe2b82dbdf8fbbd4a3780ca966599c627b082993
joint_20210926.pth               c9e1388b19d9334c90db08308c86de208f535920ee381ae8a9cb63484a78351a
retouch_generator.onnx           28d5790177a7e4f1fb70fb3ac3773d14112d8a62104cca11b9d425728a37bea4
local_detection.onnx             52185c535326a4feb432db5c715e512fae6198f244d9aa7756652af62c46d8dc
local_inpainting.onnx            fccbae44ae5c4487b164d4d511a2922f9177bba55b54ec7c983c30b35e410ea2
onnxruntime-win-x64-1.24.4.zip    d2319fddfb6ea4db99ccc4b60c85c517bcd855721f5daa6a06d40d7cb2ee2357
```

Converted ONNX bytes can vary with exporter/toolchain versions; checkpoint hashes are the reproducible source identity. The currently verified conversion used PyTorch 2.8.0 CPU, ONNX 1.19.0 and opset 11 on Windows.
