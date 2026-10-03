# Local paired portrait training

`scripts/train_paired_retouch.py` fine-tunes the existing skin retouch generator on matched original/edited JPEGs. This is offline PyTorch training, followed by ONNX export. The app still uses its normal ONNX Runtime or Burn inference path and NullState demand loading. Training does not run in the UI, load models on neutral photos, or add another network to inference.

## Dataset and preparation

The DAY 2 dataset contains 38 subjects, each with one original JPEG and one corresponding JPEG inside `EDITED`. The originals are unretouched JPEGs rather than camera RAW files. Originals use EXIF rotation; edited photos have upright orientation and different crops. All source files are read only. SHA-256 hashes detect accidental changes; ambiguous filenames, extra candidates and duplicate originals cause an error.

An anonymous numeric folder ID identifies each subject in reports. A seeded split reserves 26 subjects for training, 6 for validation/checkpoint selection and 6 for the final test. Crops/pixels from a person remain in the same split. The manifest with local source paths and all photo derivatives stays under gitignored `output/model-training/day-2/`.

`examples/training_faces.rs` imports both photos through the actual EXIF/ICC/sRGB app pipeline and runs only face geometry. The generator receives the same 512-pixel source grid used in production. Facial landmarks initialize similarity alignment; forward/backward dense flow registers local shape changes and rejects inconsistent pixels. Training targets contain local skin corrections from the registered edited image. Robust monotone RGB fitting and removal of broad residual lighting separate the edited grade from local cleanup. The runtime skin mask and additional boundary erosion protect hair, eyes, brows and lips. This target definition deliberately excludes face/body reshaping, background edits, global contrast and global color grading.

## Training and acceptance

The starting checkpoint must numerically reproduce the active ONNX model before training. The encoder/decoder stay frozen; the 195-parameter final RGB blend head learns on stratified skin, under-eye and protected pixels. Inverse mixture-probability weighting corrects region oversampling, so extra under-eye samples do not silently bias the overall skin objective. Cached features make repeated optimization inexpensive. Validation selects the checkpoint. Final accuracy uses full face maps averaged by subject, independently of the sampled training pixels.

Acceptance requires at least 3% improvement over the current model for held-out skin and under-eye MAE, a better result than leaving the photo unretouched on both measures, bounded protected output drift and skin luminance bias, and ONNX/PyTorch output agreement. These metrics use aligned local-skin targets, not literal reproduction of the whole edited JPEG. The candidate is always exported for review, but a failed candidate never replaces active runtime assets. Visual checks and real app rendering remain necessary before adopting a numerically successful candidate.

This adapts the skin/under-eye blend model. It does not retrain the face detector, landmarks, independent blemish detector/inpainting models, or manual tools. One studio's 38 photos do not establish performance across other cameras, lighting, identities or editing styles.

## Reproduction

Use the existing `.ai-tools` Python environment or install the pinned offline dependencies in `scripts/requirements-paired-training.txt`. PyTorch 2.8.0 CPU is available from its official CPU wheel index. The original model architecture remains in the Apache-licensed exporter.

```powershell
$dataset = 'C:\Users\subop\OneDrive\Desktop\DAY 2'
$trainingOut = 'output/model-training/day-2-next'
.\.ai-tools\Scripts\python.exe scripts/train_paired_retouch.py --dataset $dataset --output $trainingOut --phase inventory
cargo run --release --locked --example training_faces -- $dataset "$trainingOut/faces"
.\.ai-tools\Scripts\python.exe scripts/train_paired_retouch.py --dataset $dataset --output $trainingOut --phase prepare
.\.ai-tools\Scripts\python.exe scripts/train_paired_retouch.py --dataset $dataset --output $trainingOut --phase train --steps 5000 --learning-rate 0.0005
.\.ai-tools\Scripts\python.exe scripts/test_paired_retouch.py
```

The default training checkpoint is `models/retouch_generator.pt` when installed, otherwise the previously validated personalized head in `output/model-training/personalized-retouch/retouch_generator_head_only.pt`. Supply `--checkpoint PATH` for another active checkpoint. Do not use a checkpoint whose output differs from the current model. Use a fresh `--output DIRECTORY` for a different dataset/seed/model/target definition. `features/provenance.json` guards cache reuse against the complete checkpoint, architecture and prepared-data hashes. Source files are hashed again for preparation/training.

