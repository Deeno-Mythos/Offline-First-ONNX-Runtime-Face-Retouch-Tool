# Astra Retouch

A native Rust portrait studio with a black-and-yellow workspace, real local editing, a wgpu photo canvas, and a non-destructive workflow. A generated adult studio portrait opens on first launch so the workspace is immediately usable.

## Run

On Windows, run `Launch-Astra.ps1`, or open `target/release/astra-retouch.exe` after building. From a terminal:

```powershell
cargo run --release --locked
```

Rust 1.88+ and a working native linker are required. The app was built and checked on Windows with Rust 1.98. No API key, sign-in, paid credits, or model download is required for local editing. The generated demo is embedded in the executable.

Pass photograph paths on the command line, use **Import**, or drop files on the canvas. Import only photos you own or have permission to edit. Supported formats: JPEG, PNG, TIFF, WebP, BMP. A session supports up to 64 photos; large batches consume memory because full-resolution originals are retained.

## Working features

- **Portrait:** Auto Retouch applies conservative local settings. Skin smoothing keeps fine-scale detail and attenuates only the medium-frequency layer; tone evenness and redness reduction use a skin mask. The default mask is a conservative color heuristic, not face detection. Always inspect and refine it, particularly near clothing or backgrounds of similar color.
- **Local detail:** Paint masks for teeth, eyes, and under-eyes before adjusting their strengths. The spot-heal brush samples surrounding color, blends the marked spot, and retains some fine texture. It is intended for small blemishes, not complex object removal.
- **Color:** linear-light exposure, contrast, highlights, shadows, relative temperature/tint, saturation, sharpening, and vignette.
- **Background:** original, blur, solid backdrop, or transparent cutout, using a manual or ONNX mask. Feathered manual stamps support add/erase. Background changes are explicit selections and never part of Auto Retouch.
- **Comparison:** draggable yellow before/after divider; hold backslash to view the original; pointer-centered scroll zoom; Space + drag to pan. The 100% control shows preview pixels, not full-resolution originals.
- **Presets:** Natural Headshot, Wedding Soft, Studio Editorial, E-commerce, Moody Portrait; load/save named RON presets. Built-ins preserve the original background. E-commerce provides neutral adjustments; explicitly choose a white backdrop and supply a mask if needed.
- **Batch:** filmstrip, Ctrl-click multiselect, synchronize adjustments, exposure-outlier notes, and up to five sharpness/exposure picks. Local masks and healing points stay with their own images. Picks do not evaluate closed eyes or expressions.
- **History/session:** gesture-level undo/redo, up to 100 steps, and saved RON edit sessions including source references, local strokes, ratings, and model masks. Session files do not embed original photos; keep the source files at their saved paths. Undo history resets on reopening.
- **Export:** versioned JPG/PNG, original dimensions, web longest-edge 2048, Instagram 4:5, Story 9:16, LinkedIn 1:1. Social exports center-crop. PNG retains alpha; JPEG composites transparency onto white. Output includes an sRGB ICC profile and a versioned Markdown report with Photo / Output / Edits applied / Notes. Existing files and originals are never overwritten.
- **Compact workspace:** narrower windows use bottom tool tabs and a floating tool sheet, with a full-width photo and horizontally scrolling filmstrip.

## Optional ONNX segmentation

The model-loading UI becomes available with this build:

```powershell
cargo run --release --locked --features onnx
```

Supply an ONNX Runtime shared library compatible with `ort 2.0.0-rc.13` / the ONNX Runtime v24 API. Set `ORT_DYLIB_PATH` to its full path or place `onnxruntime.dll` in `models/` on Windows (`libonnxruntime.so` on Linux, `libonnxruntime.dylib` on macOS). Models and runtime binaries are intentionally not bundled or automatically downloaded. Review the licenses of models you supply, particularly for commercial work.

In **AI models**, choose a task and provider, then select an ONNX file. Inference runs on a worker thread; errors are shown in the UI. Provider failures are explicit, not silently relabeled as GPU processing. Supported contracts:

