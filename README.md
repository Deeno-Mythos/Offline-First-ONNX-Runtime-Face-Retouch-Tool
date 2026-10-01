# Astra Retouch

A native Rust portrait studio with a black-and-yellow workspace, real local editing, a wgpu photo canvas, and a non-destructive workflow. A generated adult studio portrait opens on first launch so the workspace is immediately usable.

## Run

On Windows, run `Launch-Astra.ps1`, or open `target/release/astra-retouch.exe` after building. From a terminal:

```powershell
cargo run --release --locked
```

Rust 1.95+ and a working native linker are required. The app was built and checked on Windows with Rust 1.98. No API key, sign-in or paid credits are required. Portrait AI runs locally with the installed ONNX models; `scripts/setup-ai.ps1` recreates this model/runtime setup on a new machine. The generated demo is embedded in the executable.

Pass photograph paths on the command line, use **Import**, or drop files on the canvas. Import only photos you own or have permission to edit. Supported formats: JPEG, PNG, TIFF, WebP, BMP. A session supports up to 64 photos; large batches consume memory because full-resolution originals are retained.

## Working features

- **Portrait AI:** automatic face detection and a 478-point face mesh, pretrained neural skin retouching, automatic blemish detection/inpainting, under-eye bag reduction, forehead wrinkle reduction and laugh-line smoothing. Detected eyes, eyebrows and lips are excluded from skin smoothing. Auto Retouch applies facial cleanup only and preserves current exposure, contrast, shadows, highlights, color, sharpening, background and shape settings; full-strength sliders produce a stronger visible effect. Style presets can still change color and light when explicitly selected. Automatic cleanup can remove freckles and moles; reduce the strength to retain them.
- **Face shape and sculpting:** eye size, nose width, lip plumpness and jawline use the detected mesh. Contour and highlight target facial structure automatically. Shape adjustments start at zero. The group is collapsed initially to keep the portrait panel compact.
- **Manual cleanup:** drag Liquify to push/pull any part of the image; Spot heal transfers nearby intact texture with boundary color matching; Clone stamp copies an exact source patch; Patch lets you lasso a repair area and drag it onto clean texture with a feathered, color-matched blend. Alt + click selects the clone source, then paint. Size, softness and strength are adjustable. Automatic blemish strength and manual healing strength are separate. Manual corrections also work outside detected faces.
- **Face details:** eyes, under-eyes and bright teeth inside the mouth are targeted automatically; brushes can refine the masks. A mouth-cavity label alone is never treated as teeth.
- **Adjustments:** inline enable toggles keep stored values while disabled. Double-click a slider track to reset it. Mask guidance appears next to localized adjustments.
- **Color:** linear-light exposure, contrast, highlights, shadows, relative temperature/tint, saturation, sharpening, and vignette. Selective HSL covers eight hue ranges; smooth luminance and RGB curves have large draggable points. Load a reference photograph asynchronously or use the current edited look to match selected portraits. Reference profiles travel with presets and sessions, and every source is measured independently. Each color family has its own amount and enable control. See [color workflow](docs/color-workflow.md).
- **Flyaway cleanup:** an explicit strength slider repairs fine isolated strands outside facial skin, around detected heads. It protects the face oval and coherent hair masses, using nearby intact background texture only when regional background evidence supports an exterior strand. This conservative detector does not use a semantic hair model; complex backgrounds may still need Spot heal or Patch. Auto Retouch keeps this control unchanged.
- **Background:** original, blur, solid backdrop, or transparent cutout, using a manual or ONNX mask. Continuous manual strokes support adjustable softness and add/erase. The mask overlay uses the renderer’s mask and erasure rules, with original-pixel regions at native zoom. Background changes are explicit selections and never part of Auto Retouch.
- **Comparison:** draggable yellow before/after divider; hold backslash to view the original; pointer-centered scroll zoom; Space + drag to pan. The 100% control uses original pixels at one physical screen pixel per image pixel, including Windows DPI scaling. Full-resolution edits load on demand; visible source regions are uploaded to the GPU. Fit, cursor-centered wheel zoom, bounded panning, double-click Fit/100%, and per-photo views are supported. Wheel and +/− use the same 1–800% limits. The GPU samples the correct visible portion of an image or native tile when zoom and pan move it outside the window, preserving its aspect ratio.
- **Presets:** Natural Headshot, Wedding Soft, Studio Editorial, E-commerce, Moody Portrait; load/save named RON presets. Cards render actual preset thumbnails; hovering temporarily previews the look without changing edits or history. Built-ins preserve the original background. E-commerce provides neutral adjustments; explicitly choose a white backdrop and supply a mask if needed.
- **Batch:** filmstrip with a yellow outline for the active photo and dots for selected photos, normal-click single selection, Ctrl-click toggles, Shift-click ranges, Ctrl+Shift-click additive ranges, synchronize adjustments, exposure-outlier notes, and up to five sharpness/exposure picks. Queued/Processing/Done reflect render requests and completed revisions; synchronized previews update even for unopened photos. Local masks and healing points stay with their own images. Picks do not evaluate closed eyes or expressions.
- **History/session:** gesture-level undo/redo and saved RON sessions include undo and redo branches, local strokes, warp gestures, clone sources, face landmarks, ratings, masks, views, selection, comparison, crops, and disabled adjustments. History retains up to 100 steps within a soft 64 MiB budget of unique action storage, always retaining the nearest undo step. Growing stroke histories serialize shared prefixes rather than repeating every list. Older v1 sessions remain readable. Autosave runs in the background after two quiet seconds or ten seconds of continued changes, retaining the preceding completed generation. Startup recovery lists saved workspaces; clean shutdown flushes the latest workspace. Session files reference original files, so keep them at their saved paths. Neural maps reload from the bounded local cache or are recomputed.
- **Export:** versioned JPG/PNG, original dimensions, web longest-edge 2048, Instagram 4:5, Story 9:16, LinkedIn 1:1. Social exports show the output dimensions and an interactive crop preview; drag or click to reposition, including individual framing for selected batch photos. Cancel export stops remaining photos and cooperatively interrupts the current render/encoding; completed exports remain, and partial files are removed. PNG retains alpha; JPEG composites transparency onto white. Output includes an sRGB ICC profile and a versioned Markdown report with Photo / Output / Edits applied / Notes. Existing files and originals are never overwritten.
- **Compact workspace:** narrower windows use bottom tool tabs and a floating tool sheet, with a full-width photo and horizontally scrolling filmstrip.

