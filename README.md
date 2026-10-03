# Hastur Retouch

A native Rust portrait studio with a black-and-yellow workspace, real local editing, a wgpu photo canvas, and a non-destructive workflow. A generated adult studio portrait opens with neutral settings on first launch so the workspace is immediately usable and portrait AI stays dormant until requested.

The startup splash uses the demo woman selected by the user and refined five-spire crown. Its loading stages come from real import and workspace restoration events, plus cache, model execution and GPU qualification when the restored edits require AI. It opens the workspace when the first preview and any requested portrait analysis are ready; **Open workspace** lets editing begin while portrait AI continues. The portrait retains its proportions when the window resizes. Crown assets are embedded in both the window and Windows executable.

## Run

On Windows, run `Launch-Hastur.ps1`, or open `target/release/hastur-retouch.exe` after building. The launcher accepts photograph paths and the same command-line options as the executable. From a terminal:

```powershell
cargo run --release --locked
```

Hastur Retouch was previously named Astra Retouch. Existing sessions and presets remain compatible, and older recovery workspaces and AI caches are discovered without moving or deleting them. `Launch-Astra.ps1` forwards to the new launcher, including any arguments. See [workspace recovery](docs/workspace-recovery.md) for the legacy locations and override names.

Rust 1.95+ and a working native linker are required. The app was built and checked on Windows with Rust 1.98. No API key, sign-in or paid credits are required. Portrait AI runs locally with the installed ONNX models; `scripts/setup-ai.ps1` recreates this model/runtime setup on a new machine. The generated demo is embedded in the executable.

Pass photograph paths on the command line, use **Import**, or drop files on the canvas. Import only photos you own or have permission to edit. Supported formats: JPEG, PNG, TIFF, WebP, BMP. A session supports up to 64 photos. Import and recovery retain metadata and sharp thumbnails; full-resolution pixels are loaded for the active photo and released when switching. File → Import folder stages a directory in the background. See [project performance and workflow](docs/asset-pipeline.md).

## Working features

