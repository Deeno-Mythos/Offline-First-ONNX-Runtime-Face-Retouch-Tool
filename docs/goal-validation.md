# Goal implementation and validation

This report tracks the nine requested workflow improvements. The integrated version passed all 105 regression tests in both debug and optimized release builds. Fresh facial-retouch review, backend comparison, 22.1/60 MP rendering, scalar/growing-history and color measurements, smoke tests, and native UI review are complete. The stricter flyaway exterior-support guard passed six focused tests, all-target clippy, and its optimized rebuild; repeated full-suite and native QA remain pending as of 2026-10-01.

| Requested improvement | Implemented behavior | Final verification |
| --- | --- | --- |
| Autosave and recovery | Background saves, previous-generation fallback, restored undo/redo and workspace state | Session/UI regressions passed. See [recovery workflow](workspace-recovery.md). |
| High-resolution rendering | 512-pixel tiles with filter halos and global coordinates; padded native viewport rendering | Parity, region reuse, stale pan rejection, and cancellation tests passed. Complete rendering measured 3.041 s at original 22.118 MP and 15.782 s at upscaled 60.005 MP; both original-size exports and native parity passed. |
| Varied portrait review | Native eye/face/hair crops and masked under-eye strength checks on five portraits | Fresh facial-retouch review passed: original dimensions, no masked under-eye spill or facial-only exterior changes, and stronger 100% than 50%. Eyes, glasses, lashes, and stubble remain coherent. See [portrait validation](portrait-validation.md). |
| Long undo histories | Shared edit instructions, 100-step history and soft 64 MiB instruction budget | Fresh 100,000-stroke/100-step scalar benchmark measured 0.098 µs record, 0.209 µs undo, 0.027 µs redo and 1.91 MiB shared storage. Growing 100,000→103,200 stamps measured 0.935 ms median per 16-stamp gesture; the budget retained 16 steps at 62.90 MiB. These operations exclude rendering. |
| Brush alignment and response | Source-coordinate strokes, DPI-aware cursor, Space-drag pan, progressive gesture previews; Escape/undo/redo end active strokes | All 80 raw-egui cleanup cases passed at five zooms and four DPI scales, plus canvas containment and interrupted-stroke tests. |
| Burn versus ONNX Runtime | Both bundled-model backends available; ONNX remains the default | Fresh isolated comparison complete: 5.166 s ONNX versus 119.110 s Burn warm, with lower Burn peak memory and close rendered output. See [backend measurements](portrait-validation.md). |
| Flyaway cleanup | Explicit strength, detected-head halo, protection for facial skin and overlapping faces; regional exterior-background evidence guards coherent hair | Six focused tests passed, including seven real internal-hair points left unchanged and four genuine exterior points still corrected. The full release suite and repeated five-portrait native QA remain pending. Complex backgrounds may need manual repair. |
| HSL and curves | Eight hue sectors, luminance/RGB curves, independent enable/amount controls and history | Color/UI tests passed. See [color workflow](color-workflow.md). |
| Batch reference matching | Embedded reference profiles, independent source measurement, selected-photo synchronization | Profile/color/UI tests passed. Ten real-photo source-profile comparisons completed, with the largest sampled RGB-average difference 1.1608 bytes at 100% matching. This isolates profile/resampling consistency, not perceptual batch quality. |

## Reproducible checks

The integrated version passed formatting, `git diff --check`, and [clippy on all targets](../output/performance/final-clippy.txt). Both complete suites passed **105 tests, 0 failures**: [debug](../output/performance/final-debug-tests.txt) and [optimized release](../output/performance/final-release-tests.txt). Release execution took 15.75 seconds excluding compilation. Debug execution occurred during the release build and is not used as a performance measurement. The additional crown/exterior-support refinement passed six focused tests and [all-target clippy](../output/performance/final-exterior-clippy.txt); final full-suite and native rechecks remain pending.

