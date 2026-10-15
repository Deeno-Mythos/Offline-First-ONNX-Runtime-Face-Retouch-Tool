# Portrait and batch workflow upgrades

Implemented user goal: better lighting manipulation; −/+ brush sizing for every tool; Liquify performance; clothing crease correction; stray hair removal; skin refinement; facial wrinkle reduction; unified skin tone; face focus scoring; selection-scoped batch editing.

All ten requirements have controls and direct validation below. Edits retain source files, history, layers and sessions; focus scores are runtime review data and are recomputed after reopening. This implements a local portrait and batch workflow, without claiming full Evoto feature parity.

| Requirement | Implementation and direct evidence |
| --- | --- |
| Lighting | Whites/Blacks tonal endpoints and painted Dodge/Burn with EV, size, softness, strength and erase. Neutral identity, alpha, tonal scope and local coverage tests; 50/100 portrait effects; native/full parity; installed undo/masks/session/PNG checks. |
| Brush −/+ | Minus, Plus, shifted Equals and brackets use native-pixel limits, with brush sizing taking priority over custom commands. All 14 targets tested for size, modal/text ownership and view stability. New manual strokes retain their sizes; Space pans. Patch uses saved feather width, including legacy-session fallback and transform projection. |
| Liquify performance | Skips inactive brushes, square-footprint corners and unneeded face projection; ignores zero-strength tiny strokes when selecting the displacement grid; reuses geometry across AI readiness changes when shape controls are neutral. Ordered mapping and growing-stroke pixel parity tests plus production input-to-GPU timing replay. Timings and variability are reported below. |
| Clothing wrinkles | Dedicated painted fabric correction reduces medium-frequency crease shading while retaining fine weave and broad drape. Strength/erase/layer/history tests, local pixel protection, native/export agreement, and visual review on blue cotton and pinstripes. Manual selection; avoid seams and printed patterns. |
| Stray hairs | Dedicated strand brush uses opposed donor agreement and elongated continuity inside painted regions. Tests reject broad edges and isolated spots, preserve transparency, support erasing, and follow layer visibility/cache changes without loading AI. Native portrait checks and installed session/export checks passed. Existing conservative automatic exterior cleanup remains available. |
| Skin refinement | Keep skin detail controls high-frequency texture retention during neural/frequency smoothing. Grain-retention, serialization and native-region tests; three portrait reviews at 50/100; protected facial features and texture visually inspected. |
| Facial wrinkles | Forehead, laugh-line and under-eye controls use the existing mesh-localized neural/frequency pipeline. Three portraits show stronger effects at 100 than 50, native-region agreement and lossless combined exports. No global contrast or identity change is added by facial cleanup; deep lines can remain. |
| Unified skin tone | Choose a source face's median skin chroma; match this face, other faces or selected portraits while preserving perceptual lightness and fine detail. Explicit skin masks cover neck/body as needed. Oklab reference values, gamut handling, invalid-target safety, disable/serialization/native tests and mask-exclusion checks passed. |
| Face focus scoring | Native-source face and eye scores, with the least-focused measurable face determining a group score. Background outside detected regions is excluded. Geometry-only worker requests, staged native loading, selection/cancellation/stale-result tests, no-face/tiny states and measured blur ordering passed. Ratings can mark up to five selected picks; expressions need human review. |
| Batch editing | Portrait / Light + color / Background selective sync, optional bounded exposure adaptation, Auto Retouch selected, AI progress/pause/resume, face review/rating and existing queued exports. Tests preserve local strokes, geometry, excluded adjustments and unselected photos; unopened selected photos queue their own source/maps. Workers stage one source at a time and keep manual feedback independent. |

## Validation

The subsequent clothing/stray-hair correction is documented in [painted brush repair](painted-repair.md), with 267 passing tests and measured defect-reduction checks. Its new installer supersedes the earlier workflow package listed below.

`cargo test --release --locked --lib -j2`: **260 passed**. `cargo clippy --release --all-targets --locked -j2 -- -D warnings`, formatting and diff checks passed. Release LTO was disabled for the local build, as in prior qualification.