- **NullState AI:** neutral import, color adjustments, navigation and manual cleanup do not load portrait models. Enabled portrait edits request only facial geometry, neural skin retouch or automatic blemish repair and their shared dependencies. Completed analysis is reused, and the portrait worker releases model instances after 90 idle seconds while retaining photo maps and edits. Auto Retouch and explicit Reanalyze portrait can request all stages. See [NullState lifecycle and validation](docs/nullstate.md).
- **Portrait AI:** automatic face detection and a 478-point face mesh, pretrained neural skin retouching, automatic blemish detection/inpainting, under-eye bag reduction, forehead wrinkle reduction and laugh-line smoothing. Detected eyes, eyebrows and lips are excluded from skin smoothing. Auto Retouch applies facial cleanup only and preserves current exposure, contrast, shadows, highlights, color, sharpening, background and shape settings; full-strength sliders produce a stronger visible effect. Style presets can still change color and light when explicitly selected. Automatic cleanup can remove freckles and moles; reduce the strength to retain them.
- **Face shape and sculpting:** eye size, nose width, lip plumpness and jawline use the detected mesh. Contour and highlight target facial structure automatically. Shape adjustments start at zero. The group is collapsed initially to keep the portrait panel compact.
- **Manual cleanup:** drag Liquify to push/pull any part of the image; Spot heal transfers nearby intact texture with boundary color matching; Clone stamp copies selected texture; Patch lets you lasso a repair area and drag it onto clean texture with a live, feathered, color-matched preview. **Choose source** or Alt+click selects the clone source, with optional alignment across strokes. Size in original pixels, softness and per-stroke strength are adjustable. The canvas explains and repairs hidden, transparent or unavailable layer targets. Manual corrections also work outside detected faces. See [manual tools and validation](docs/manual-tools.md).
- **Face details:** eyes, under-eyes and bright teeth inside the mouth are targeted automatically; brushes can refine the masks. A mouth-cavity label alone is never treated as teeth.
- **Adjustments:** inline enable toggles keep stored values while disabled. Double-click a slider track to reset it. Mask guidance appears next to localized adjustments.
- **Color:** linear-light exposure, contrast, highlights, shadows, relative temperature/tint, saturation, sharpening, and vignette. Selective HSL covers eight hue ranges; smooth luminance and RGB curves have large draggable points. Load a reference photograph asynchronously or use the current edited look to match selected portraits. Reference profiles travel with presets and sessions, and every source is measured independently. Each color family has its own amount and enable control. See [color workflow](docs/color-workflow.md).
- **Flyaway cleanup:** an explicit strength slider repairs fine isolated strands outside facial skin, around detected heads. It protects the face oval and coherent hair masses, using nearby intact background texture only when regional background evidence supports an exterior strand. This conservative detector does not use a semantic hair model; complex backgrounds may still need Spot heal or Patch. Auto Retouch keeps this control unchanged.
- **Background:** original, blur, solid backdrop, or transparent cutout, using a manual or ONNX mask. Continuous manual strokes support adjustable softness and add/erase. The mask overlay uses the renderer’s mask and erasure rules, with original-pixel regions at native zoom. Background changes are explicit selections and never part of Auto Retouch.
- **Comparison:** draggable yellow before/after divider; hold backslash to view the original; pointer-centered scroll zoom; Space + drag to pan. The 100% control uses original pixels at one physical screen pixel per image pixel, including Windows DPI scaling. Full-resolution edits load on demand; visible source regions are uploaded to the GPU. Fit, cursor-centered wheel zoom, bounded panning, double-click Fit/100%, and per-photo views are supported. Wheel and +/− use the same 1–800% limits. The GPU samples the correct visible portion of an image or native tile when zoom and pan move it outside the window, preserving its aspect ratio.
- **Presets:** Natural Headshot, Wedding Soft, Studio Editorial, E-commerce, Moody Portrait; load/save named RON presets. Cards render actual preset thumbnails; hovering temporarily previews the look without changing edits or history. Built-ins preserve the original background. E-commerce provides neutral adjustments; explicitly choose a white backdrop and supply a mask if needed.
- **Batch:** filmstrip with a yellow outline for the active photo and dots for selected photos, normal-click single selection, Ctrl-click toggles, Shift-click ranges, Ctrl+Shift-click additive ranges, synchronize adjustments, exposure-outlier notes, and up to five sharpness/exposure picks. Queued/Processing/Done reflect render requests and completed revisions; synchronized previews update even for unopened photos. Local masks and healing points stay with their own images. Picks do not evaluate closed eyes or expressions.
- **History and layers panels:** Tools, History and Layers share one hideable inspector with View-menu access at every width. Tab hides or reopens the last panel; history rows restore earlier or redo states. Up to 32 independent retouch, adjustment and original-copy layers support thumbnails, names, ordering, opacity, blend modes, locking and reversible visibility above an immutable Background. Vector icon controls, name/type filters, Alt-click solo, right-click actions, color labels, adjustment copy/paste/reset and layer shortcuts streamline editing. Layers and their history are saved per photo. See [workspace panels](docs/workspace-panels.md).
- **History/session:** gesture-level undo/redo and saved RON sessions include undo and redo branches, local strokes, warp gestures, clone sources, face landmarks, ratings, masks, views, selection, comparison, crops, and disabled adjustments. History retains up to 100 steps within a soft 64 MiB budget of unique action storage, always retaining the nearest undo step. Growing stroke histories serialize shared prefixes rather than repeating every list. Older v1 sessions remain readable. Autosave runs in the background after two quiet seconds or ten seconds of continued changes, retaining the preceding completed generation. Startup automatically restores the newest usable saved workspace; clean shutdown flushes the latest workspace. Session files reference original files, so keep them at their saved paths. Neural maps reload from the bounded local cache or are recomputed.
- **Project assets and export queue:** right-click a filmstrip thumbnail for Delete from Project. File → Export queue shows independent jobs, progress and cancellation, with Print/Web/Cutout presets and one or two concurrent jobs. Original files remain untouched. See [asset pipeline validation](docs/asset-pipeline.md).
- **Export:** versioned JPG/PNG, original dimensions, web longest-edge 2048, Instagram 4:5, Story 9:16, LinkedIn 1:1. Social exports show the output dimensions and an interactive crop preview; drag or click to reposition, including individual framing for selected batch photos. Cancel export stops remaining photos and cooperatively interrupts the current render/encoding; completed exports remain, and partial files are removed. PNG retains alpha; JPEG composites transparency onto white. Output includes an sRGB ICC profile and a versioned Markdown report with Photo / Output / Edits applied / Notes. Existing files and originals are never overwritten.
- **Compact workspace:** narrower windows use bottom tool tabs and a floating Tools/History/Layers sheet, with a full-width photo and horizontally scrolling filmstrip. Done dismisses the sheet; View provides panel access.
- **Everyday controls:** saved Preferences (`Ctrl+,`) cover interface size, motion, wheel behavior, brush guides/defaults, autosave timing/startup restoration and export defaults. `Ctrl+K` searches the app command catalog. Preferences → Shortcuts records personal bindings; Workspace preferences can disable hover tooltips. `F` focuses the photo workspace, `F6` toggles the filmstrip, brackets adjust brush size (Shift: strength), arrows navigate photos (Shift: select range), and 1–5 rate photos. Text fields and modal dialogs protect normal typing and canvas input. See [preferences and shortcuts](docs/app-preferences.md).

