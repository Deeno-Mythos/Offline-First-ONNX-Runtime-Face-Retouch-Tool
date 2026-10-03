# Project performance and workflow

The project keeps bounded, lossless, color-managed thumbnails for inactive photos. The selected photo is opened by an independent original-loader thread. Switching photos cancels obsolete requests, drops inactive original/working pixels and generated AI maps, and clears source/filter, native tile, hover, patch, undo-preview and GPU caches. Edits, masks, ratings, view settings and undo/redo instructions remain in the project.

The filmstrip uses thumbnails capped at 512 pixels on the longest edge. EXIF orientation and embedded RGB ICC profiles follow the same sRGB conversion as native import. Thumbnail cache keys include the source path, size, modification time and decoder version; the lossless PNG cache is bounded to 256 MiB. Source files are never modified.

On Windows, the WIC scaler requests reduced pixels through a decoder pipeline. Other platform codecs can require a temporary decode while creating a thumbnail; these pixels are not retained in the project. Active editing and native export always open the original, never the thumbnail. A failed native load shows a retry action and blocks canvas painting.

| Objective | Behavior |
| --- | --- |
| Deferred loading | Import/recovery stages metadata and thumbnails; the active photo alone opens native pixels. AI preparation only follows active enabled adjustments. |
| Brush cache and overlays | Existing bounded preview/native brush caches remain independent of import/export. Mask overlays, layer-thumbnail refreshes and new AI preparation are suppressed during brush gestures. Small cursor/source guides remain visible. |
| Delete from Project | Right-click a filmstrip photo. Removes its edits/history/asset record, cancels its requests, releases memory and owned thumbnail/AI scratch files. Autosave immediately records the removal, including an empty project. Original source files remain on disk. |
| Batch import | File → Import photos or Import folder; folders may also be dropped on the app. Staging runs in the background, keeps the active selection stable and allows editing before completion. Folder staging reads supported files directly in the selected directory; the project limit remains 64 photos. |
| Batch export | File → Export queue shows independent jobs and cancellation. Jobs retain edit/settings snapshots across project changes, load originals as needed and prepare required AI. Print, Web and Cutout presets select format, size and quality. Preferences → Export permits one or two simultaneous jobs; one is the default to limit memory. Export pixel workers are bounded and release their AI weights after each job. |
| Vector toolbar | Liquify, healing, clone, patch, undo, redo, export, layers/history and the existing tool rail use vector icons with accessible names. Global Preferences → Workspace → Show hover tooltips controls hover help. |
| Shortcuts manager | Preferences → Shortcuts records single keys or modifier combinations for tools, file/view commands, built-in presets, project removal and layer actions. Conflicts are rejected; Clear and Default are available per command. Bindings persist with preferences and replace defaults in execution/menu help. Layer commands remain scoped to the Layers panel. Typing/modal input is protected; Space, Tab, backslash and Escape retain navigation/cancel behavior. |
| Smart proxy/native fidelity | The hardware viewport retains its DPI/zoom-dependent native region rendering. Manual sampling and export use uncompressed, oriented native pixels. Thumbnails are filmstrip/loading representations, not brush sources. Stale previews cannot release a newer render or overwrite a newly selected/deleted asset. |

## Validation

`cargo test --release --locked --lib -- --test-threads=2` includes real egui input tests for deferred activation, stable selection during import, deletion/late results, empty recovery, key recording/conflict rejection/persistence, global tooltips, live brushes, native zoom and sharp saved previews. Asset tests cover JPEG, PNG, TIFF, BMP and WebP thumbnails, cache invalidation, exact lossless native reopening/export and source preservation. Export worker tests cover independent projects/formats and continued queue processing after cancellation.

`cargo run --release --locked --example asset_pipeline -- "C:\Users\subop\OneDrive\Desktop\DAY 2"` performs a read-only 24-portrait staging/residency audit and writes `output/asset-pipeline/report.txt` plus an oriented thumbnail contact sheet. Retained pixel-byte measurements exclude unrelated process/runtime/GPU allocations.

`cargo run --release --locked --example live_tools_performance` exercises growing liquify/heal/clone strokes through the dedicated overview and native-region caches. These CPU render timings are not end-to-end cursor latency measurements.

The trained DAY 2 model and its validation reports remain separate from project loading. See [paired-training.md](paired-training.md) for the 38-pair split, protected regions and held-out results.

The final 2026-10-02 release regression run passed all 189 tests (`output/workflow-tests-final.log`); all-target Clippy passed with warnings denied (`output/workflow-clippy-final.log`). Recording controls and the Preferences footer are checked for stable positions while the pointer approaches. Auto Retouch keyboard execution is tested for preserved contrast/exposure/face shape and undo. Layer binding replacement is checked against its disabled former default and text-input/panel scope. All 76 DAY 2 source hashes and the installed trained generator checksum were verified unchanged.

## Measured batch and brush results

On the 24 DAY 2 originals, staging took 1.492 s, cached staging 0.112 s, and opening the first native original 0.586 s. The photographs are 3840×5760. Staged RGBA allocations totaled 16,760,832 bytes (15.98 MiB), compared with 2,123,366,400 bytes (2025 MiB) for all native sources: a 99.21% reduction in retained source-pixel allocations. This excludes working previews, UI/runtime/GPU allocations, caches and explicit masks. The helper verified that releasing the active source destroyed its native Arc. See [batch report](../output/asset-pipeline/report.txt).

The real WGPU app opened all 24 neutral photos with one selected native original and captured the workspace successfully. Whole-process Windows peak working set was 620.50 MiB in that run; this includes startup, UI, runtime and driver allocations. There is no measured prior whole-app baseline, so the source-pixel reduction is not a claim of a 99% process-memory reduction. The [workspace capture](../output/asset-pipeline/batch-workspace.png) and [Shortcuts Manager](../output/asset-pipeline/shortcuts-manager.png) were visually reviewed.

The growing-stroke helper compared its final PNGs with the existing references; all six results were byte-identical. The warmed timings below use the same 960×1440 overview and 512×512 native region described in [live-tools.md](live-tools.md). AI was inactive. These are CPU rendering times, not end-to-end display latency or a guaranteed frame rate. Tail latency varies, notably the native clone p95 in this run.

| View | Tool | Median | p95 |
| --- | --- | ---: | ---: |
| Overview | Liquify | 18.93 ms | 35.47 ms |
| Overview | Spot heal | 23.04 ms | 26.42 ms |
| Overview | Clone stamp | 22.05 ms | 24.11 ms |
| Native region | Liquify | 16.14 ms | 17.81 ms |
| Native region | Spot heal | 29.11 ms | 40.44 ms |
| Native region | Clone stamp | 22.04 ms | 77.15 ms |

Detailed timings and exact-reference verification are recorded in `output/live-tools/workflow-validation.log`. The release app rebuild is recorded in `output/workflow-build-final.log`.

The final rebuilt app refreshed five captures: the 24-photo workspace, desktop/600×560 Shortcuts Manager, export presets and 100% native Clone workspace. Each exited with code 0 and empty stderr; the batch, native view and dialogs were visually reviewed. Staged photos no longer display a misleading pending-render label. Formatting and diff whitespace checks passed, and all 76 training-source hashes were checked again after these launches.