- Cross-path assertions: `src/workflow_tests.rs`, `src/ui_tests.rs`, `src/worker.rs`, and the focused lighting/skin/focus/cleanup/geometry tests.
- Native image review: `examples/workflow_review.rs`; results and images in `output/workflow-upgrades/portrait-review/`. Two generated sources and the bundled portrait were analyzed with CPU ONNX. Every tested effect increased from 50 to 100. Region/full renders differed by at most one byte; original-size PNG exports were identical to the rendered pixels. Original file bytes stayed unchanged. A 6.3MP upscaled stress fixture also passed tiled/native parity.
- Skin matching mean Oklab lightness error was 0.000552 / 0.000545 / 0.000569 across the three portraits, including output quantization. Pixels outside the skin mask were unchanged.
- Sharp/moderate/heavy blur focus scores were 69.69/39.54/18.67, 65.08/43.74/20.76 and 64.93/42.05/20.81. These are relative detail scores, not calibrated probabilities of a usable photograph.
- Packaged application: eight desktop/compact native screenshots in `output/workflow-upgrades/native/`; CPU installation check passed all eight manual tools, exact undo/redo, layer masks, saved history, sessions, lossless PNG and recovery fallback. Screenshot startup also used the locally qualified DirectML provider.
- Installer install/uninstall passed with all **47 payload hashes** verified. This was the development machine, not a clean Windows host. Physical mouse/display latency was not measured; application input was replayed through egui and the actual workers/GPU canvas.

## Performance measurements

The replay uses the generated 1122×1402 source, Intel UHD / Vulkan, eight held and eight short gestures per tool/view. It measures application pointer input to completed offscreen GPU frame; OS event delivery and physical refresh are excluded. New clothing, Dodge, Burn and hair tools also produced completed frames during held gestures at Fit, 62%, 90% and 100%.

| Liquify view | Before: initial / repeat median (ms) | After: initial / repeat median (ms) |
| --- | ---: | ---: |
| Fit | 13.94 / 12.09 | 15.72 / 14.20 |
| 62% | 17.10 / 20.62 | 15.75 / 19.46 |
| 90% | 18.45 / 23.81 | 19.65 / 21.59 |
| 100% | 15.35 / 22.92 | 16.79 / 15.91 |

The repeat run improved zoomed medians; Fit and some tails varied or regressed. These measurements do **not** establish a universal speedup or continuous 60 fps. Current Liquify p95 ranged roughly 44–70 ms in the repeat run; the new brushes had roughly 35–49 ms medians at 100%, with cold/tail frames up to approximately 119 ms. First blur/mask preparation can still stutter. Growing-stroke Liquify/Heal/Clone final pixels remained byte-identical to their saved pre-change references. Direct-render timings varied upward too; those CSVs are retained, rather than reported as gains. A run overlapping inference is labeled separately and excluded from comparisons.

Raw evidence: `output/workflow-upgrades/feedback-before*/`, `feedback-after*/`, and `liquify-before/`. The baseline executable is preserved locally under `baseline-bin/`. Performance is specific to these fixtures and this hardware.

## Replacement images and package

At the user's request, generated two fictional adult portrait sources and edited visual references, with full prompts and hashes in `output/synthetic-fixtures/`. These are PNGs, not camera RAW. Generative references can shift small details and require registration/target validation before supervised training. **No model was retrained on these pairs.** They stay outside the installer. See [fixture notes](../output/synthetic-fixtures/README.md).

Windows x64 installer qualified for this workflow pass: `output/beta/0.1.0-beta.1/20261014-013745-571/artifacts/Hastur-Retouch-0.1.0-beta.1-windows-x64-setup.exe`. The subsequent size/cache improvement is documented in [AI efficiency](ai-efficiency.md).

SHA256: `51dbd990d465855c8f1e6404e2ffa24e191afa82b8512f4232e85d8a60bec5d5`; 506,329,331 bytes. The installer is unsigned. It was built from the current working tree based on commit `50522823266c699cb23c7829cd08219371a74a5f`, which has uncommitted changes. No GitHub release or tag was changed. Clean-PC and physical-input testing remain release-qualification limits; the requested implementation and local validation are complete.