Outputs include the pairing manifest, preparation metrics, protected masks/target comparisons, learning curve, per-person validation/test errors, candidate PyTorch/ONNX files, and held-out before/target/current/candidate review images. The candidate uses the same `[1,3,512,512]` input/output contract and opset 11. `--phase promote` requires all recorded quality gates and model checksums to pass, preserves the prior ONNX/Burn/checkpoint and installs the validated candidate. Adopting it also requires regenerating the Burn pack with Cargo and validating both providers. Setup checks the installed training record and preserves verified learned weights.

## DAY 2 result — 2026-10-02

The final run optimized for 5,000 steps at learning rate 0.0005 and selected step 3,925 on validation. Preliminary 500/3,000-step trials were superseded; the longer run and sampling correction were based on validation behavior. The test split was not used for checkpoint selection. An additional regression check on the earlier portrait's synthetic defects found 3.23% deterioration with the complete head update. Conservative parameter interpolation retains 80% of the learned update without loading/running two models; that weight was chosen for the earlier portrait's preservation check. Whole-face metrics below are averages of the six test subjects, not training-pixel or training-subject results. All six improved in both skin and under-eye error, with individual under-eye improvement ranging from 0.17% to 11.36%.

| Held-out local-target MAE, RGB in 0–1 | Previous active model | DAY 2 model | Improvement |
| --- | ---: | ---: | ---: |
| Skin | 0.00485363 | 0.00453904 | 6.48% |
| Under-eyes | 0.00855062 | 0.00823271 | 3.72% |

Leaving the photos untouched scored 0.00488718 skin / 0.00939827 under-eye MAE. Protected raw-network output drift averaged 0.00031622; runtime masks prevent the skin generator from editing those protected pixels. Mean skin luminance bias was +0.00071118. ONNX checker passed; all six held-out outputs agreed with PyTorch with maximum blend-map difference 0.00001431. The earlier single-photo synthetic benchmark scored 0.04258789 repair MAE versus 0.04179295 previously (1.90% worse, within the fixed 3% preservation limit); clean-output drift was 0.00042698. This is a measured tradeoff, not an improvement on every benchmark. Promotion checks that this preservation result belongs to the exact candidate being installed.

The installed generator SHA-256 is `348da8ed8114d0951cfb1e5026fd6ed4c70a087e92ab8a7421723b96b666cb40`. The previous active ONNX, Burn pack and matching training checkpoint are retained in `output/model-training/day-2/prior-active/`. Its hashes and aggregate acceptance results are recorded in local `models/retouch-training.json`. Existing AI caches naturally invalidate when model metadata changes. Restart or reanalyze an already prepared photo to use new maps.

For exact reproduction of the installed candidate, use the original run's output/seed and explicitly pass `--checkpoint output/model-training/day-2/prior-active/retouch_generator.pt --active-onnx output/model-training/day-2/prior-active/retouch_generator.onnx --steps 5000 --learning-rate 0.0005 --adaptation-weight 0.8`. This compares against the archived baseline while retaining the current active model.

The final installed version passed seven preparation/promotion/setup regression tests, Clippy with warnings denied, formatting/diff checks, the real-model portrait smoke checks, and source-folder write protection. ONNX CPU, Burn and Auto/DirectML rendered matching 3840×5760 skin/under-eye output with unchanged alpha and zero contrast/exposure. Both overview and native comparisons differed by at most one RGB byte between providers; DirectML kernel profiles verified all three demanded models, including the changed generator. All 76 source JPEG hashes were unchanged. Artifacts and provider comparisons are in `output/model-training/day-2/final-validation.json`; native render cases are under `final-cpu`, `final-burn` and `final-auto`. The earlier full-pipeline run additionally checked the unchanged blemish/inpainting models and automatic portrait controls. The final `paired_model_validation` example requests only the changed skin model and its geometry dependencies, keeping that check independent of the unchanged inpainting stage.

Methods follow the official [PyTorch transfer learning guidance](https://docs.pytorch.org/tutorials/beginner/transfer_learning_tutorial.html) and [OpenCV registration documentation](https://docs.opencv.org/4.x/dc/d6b/group__video__track.html).
