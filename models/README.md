# Local portrait models

The app uses real pretrained ONNX weights. It never downloads models or uploads photos at startup. `scripts/setup-ai.ps1` reproduces the verified CPU and Windows DirectML setup, including checksummed downloads and the isolated conversion environment. `scripts/setup-ai-runtime.ps1` installs only the DirectML runtime without converting models again. ONNX is enabled by default; manual editing is also available in builds made with `--no-default-features`.

The default `Auto` provider runs inference through ONNX Runtime. It qualifies each bundled model's first real GPU output against CPU and verifies GPU kernel events before using the GPU; missing providers, errors or output disagreement select a visible CPU fallback. Explicit CPU, DirectML, CUDA and CoreML remain available. When all five ONNX files are present during a build, `burn-onnx` also converts them into generated Rust models and writes `.bpk` weights under `models/burn/`. The `Burn CPU` provider runs those bundled portrait models with Burn's CubeCL CPU backend and remains opt-in. Custom imported ONNX mask models use CPU in Auto mode and can also use an explicitly selected ONNX provider. Burn's generated code and weights are local build artifacts and are not checked into source control.

| Model | Source and license | Contract |
| --- | --- | --- |
| `face_detection_short_range.onnx` | [Yakhyo MediaPipe conversion](https://github.com/yakhyo/mediapipe-face-mesh-onnx), Google MediaPipe architecture/weights, Apache 2.0 | RGB `[1,3,128,128]` in `[-1,1]`; 896 box regressors and score logits. Letterboxing, weighted NMS, overlapping detection crops. |
| `face_landmarker_Nx3x256x256.onnx` | Same source, Apache 2.0 | Rotated square RGB crops `[1,3,256,256]` in `[0,1]`; 478 crop-pixel XYZ landmarks and presence logit. ROI side is 1.5 times the larger box dimension. |
| `retouch_generator.onnx` | [Alibaba / ModelScope skin retouching](https://www.modelscope.cn/models/iic/cv_unet_skin_retouching_torch), Apache 2.0; locally fine-tuned from the original checkpoint | RGB `[1,3,512,512]` in `[-1,1]`; learned RGB blend layer, applied as `(1−2m)I²+2mI` with neutral blend 0.5. The 195-parameter output head was adapted on DAY 2 paired studio portraits after the earlier single-photo adaptation, retaining 80% of the learned update for preservation. Whole-face held-out local skin MAE improved 6.48%; under-eye MAE improved 3.72%. Split: 26 train / 6 validation / 6 test subjects. This is a small studio dataset result, not a general benchmark; the earlier synthetic benchmark was 1.90% worse within its 3% preservation limit. See [paired training](../docs/paired-training.md). |
| `local_detection.onnx` | Same Alibaba pretrained joint checkpoint, Apache 2.0 | RGB `[1,3,768,768]` in `[-1,1]`; blemish mask logits, sigmoid and 0.35/0.5 confidence thresholds. |
| `local_inpainting.onnx` | Same checkpoint, Apache 2.0 | Masked RGB `[1,3,576,576]` in `[-1,1]` and validity `[1,1,576,576]`; normalized repaired RGB. Skin/feature protection restricts repaired regions. |

The retouch and local models were converted from verified checkpoints using the architecture/export code from [aoguai/skin-retouching-onnxruntime](https://github.com/aoguai/skin-retouching-onnxruntime). That Apache-licensed exporter is preserved in `scripts/third_party/export_skin_retouching_onnx.py`; original copyright headers are retained. See `LICENSE-modelscope.txt` and `LICENSE-mediapipe.txt`.

`scripts/train_personalized_retouch.py` writes its gated ONNX candidate and validation artifacts under `output/model-training/personalized-retouch/`; it does not replace the active ONNX or Burn assets automatically.

`scripts/train_paired_retouch.py` prepares registered, protected local skin targets and validates a candidate against both the active model and untouched photos. Its explicit `promote` phase verifies quality gates/checksums, retains the prior active ONNX/Burn pack/checkpoint, and installs the new ONNX plus training checkpoint. A Cargo build regenerates the Burn pack. Generated `retouch-training.json` records hashes and aggregate results; setup preserves verified trained weights rather than replacing them with the pretrained generator. `models/retouch_generator.pt` is the local active training checkpoint; both it and the record are excluded from source control. The prior one-photo fine-tune remains archived: 64 synthetic-pair steps, 1.90% held-out synthetic repair improvement, clean-output shift 0.00059.

The engine preserves source-resolution texture: it registers model blend layers and repair deltas to the portrait rather than replacing the entire face with a small neural output. Eye, eyebrow and lip contours are excluded from skin smoothing. Teeth are gated by mouth geometry and pixel color; a mouth cavity alone does not indicate teeth. Fine hairs and complex scars can require manual healing or clone cleanup.

ONNX Runtime 1.24.4 uses the v24 C API. Its MIT license and third-party notices are included as `LICENSE` and `ThirdPartyNotices.txt`; the separate `models/directml/` directory also includes DirectML redistribution licenses. The earlier Intel UHD no-face failure was traced to vendor metacommands. Hastur disables those metacommands while retaining real DirectML GPU kernels. All five models now execute on this computer's GPU, and native retouch output agrees with CPU within one byte on the demo and supplied red-dress portrait. See [hardware qualification and measured performance](../docs/hardware-acceleration.md). Provider choices require a compatible runtime and working driver; completed status identifies the provider actually exercised, and cached status retains its source provider.

Automatic neural maps are written to `.hastur-cache/` beside `models/`. Existing `.astra-cache/` files remain readable as a fallback, including when the newer cache is missing or unreadable. The rename does not move, delete, or prune legacy cache files. The key includes image pixels, dimensions, model metadata and a pipeline version; existing format identifiers remain compatible. Invalid caches are ignored. Cached output is labeled as cached and retains its original provider label. New cache storage is capped at 512 MiB; retained legacy files are separate from that limit. Reanalyze portrait bypasses the cache. Sessions retain sliders, local tools and face metadata; generated neural maps are reloaded from cache or recomputed.

Verified SHA-256 values:

```text
face_detection_short_range.onnx  2f2689b040becf555706d2cb978d2f0e3296ea82413734fba9a856c66c5f2b17
face_landmarker_Nx3x256x256.onnx  111795f8703cdeb6d0c68a9f3cc966a0f23f8786bb00f4577a11f461fc4276ac
pytorch_model.pt                760d0c0b95c4e0a692165677fe2b82dbdf8fbbd4a3780ca966599c627b082993
joint_20210926.pth               c9e1388b19d9334c90db08308c86de208f535920ee381ae8a9cb63484a78351a
retouch_generator.onnx           348da8ed8114d0951cfb1e5026fd6ed4c70a087e92ab8a7421723b96b666cb40
local_detection.onnx             52185c535326a4feb432db5c715e512fae6198f244d9aa7756652af62c46d8dc
local_inpainting.onnx            fccbae44ae5c4487b164d4d511a2922f9177bba55b54ec7c983c30b35e410ea2
onnxruntime-win-x64-1.24.4.zip    d2319fddfb6ea4db99ccc4b60c85c517bcd855721f5daa6a06d40d7cb2ee2357
microsoft.ml.onnxruntime.directml.1.24.4.nupkg  57e9f11b73437bef7a309496135d4c1f96b1a8e9ddba60013fa27bfc1d788681
microsoft.ai.directml.1.15.4.nupkg  4e7cb7ddce8cf837a7a75dc029209b520ca0101470fcdf275c1f49736a3615b9
```

Converted ONNX bytes can vary with exporter/toolchain versions; checkpoint hashes are the reproducible source identity. The currently verified conversion used PyTorch 2.8.0 CPU, ONNX 1.19.0 and opset 11 on Windows.