## Local ONNX portrait AI

ONNX support is enabled in the default build. The five installed models provide face detection, dense landmarks, neural retouching, blemish segmentation and inpainting. No photos or inference requests leave the device. The model and runtime binaries live in `models/`; they are excluded from source control. See [model provenance, contracts and licenses](models/README.md).

Matched original/edited portraits can also be used for offline local skin-model fine-tuning. The paired pipeline aligns cropped edits, excludes global grading/reshaping, splits by person and compares a candidate against both the current AI and untouched photos. See [paired training and validation](docs/paired-training.md).

On a new Windows machine, recreate the verified setup before building:

```powershell
.\scripts\setup-ai.ps1
cargo run --release --locked
```

NullState runs only the portrait stages needed by enabled edits on a dedicated worker. Face shape, eyes, teeth, tone, contour and flyaway cleanup need facial geometry; skin smoothing, under-eye bags and wrinkle/laugh-line cleanup also need the retouch generator; automatic blemishes need detection and inpainting. Shared dependencies run once per photo and provider, and completed maps are reused while sliders change. Neutral photos, including the fresh demo, do not prepare portrait AI. Restored active edits request their missing analysis automatically. **Reanalyze portrait** explicitly recomputes all stages.

Model instances are created on first use and released after 90 seconds of idle portrait work. Generated maps and bounded disk caches remain usable, so repeating a prepared effect does not reload weights. The runtime library and allocator/driver caches can remain initialized; dormancy does not promise zero process memory. Idle inference threads sleep, and CPU inference flushes denormal values to zero to avoid arithmetic stalls; real-model output checks cover this configuration. Face masks arrive first, then requested neural maps; the UI shows the actual preparation state and waits for requested AI before export. Missing models/runtime and provider failures are recoverable and visible. Manual tools remain available. CPU preparation can be slow, particularly for the inpainting network; the UI stays responsive while it runs.

In **AI models**, reanalyze the portrait or choose an additional face/background segmentation model. **Auto** is the default for bundled portrait models. It checks real GPU kernel execution and compares each model's first output with CPU before keeping that provider. Unsupported, incorrect, or failed GPU execution falls back visibly to CPU. On Windows, DirectML vendor metacommands are disabled to avoid incorrect output on the tested Intel driver. The GPU-capable runtime also supports CPU and preserves explicit CPU, DirectML, CUDA, CoreML, and Burn choices. See [hardware acceleration and validation](docs/hardware-acceleration.md). Imported custom mask models use CPU under Auto; choose an explicit GPU provider for those models.

Set `ORT_DYLIB_PATH` to override the local ONNX Runtime 1.24.4 runtime (v24 API). `scripts/setup-ai-runtime.ps1` installs only the DirectML runtime without repeating model conversions. Builds with bundled models also offer **Burn CPU**, executing Burn-generated Rust models through CubeCL CPU. Earlier isolated optimized-release benchmarks on this computer measured warmed CPU AI at 5.166 seconds through ONNX Runtime and 119.110 seconds through Burn, with close rendered output agreement. Burn used less peak process memory and remains selectable. See [runtime and portrait validation](docs/portrait-validation.md).

Additional segmentation contracts:

| Task | Input | Output |
| --- | --- | --- |
| Face parsing | Float32 `[1,3,512,512]`, RGB ImageNet mean/std | Float32 `[1,19,H,W]` logits using CelebAMask-HQ class order |
| Foreground segmentation | Float32 `[1,3,1024,1024]`, RGB in `[-0.5,0.5]` | Float32 `[1,1,H,W]` foreground probabilities in `[0,1]` |

The built-in portrait pipeline detects and aligns individual faces. Custom whole-image parsing models retain their documented contract; arbitrary RMBG/U2-Net variants are not interchangeable.