## Local ONNX portrait AI

ONNX support is enabled in the default build. The five installed models provide face detection, dense landmarks, neural retouching, blemish segmentation and inpainting. No photos or inference requests leave the device. The model and runtime binaries live in `models/`; they are excluded from source control. See [model provenance, contracts and licenses](models/README.md).

On a new Windows machine, recreate the verified setup before building:

```powershell
.\scripts\setup-ai.ps1
cargo run --release --locked
```

Models run once per imported photo on a dedicated portrait worker. ONNX sessions and resulting maps are reused while sliders change. Idle inference threads sleep, and CPU inference flushes denormal values to zero to avoid arithmetic stalls; real-model output checks cover this configuration. A bounded disk cache also avoids repeating inference when unchanged photos reopen. Face masks arrive first, then neural maps; the UI shows the actual preparation state and waits for requested AI before export. Missing models/runtime and provider failures are recoverable and visible. Manual tools remain available. CPU preparation can be slow, particularly for the inpainting network; the UI stays responsive while it runs.

In **AI models**, reanalyze the portrait or choose an additional face/background segmentation model. Set `ORT_DYLIB_PATH` to override the local runtime. ONNX Runtime 1.24.4 uses the v24 API and remains the default CPU backend. Builds with the bundled models also offer `Burn CPU`, executing Burn-generated Rust models through CubeCL CPU. Fresh isolated optimized-release benchmarks on this computer, with LTO disabled, measured complete warmed AI at 5.166 seconds through ONNX Runtime and 119.110 seconds through Burn, with close rendered output agreement. Burn used less peak process memory and remains selectable. See [measured runtime and portrait validation](docs/portrait-validation.md). Imported custom ONNX masks use ONNX Runtime. DirectML, CUDA and CoreML require a compatible runtime/provider; selecting a provider does not imply that inference ran on a GPU.

Additional segmentation contracts:

| Task | Input | Output |
| --- | --- | --- |
| Face parsing | Float32 `[1,3,512,512]`, RGB ImageNet mean/std | Float32 `[1,19,H,W]` logits using CelebAMask-HQ class order |
| Foreground segmentation | Float32 `[1,3,1024,1024]`, RGB in `[-0.5,0.5]` | Float32 `[1,1,H,W]` foreground probabilities in `[0,1]` |

The built-in portrait pipeline detects and aligns individual faces. Custom whole-image parsing models retain their documented contract; arbitrary RMBG/U2-Net variants are not interchangeable.

