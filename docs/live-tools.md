# Live manual-tool performance

The subsequent [display profiling and SIMD pass](render-performance.md) measures and improves the CPU preparation and GPU upload/conversion stages that follow these native renders.

Brush edits submit a preview in the same UI frame as the pointer movement. The first overview update bypasses the slider pause debounce; subsequent overview submissions have a 16 ms minimum interval and only one full preview is outstanding. An in-flight intermediate result may appear while the pointer is held, followed by the newest revision. Releasing the pointer requires the final revision, so an older result cannot overwrite the finished stroke.

Overview and native-detail previews have separate image workers, each with a private Rayon pool of up to four threads. Mask and preset work cannot occupy their job queues. At high zoom, brush strokes request the visible native region while dragging and skip redundant overview work. The last sharp native frame remains visible until a newer native frame arrives; completing an overview or saving a workspace never substitutes an enlarged overview. On release, stale results are rejected and the final revision is rendered. The 1440-pixel overview limit and original-resolution export are unchanged.

Patch source dragging also uses the independent native-detail worker, rendering a temporary candidate without recording edits or history. A completed candidate replaces the canvas image pair, and a matching final frame stays visible until the committed preview arrives. The [manual-tool repair](manual-tools.md) documents target visibility checks, same-frame input registration, per-stroke healing strength, aligned clone sampling and corrected Patch texture sampling.

Liquify retains the previous deformation field. Appending strokes recomputes grid vertices within their combined footprints using the complete ordered inverse mapping. Changing the grid resolution, image dimensions, face geometry, earlier strokes or layer strength rebuilds the field. Native region rendering shares this cache across edits and pans.

Healing, clone and patch previews reuse completed pixels when newly appended operations follow the existing heal → clone → patch order. New operations still sample the immutable source. Native rendering likewise reuses prepared donor choices and boundary corrections. Undo, edited instructions, healing strength, layer opacity, and inserting a heal beneath an existing clone or patch invalidate the affected reuse. Finished output remains deterministic.

## Native viewport repair, 2026-10-03

The earlier 512×512 benchmark did not cover the full padded viewport used at high zoom. Each live brush result still redrew that entire viewport, delaying visible feedback despite prompt submission. `Renderer::render_viewport` now retains one sharp native frame (at most 32 MiB) and replaces the pixels affected by newly appended Liquify, Spot Heal and Clone Stamp instructions or the moving Patch donor. No resolution reduction is introduced. Changes to Patch boundaries restore both the previous and new destinations. Cleanup applied after deformation projects its source footprint through the deformation grid, including folded cells.

Changes to the image, segmentation, viewport, adjustments, layer metadata, earlier strokes or an existing deformation grid's resolution trigger the complete native renderer. Editing below another visible layer also uses the complete renderer because its filters and deformations can depend on changed input. An offscreen local edit does not redraw visible pixels. First contact uses the local path once the viewport baseline is ready; a cold baseline, large brush, cache miss or structural change can take longer. Frames exceeding the 32 MiB retention limit use the complete renderer.

Spot Heal additionally retains immutable filtered donor samples across strokes and pans. Each tile includes its blur radius in the cache key. The cache contains at most eight 512×512 half-float tiles (16 MiB per native source), counted inside the native renderer's existing 64 MiB cache budget. Releasing worker memory drops both the viewport frame and donor cache. Source JPEGs and learned ONNX/Burn models are untouched.

The new raw-egui integration test uses actual image workers and verifies two changing canvas results while the primary button remains held for all four tools, in both fitted and native views. One completed gesture is still one undo step. Pixel regression tests cover overlapping strokes, strong deformation followed by healing/cloning/patching, moving and reshaping Patch drafts, undo, pan, new source/masks, opacity, lower-layer changes and cancellation. Synthetic filtered regions use the existing native region tolerance of one RGB byte for floating-point accumulation/interpolation; the benchmark's portrait frames are byte-identical to the full native renderer.

The larger benchmark uses the same 3840×5760 photograph, a four-thread pixel pool, 48 existing instructions and 16 growing updates, including a moving Patch donor. Reproduce with `cargo run --release --locked --example viewport_performance` after compilation and other heavy work stop. Timing CSVs, reference frames and rendered frames are in `output/live-viewport/`. Timings measure CPU rendering and returning a complete sharp frame; they exclude GPU upload, screen refresh and input latency, and do not guarantee a fixed frame rate.