Official references: [egui](https://github.com/emilk/egui), [egui-wgpu callbacks](https://docs.rs/egui-wgpu/0.33.3/egui_wgpu/trait.CallbackTrait.html), [ort sessions](https://docs.rs/ort/2.0.0-rc.13/ort/session/struct.Session.html), [ONNX Runtime execution providers](https://onnxruntime.ai/docs/execution-providers/).

## Architecture and limits

`src/engine.rs` owns deterministic image processing and export. Adjustments operate in linear sRGB float32; GPU compute converts the preview into Rgba16Float working textures for an egui-wgpu/WGSL canvas. The renderer converts back to display sRGB and supports both sRGB and non-sRGB framebuffer targets. Retouch filtering runs on the CPU with Rayon; it uses separable sliding box filters for frequency separation. It is not a full GPU Gaussian/HSL/LUT editing engine. Masks are float arrays on the CPU, not R8 GPU masks. Import, portrait inference, session I/O, preview rendering, and cancellable export have separate workers. Preview work takes priority over detail and preset previews, and obsolete renders are cancelled. Neural blend maps are applied to original-resolution pixels, preserving the source detail; inverse sampling applies face and manual deformations at every output size. Workers request repaint through channels. Slider preview jobs are debounced. Brush edits submit immediately to a dedicated worker and reuse deformation and cleanup caches. Both paths share bounded source/filter caches and discard obsolete results after release. Original GPU textures are retained across edits, and native viewports reuse padded source tiles while panning.

Embedded RGB ICC profiles are converted to sRGB with moxcms, and EXIF orientation is applied. Untagged images are treated as sRGB. Non-RGB ICC profiles are rejected with a conversion message. Editing/export currently uses 8-bit source RGBA and 8-bit JPG/PNG; 16-bit inputs are converted to 8-bit. Full-resolution export recalculates edits from the untouched original in 512-pixel tiles with filter halos and global mask, brush, color, and deformation coordinates. Floating-point filtering buffers stay bounded rather than growing with image size. Originals and final 8-bit export buffers remain resident. Native zoom and preset hover request only the padded visible region, reusing preparation across pans; they retain sharp source detail while a newer edit renders. At high zoom, brush drags update the visible native region and retain its sharp pixels while newer results render. Overview previews have a 1440-pixel longest edge; images above 80 megapixels are rejected. Monitor/display ICC calibration is not implemented.

Not implemented: makeup, age/gender detection, closed-eye culling, semantic group skin-tone matching, LaMa inpainting, arbitrary scene replacement, RAW development, or Lightroom/Photoshop plug-in integration. Native panels use flat translucent-compatible styling; live backdrop blur is not implemented.

## Performance verification

See [live manual-tool performance](docs/live-tools.md) for growing-stroke measurements and refinement behavior, [display profiling and SIMD](docs/render-performance.md) for GPU upload improvements, [sessions and workspace recovery](docs/workspace-recovery.md) for reopening work, and [goal implementation and validation](docs/goal-validation.md) for the current verification status of the nine workflow improvements.

`examples/performance.rs` measures imports, cached AI, all retouch families, manual tools, mask overlays, full-resolution rendering, JPG/PNG export, warmed previews, and snapshot copies. It compares optimized previews and PNGs against captured baselines (at most one byte per channel, mean error below 0.05). JPEG comparisons allow a small codec difference because DCT quantization can spread a one-byte input change. Use `--reference` to capture a baseline before a future change; regular runs validate it. `--inference` also runs all real ONNX models after rendering and exporting, checking the resulting maps against the cached analysis. Combine it with `--ai-first` to check inference both before and after export. Warmed preview timings report the median of five runs. Keep the compiler and other heavy jobs stopped while timing inference. Measurements from this optimization pass are in `output/performance/validation.md`.

Preview source/filter reuse is capped at 160 MiB and two sources. Editable layers also reuse at most two lower-layer composites within 192 MiB. Inactive batch originals are released; large active images and new model sessions still require substantial memory. Model input resolution, network weights, exported resolution, and linear-light processing are preserved.

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
target/release/hastur-retouch.exe --screenshot output/studio.png
target/release/hastur-retouch.exe --compare --screenshot output/comparison.png
target/release/hastur-retouch.exe --100 --screenshot output/native-detail.png
target/release/hastur-retouch.exe --export-preview --screenshot output/export-crop.png
target/release/hastur-retouch.exe --tool presets --screenshot output/presets.png
target/release/hastur-retouch.exe --size 800x760 --screenshot output/compact.png
target/release/hastur-retouch.exe --splash-preview --screenshot output/splash.png
target/release/hastur-retouch.exe --preferences-preview --screenshot output/preferences.png
target/release/hastur-retouch.exe --commands-preview --screenshot output/commands.png
target/release/hastur-retouch.exe --focus-workspace --screenshot output/focus.png
```

`scripts/fetch-deps.py` is an optional checksum-verified Cargo.lock downloader for broken DNS resolvers. Normal installations use Cargo directly. It accepts a currently resolved IPv4 address for static.crates.io and uses curl's per-request resolution override without modifying system DNS.

The demo asset and its generation prompt are documented in `assets/README.md` and `assets/demo-prompt.txt`.