Official references: [egui](https://github.com/emilk/egui), [egui-wgpu callbacks](https://docs.rs/egui-wgpu/0.33.3/egui_wgpu/trait.CallbackTrait.html), [ort sessions](https://docs.rs/ort/2.0.0-rc.13/ort/session/struct.Session.html), [ONNX Runtime execution providers](https://onnxruntime.ai/docs/execution-providers/).

## Architecture and limits

`src/engine.rs` owns deterministic image processing and export. Adjustments operate in linear sRGB float32; GPU compute converts the preview into Rgba16Float working textures for an egui-wgpu/WGSL canvas. The renderer converts back to display sRGB and supports both sRGB and non-sRGB framebuffer targets. Retouch filtering runs on the CPU with Rayon; it uses separable sliding box filters for frequency separation. It is not a full GPU Gaussian/HSL/LUT editing engine. Masks are float arrays on the CPU, not R8 GPU masks. Import, portrait inference, session I/O, preview rendering, and cancellable export have separate workers. Preview work takes priority over detail and preset previews, and obsolete renders are cancelled. Neural blend maps are applied to original-resolution pixels, preserving the source detail; inverse sampling applies face and manual deformations at every output size. Workers request repaint through channels. Preview jobs are debounced, share bounded source/filter caches, and discard stale results. Original GPU textures are retained across edits, and native viewports reuse padded source tiles while panning.

Embedded RGB ICC profiles are converted to sRGB with moxcms, and EXIF orientation is applied. Untagged images are treated as sRGB. Non-RGB ICC profiles are rejected with a conversion message. Editing/export currently uses 8-bit source RGBA and 8-bit JPG/PNG; 16-bit inputs are converted to 8-bit. Full-resolution export recalculates edits from the untouched original in 512-pixel tiles with filter halos and global mask, brush, color, and deformation coordinates. Floating-point filtering buffers stay bounded rather than growing with image size. Originals and final 8-bit export buffers remain resident. Native zoom and preset hover request only the padded visible region, reusing preparation across pans; they retain sharp source detail while a newer edit renders. Overview previews have a 1440-pixel longest edge; images above 80 megapixels are rejected. Monitor/display ICC calibration is not implemented.

Not implemented: makeup, age/gender detection, closed-eye culling, semantic group skin-tone matching, LaMa inpainting, arbitrary scene replacement, RAW development, or Lightroom/Photoshop plug-in integration. Native panels use flat translucent-compatible styling; live backdrop blur is not implemented.

## Performance verification

See [sessions and workspace recovery](docs/workspace-recovery.md) for reopening work and [goal implementation and validation](docs/goal-validation.md) for the current verification status of the nine workflow improvements.

`examples/performance.rs` measures imports, cached AI, all retouch families, manual tools, mask overlays, full-resolution rendering, JPG/PNG export, warmed previews, and snapshot copies. It compares optimized previews and PNGs against captured baselines (at most one byte per channel, mean error below 0.05). JPEG comparisons allow a small codec difference because DCT quantization can spread a one-byte input change. Use `--reference` to capture a baseline before a future change; regular runs validate it. `--inference` also runs all real ONNX models after rendering and exporting, checking the resulting maps against the cached analysis. Combine it with `--ai-first` to check inference both before and after export. Warmed preview timings report the median of five runs. Keep the compiler and other heavy jobs stopped while timing inference. Measurements from this optimization pass are in `output/performance/validation.md`.

Preview reuse is capped at 160 MiB and two sources. Batch originals remain resident; large images and new model sessions still require substantial memory. Model input resolution, network weights, exported resolution, and linear-light processing are preserved.

## Validation

```powershell
cargo fmt --check
cargo check --locked --features onnx
cargo clippy --locked --all-targets --features onnx -- -D warnings
cargo test --release --locked
cargo run --release --locked --example smoke
cargo run --release --locked --example ai_smoke
```

The regression suite includes real egui pointer, wheel, modifier, slider, brush, hover, crop, and cancellation events. It verifies source pixels and DPI at 100%, input ownership, stroke continuity, per-photo views, stale render rejection, unopened batch updates, and session compatibility. Worker tests verify distinct preset thumbnails and batch cancellation with completed files preserved. Engine tests cover neutral byte identity, original/geometry/alpha preservation, localized masks, transparent cutouts, history branching, preset persistence, and race-safe versioned exports. The AI smoke example uses the real models and validates increasing 50/100 strength for all eleven portrait adjustments, saving landmarks, masks and rendered variants in `output/ai-validation`. The smoke example imports the generated portrait and verifies three full-resolution preset exports without changing the source.

To render a native UI screenshot and exit:

```powershell
target/release/astra-retouch.exe --screenshot output/studio.png
target/release/astra-retouch.exe --compare --screenshot output/comparison.png
target/release/astra-retouch.exe --100 --screenshot output/native-detail.png
target/release/astra-retouch.exe --export-preview --screenshot output/export-crop.png
target/release/astra-retouch.exe --tool presets --screenshot output/presets.png
target/release/astra-retouch.exe --size 800x760 --screenshot output/compact.png
```

`scripts/fetch-deps.py` is an optional checksum-verified Cargo.lock downloader for broken DNS resolvers. Normal installations use Cargo directly. It accepts a currently resolved IPv4 address for static.crates.io and uses curl's per-request resolution override without modifying system DNS.

The demo asset and its generation prompt are documented in `assets/README.md` and `assets/demo-prompt.txt`.