The 80 raw-egui cases cover 62/90/100/200/400% zoom × 100/125/150/200% DPI × Liquify/Spot heal/Clone stamp/Patch. A separate isolated execution of the final optimized binary measured complete stroke gestures at a median of 1.26 ms and p95 of 1.86 ms, with no compiler or other heavy process running; all cases passed. These measurements cover UI event and gesture processing across several frames, not retouch rendering or display latency. See [final isolated brush-response log](../output/performance/final-isolated-brush-response.txt).

The app and benchmark build succeeded with optimized release code (`opt-level=3`) and `CARGO_PROFILE_RELEASE_LTO=false` for the entire dependency graph. The source release profile is unchanged. This avoids the lengthy link-time compilation on this computer. See the [initial optimized build](../output/performance/optimized-build.txt) and [final exterior-guard rebuild](../output/performance/final-exterior-guard-build.txt). Fresh benchmark artifacts are linked below.

The [smoke test](../output/performance/smoke.txt) verified three full-resolution 1122×1402 preset exports in 0.34 s with the original unchanged. The [real-model AI smoke results](../output/ai-validation/results.txt) verified increasing 50/100% effect on eleven portrait sliders, plus a 2244×2804 (6.29 MP) source-size PNG combining AI, warp, and clone corrections; the decoded export exactly matched the renderer and the source remained unchanged.

Native screenshot review found sharp 100% source pixels, preserved aspect ratio, readable controls, and no overlap in the desktop color and 800×760 compact layouts: [native portrait comparison](../output/screenshots/portrait-native-final.png), [desktop color panel](../output/screenshots/color-final.png), and [compact color navigation](../output/screenshots/color-compact-final.png). All three captures use the final rebuilt app and exited successfully. Compact navigation uses bottom tabs and opens tools when selected; flyaway cleanup was disabled in these Auto Retouch captures.

```powershell
cargo fmt --check
cargo test --locked --lib -j 2 -- --nocapture
$env:CARGO_PROFILE_RELEASE_LTO = 'false'
cargo build --release --locked -j 2 --bin astra-retouch --example runtime_validation --example high_resolution --example history_performance --example color_consistency --example smoke --example ai_smoke
cargo test --release --locked --lib -j 2 -- --nocapture
.\scripts\validate-portraits.ps1 -SkipDownload -SkipBuild -Backends
target/release/examples/history_performance.exe
```

The portrait script verifies fixture hashes and licenses are recorded in [portrait-validation-sources.md](../scripts/portrait-validation-sources.md). `-SkipBuild` requires the prebuilt executable and preserves the optimized helper's build flags; without it, the script performs a release build using the current Cargo profile and environment. `-SkipDownload` requires the fixture files to be present and still checks their hashes. `-Backends` repeats the CPU/Burn comparison. Large-image measurements use `high_resolution` with an original photograph, the fresh portrait-analysis directory, an output directory, and optionally a target megapixel count. The 60 MP case is an upscaled stress fixture, not a second camera original. Measure in isolation after compiler and other heavy processes stop.

Original and final 8-bit image buffers remain resident; tiling bounds filtering buffers rather than making memory independent of photograph size. Backend measurements cover one fixed inference fixture on one machine. Native portrait review checks visible behavior on the documented set and does not demonstrate new training or universal model quality.

The fresh backend comparison used identical demo input, two actual inference runs per isolated process, and the LTO-disabled optimized build after compiler processes stopped. ONNX measured 4.249 s cold and 5.166 s warm, with peak working sets of 1,636.99/2,350.07 MiB. Burn measured 136.945/119.110 s, with peaks of 1,364.94/1,413.95 MiB. Burn was about 23.1× slower warm on this run. The actual Auto Retouch overviews differed at 7 of 1,573,044 pixels, at most one byte per channel; mean absolute byte difference was `1.11249e-6`. The outputs are close, not identical. See [portrait validation](portrait-validation.md) for the per-model measurements and fixture limits.