| Native viewport | Tool | Full redraw median | Live update median | Live p95 | Pixels redrawn |
| --- | --- | ---: | ---: | ---: | ---: |
| 1536×1024 | Liquify | 106.24 ms | 3.05 ms | 3.50 ms | 0.86% |
| 1536×1024 | Spot Heal | 152.60 ms | 5.60 ms | 30.86 ms | 0.29% |
| 1536×1024 | Clone Stamp | 185.83 ms | 8.76 ms | 17.09 ms | 0.62% |
| 1536×1024 | Patch | 131.95 ms | 8.90 ms | 10.11 ms | 0.96% |
| 2048×1536 | Liquify | 497.99 ms | 9.59 ms | 9.94 ms | 0.43% |
| 2048×1536 | Spot Heal | 284.57 ms | 8.92 ms | 37.40 ms | 0.15% |
| 2048×1536 | Clone Stamp | 307.98 ms | 11.17 ms | 12.10 ms | 0.31% |
| 2048×1536 | Patch | 256.40 ms | 10.27 ms | 12.41 ms | 0.48% |

Full redraw timings use the current renderer, including the donor cache, with viewport reuse bypassed. Before retaining donor samples, incremental Spot Heal still took 46.60/48.91 ms median in these two viewports; those measurements are preserved in `timings-before-donor-cache.csv`. All 128 incremental frames in the final benchmark are byte-identical to the complete native redraws, and all eight final PNGs match the saved frames from before the donor-cache change.

Validation: 195 optimized library tests passed with ONNX/Burn enabled, all-target Clippy passed with warnings denied, formatting and whitespace checks passed, and the release app rebuilt. The executable captured the 100% native workspace with Liquify selected and the 100% temporary Patch feedback canvas; both runs exited 0 with empty stderr. Captures are `output/live-viewport/native-workspace.png` and `patch-feedback.png`.

## Measured on this Windows workspace, 2026-10-02

The growing-stroke benchmark starts with 48 instructions, appends one per frame for 16 frames, and measures a warmed `Renderer`. It uses `example-img/CTU DUMANJUG ORG 09.28.26_JAMESBRO-522.JPG`: a 960×1440 overview and a 512×512 region of the 3840×5760 original. AI is inactive to isolate manual-tool rendering. The final PNGs in all six cases are byte-identical to references captured before these optimizations.

| Render | Tool | Before median | After median | After p95 |
| --- | --- | ---: | ---: | ---: |
| Overview | Liquify | 34.42 ms | 14.16 ms | 17.93 ms |
| Overview | Spot heal | 59.93 ms | 18.76 ms | 19.69 ms |
| Overview | Clone stamp | 49.61 ms | 17.23 ms | 19.44 ms |
| Native region | Liquify | 37.24 ms | 11.85 ms | 13.28 ms |
| Native region | Spot heal | 102.59 ms | 33.46 ms | 41.73 ms |
| Native region | Clone stamp | 36.41 ms | 14.86 ms | 17.32 ms |

These are CPU render timings, excluding display upload and refresh. Actual response depends on the photo, CPU, active adjustments, brush radius and accumulated instructions. A first render must still prepare source/filter caches; this does not guarantee 60 fps for every edit.

Reproduce without another compiler or heavy workload running:

```powershell
$env:CARGO_PROFILE_RELEASE_LTO='false'
cargo run --release --locked --no-default-features --example live_tools_performance
```

Captured reference PNGs and timing CSVs are in `output/live-tools/`. A regular run verifies those PNGs; `--reference` deliberately replaces them for a future baseline.

Regression coverage includes same-frame submissions for Liquify/heal/clone, native updates while dragging, retention of sharp pixels after release/overview completion/session save, exact incremental field and cleanup results after undo/overlap/layer changes, and preview completion/cancellation while the background worker is deliberately blocked.

The original optimization pass passed 143 optimized tests. The completed editable-layer, native-preview and layer-workflow follow-ups passed all 157 optimized tests with ONNX/Burn enabled. Formatting, whitespace checks and all-target Clippy with warnings denied passed. The rebuilt `target/release/hastur-retouch.exe` launched and captured the fitted workspace, compact panel and saved 100% native view with exit code 0 and empty stderr. Current captures are in `output/layer-stack/icons-desktop.png`, `icons-compact.png` and `icons-native.png`.