| Task | Input | Output |
| --- | --- | --- |
| Face parsing | Float32 `[1,3,512,512]`, RGB ImageNet mean/std | Float32 `[1,19,H,W]` logits using CelebAMask-HQ class order |
| Foreground segmentation | Float32 `[1,3,1024,1024]`, RGB in `[-0.5,0.5]` | Float32 `[1,1,H,W]` foreground probabilities in `[0,1]` |

Face parsing supplies skin/eye masks; teeth and under-eyes remain manual because a mouth-cavity label is not a teeth label. Foreground models must match the documented preprocessing and output contract; arbitrary RMBG/U2-Net variants are not interchangeable. Current parsing resizes the whole image, so face-close portraits work best. Face detection/alignment is not implemented.

Providers: CPU, DirectML, CUDA, CoreML. DirectML requires a compatible Windows runtime; CUDA/CoreML require their respective platform/runtime support. The ONNX feature is compile-checked. Inference on real model weights has not been validated in this workspace because no runtime or model weights were supplied.

Official references: [egui](https://github.com/emilk/egui), [egui-wgpu callbacks](https://docs.rs/egui-wgpu/0.33.3/egui_wgpu/trait.CallbackTrait.html), [ort sessions](https://docs.rs/ort/2.0.0-rc.13/ort/session/struct.Session.html), [ONNX Runtime execution providers](https://onnxruntime.ai/docs/execution-providers/).

## Architecture and limits

`src/engine.rs` owns deterministic image processing and export. Adjustments operate in linear sRGB float32; GPU compute converts the preview into Rgba16Float working textures for an egui-wgpu/WGSL canvas. The renderer converts back to display sRGB and supports both sRGB and non-sRGB framebuffer targets. Retouch filtering runs on the CPU with Rayon; it uses separable sliding box filters for frequency separation. It is not a full GPU Gaussian/HSL/LUT editing engine. Masks are float arrays on the CPU, not R8 GPU masks. Import, preview rendering, inference, and export run off the UI thread and request repaint through channels. Preview jobs are debounced and stale results are discarded.

Embedded RGB ICC profiles are converted to sRGB with moxcms, and EXIF orientation is applied. Untagged images are treated as sRGB. Non-RGB ICC profiles are rejected with a conversion message. Editing/export currently uses 8-bit source RGBA and 8-bit JPG/PNG; 16-bit inputs are converted to 8-bit. Full-resolution export recalculates edits from the untouched original. Preview longest edge is capped at 1440, and images above 80 megapixels are rejected. Large full-resolution exports can require several GB of memory. Monitor/display ICC calibration is not implemented.

Not implemented: face/body sculpting, makeup, automatic blemish/wrinkle/flyaway cleanup, face/age/gender detection, closed-eye culling, group skin-tone matching, reference color matching, LaMa inpainting, arbitrary scene replacement, RAW development, HSL/curves, or Lightroom/Photoshop plug-in integration. The UI lists these limits instead of presenting inactive sliders as functioning AI tools. Native panels use flat translucent-compatible styling; live backdrop blur is not implemented.

## Validation

```powershell
cargo fmt --check
cargo check --locked --features onnx
cargo clippy --locked --all-targets --features onnx -- -D warnings
cargo test --release --locked
cargo run --release --locked --example smoke
```

The regression suite covers neutral byte identity, original/geometry/alpha preservation, localized masks, transparent cutouts, history branching, preset persistence, and race-safe versioned exports. The smoke example imports the generated portrait and verifies three full-resolution preset exports without changing the source.

To render a native UI screenshot and exit:

```powershell
target/release/astra-retouch.exe --screenshot output/studio.png
target/release/astra-retouch.exe --compare --screenshot output/comparison.png
target/release/astra-retouch.exe --tool presets --screenshot output/presets.png
target/release/astra-retouch.exe --size 800x760 --screenshot output/compact.png
```

`scripts/fetch-deps.py` is an optional checksum-verified Cargo.lock downloader for broken DNS resolvers. Normal installations use Cargo directly. It accepts a currently resolved IPv4 address for static.crates.io and uses curl's per-request resolution override without modifying system DNS.

The demo asset and its generation prompt are documented in `assets/README.md` and `assets/demo-prompt.txt`.