## Large-image measurements

The isolated render helper uses precomputed portrait maps without resident inference models. Peaks are whole-process Windows working-set peaks, including originals, image-loading buffers, viewport preparation, and export output; they do not describe the full application's combined AI/render memory. There is no measured older-renderer baseline for these large photographs.

| Case | Native viewport cold / warm median | Under-eye slider median | Complete render / peak | JPG export including rerender / peak |
| --- | ---: | ---: | ---: | ---: |
| Original 3840×5760, 22.118 MP | 423.75 / 182.06 ms | 155.63 ms | 3.041 s / 211.44 MiB | 3.786 s / 281.91 MiB |
| Upscaled 6325×9487, 60.005 MP | 540.38 / 474.77 ms | 485.23 ms | 15.782 s / 884.58 MiB | 19.217 s / 884.58 MiB |

The viewport is 1280×900 source pixels. For both cases, five warmed requests were byte-identical and comparison against the complete render differed by at most one byte per channel. Exports retained each benchmark image's exact dimensions. The 60 MP peak was already reached during Lanczos3 upscaling before rendering; it is a cumulative high-water mark rather than render-only additional memory. See [original-resolution results](../output/high-resolution/original/results.txt) and [60 MP stress results](../output/high-resolution/60mp/results.txt).

## History and reference consistency

The history benchmark records scalar setting changes with unchanged shared stroke vectors, then traverses all 100 retained undo and redo steps. It measures edit-instruction operations without image rendering, file I/O, or source-image memory. Mutating action vectors can allocate new storage; the regression suite separately verifies budget trimming and preserved nearest history steps.

| Strokes / retained steps | Record | Undo | Redo | Shared stroke storage |
| --- | ---: | ---: | ---: | ---: |
| 1,000 / 100 | 0.069 µs | 0.189 µs | 0.024 µs | 20,000 bytes |
| 100,000 / 100 | 0.098 µs | 0.209 µs | 0.027 µs | 2,000,000 bytes (1.91 MiB) |

See [history.csv](../output/performance/history.csv). Native rendering timing is reported separately above.

A second benchmark adds 16 new stamps per gesture over 200 gestures. These changes allocate new action vectors, so timing includes copy-on-write and budget trimming. Undo/redo restored the exact expected stamp counts and instruction storage stayed below 64 MiB.

| Initial → final stamps | Retained steps | Gesture median / p95 | Undo / redo | Retained history instructions |
| --- | ---: | ---: | ---: | ---: |
| 1,000 → 4,200 | 100 | 0.031 / 0.055 ms | 3.210 / 3.962 µs | 12.88 MiB |
| 100,000 → 103,200 | 16 | 0.935 / 1.113 ms | 1.269 / 0.756 µs | 62.90 MiB |

The 100,000-stamp case retains fewer steps because each changing vector occupies new storage. Current-edit storage and rendering are excluded from the history figure. See [history-growing.csv](../output/performance/history-growing.csv) and [benchmark run](../output/performance/history-final-run.txt).

Reference consistency compares profiles prepared from each complete preview and complete original, applying both to the same original sample pixels with that original as the identity target. Ten cases cover five photographs at 65% and 100% matching on a normalized 128×128 grid. This isolates source-statistics/resampling differences; it is not a full exported-image or perceptual batch-match score.

| Source at 100% matching | Mean RGB-average byte difference | p95 | Maximum sampled RGB average |
| --- | ---: | ---: | ---: |
| Demo, NASA, and Grace Hopper (no preview resize) | 0 | 0 | 0 |
| William Stitt macro portrait | 0.1694 | 0.4753 | 0.6135 |
| Supplied 22.118 MP portrait | 0.2597 | 0.6363 | 1.1608 |

See [color-consistency.txt](../output/performance/color-consistency.txt) and [color workflow](color-workflow.md). Statistical reference grading cannot guarantee semantic skin-tone or scene matching.
