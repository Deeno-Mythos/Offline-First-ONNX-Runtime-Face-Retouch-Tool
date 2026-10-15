# Manual-tool input feedback, 2026-10-04

The sharp native canvas now completes before its overview after a short click or released manual stroke. Previously, Heal, Clone and committed Patch feedback could wait for a full overview despite an independent native worker being available. Held native strokes already had priority and retain that behavior.

Fitted manual previews and temporary fitted Patch feedback also use the bounded affected-area renderer. It keeps one complete frame (up to 32 MiB per renderer) and replaces only the exact changed footprint. The first whole-overview frame uses the faster complete renderer to warm that cache; native subregions continue to use the region renderer. Global adjustments, structural edits, undo/removal and source/segmentation changes use the complete path. Pure adjustments without manual work retain their previous cached renderer. A sampled lower layer can depend on changed upper pixels at a distant destination; that case deliberately redraws completely. The full/native image sampling and pixel math are unchanged.

## Measurement

`feedback_latency` replays actual egui pointer-button/movement events through `HasturApp`, the production worker threads, canvas callbacks, changed-pixel upload, linear conversion and a completed offscreen GPU frame. It measures Liquify, Spot Heal, Clone Stamp and moving/committed Patch at Fit, 62%, 90% and 100%. Each tool/view records eight held movements and eight short press/move/release gestures after the workspace settles. The initial press happens before the measured movement; unfinished first-frame preparation may affect the first sample, and later samples reuse the growing edit state. These primarily measure successive-stroke feedback, not isolated cold first-click cost. The source is the original 3840×5760 supplied JPEG, with manual edits only and no portrait inference.

The A/B comparison uses the same release executable. `--baseline` disables sharp-canvas-first scheduling and incremental fitted previews, reproducing the previous scheduling/render policy. The normal run enables both. The earlier native-scheduling-only comparison remains in `output/feedback-profile/{before,after}/`; the first combined comparison is retained in `output/feedback-profile/final/{before,after}/`. The final verified comparison, including the faster overview warmup, is stored separately in `output/feedback-profile/verified/{before,after}/`.

Settling requires a completed accepted native region matching the current canvas tile, with no pending region request, so a partial old-crop fallback during a pan cannot count as warm. Moving Patch feedback must match the current lasso boundary as well as its donor offset; committed feedback must match the latest saved patch instruction.

`input_to_gpu_complete_ms` starts before the edit-producing movement is delivered and ends after the updated canvas frame completes. `input_to_canvas_ready_ms` subtracts the final tessellation/upload/GPU stage from that total; it is an estimated UI acceptance interval, not an independent worker timestamp. The GPU stage uses a synchronous completion wait for this diagnostic only. Replays poll frames between worker completions and exclude OS input delivery, display scanout, compositor delays and physical monitor refresh. These are local integration timings, not a promised frame rate.

## Correctness

Complete final canvas screenshots are compared between the two policies. Unit checks compare every incremental fitted tool update with its complete render, including moved Patch donors, cancellation and reset/undo. Native short-click UI checks require the sharp frame to submit before the overview; existing held-pointer, sharp-frame retention, save and GPU-canvas tests remain in the suite. Lower-layer visible-sampling tests require distant destination pixels to update after an upper edit.

## Verified results

Measured on Intel Core i3-1215U / Intel UHD Graphics, Vulkan, using the optimized release build with `CARGO_PROFILE_RELEASE_LTO=false`. The table gives ordinary medians of eight samples per gesture, in milliseconds from edit-producing input to completed offscreen GPU frame. Held and short gestures are separated because the previous native policy already prioritized held input.

| Tool | Fit held, before → after | Fit short, before → after | 90% held, before → after | 90% short, before → after |
| --- | ---: | ---: | ---: | ---: |
| Liquify | 95.06 → 26.55 | 97.01 → 29.35 | 18.60 → 19.36 | 17.08 → 18.97 |
| Spot Heal | 271.19 → 46.13 | 159.78 → 51.62 | 34.30 → 33.37 | 183.87 → 44.14 |
| Clone Stamp | 255.95 → 43.11 | 147.12 → 43.94 | 32.15 → 30.20 | 170.21 → 39.63 |
| Patch | 122.27 → 17.09 | 136.80 → 33.59 | 21.27 → 20.30 | 147.12 → 18.73 |

Pooling the sixteen fitted samples gives median reductions of 70.7%, 76.1%, 78.3% and 82.5%, respectively. Native Liquify and held repairs show little consistent change; the major native saving is short/released repair feedback. `comparison.json` contains every tool/view/gesture median and nearest-rank p95, including 62% and 100%.

Initial cache preparation still causes outliers. The largest fitted samples were Liquify 105.94 → 103.73 ms, Heal 362.51 → 266.27 ms, Clone 271.62 → 166.46 ms and Patch 147.73 → 157.56 ms. The first Liquify regression exposed by the intermediate region-only warmup (321.60 ms) is removed. The remaining Patch maximum increased slightly, while later feedback improved substantially. With only eight/sixteen samples, the nearest-rank p95 is the maximum; these are small local measurements with desktop timing variation, not a broad latency distribution or universal first-stroke guarantee.

All sixteen complete final screenshots, across four tools and four views, are pixel-identical between policies. `parity.json` records dimensions and PNG hashes. The 209 optimized library tests passed with ONNX/Burn enabled, strict release all-target Clippy passed, and the executable/profiling helper rebuilt successfully. Native portrait review separately verified under-eye localization and direction on five fixtures; [layer panel captures](workspace-panels.md) verify masks and sampling controls.

## Reproduce

Stop compilation and other heavy work before collecting timings. In PowerShell:

```powershell
$env:CARGO_PROFILE_RELEASE_LTO='false'
cargo build --release --locked --example feedback_latency
$portrait='C:/Users/subop/AppData/Local/Temp/codex-clipboard-0e8e78eb-8ef8-4b95-bdc8-c8abf4a980d9.jpg'
./target/release/examples/feedback_latency.exe $portrait output/feedback-profile/verified/before --baseline
./target/release/examples/feedback_latency.exe $portrait output/feedback-profile/verified/after
```

The temporary original path must still exist; another local portrait can be supplied for a fresh comparison. CSVs include every sample and the adapter name is recorded in `results.txt`. The helper is not called by the normal application render loop. See [display/GPU stage profiling](render-performance.md), [live tools](live-tools.md) and [editable layer masks](workspace-panels.md).
