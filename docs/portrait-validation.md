# Portrait quality and CPU backend validation

Measured locally on 2026-10-01: Intel Core i3-1215U (6 cores / 8 logical processors), 16 GiB RAM, Windows, optimized release build (`opt-level=3`, whole-profile `CARGO_PROFILE_RELEASE_LTO=false`). Compiler processes were stopped before the inference comparison. These measurements describe this computer and these installed models.

## ONNX Runtime versus Burn

Both backends analyzed the identical 1122×1402 bundled demo in isolated processes, running all five bundled models. AI output cache was bypassed. Each process ran twice, retaining its model sessions for the second run. Cold timings include weight loading and any backend compilation; warm timings still perform actual inference. Peak working set is the native Windows process peak, including weights and intermediate inference buffers.

| Backend | Cold complete AI | Warm complete AI | Cold peak working set | Peak after second run |
| --- | ---: | ---: | ---: | ---: |
| ONNX Runtime CPU | 4.249 s | 5.166 s | 1,636.99 MiB | 2,350.07 MiB |
| Burn CubeCL CPU | 136.945 s | 119.110 s | 1,364.94 MiB | 1,413.95 MiB |

Burn's warm run was 23.1× slower on this computer. Burn consumed less peak process memory here, but its latency makes ONNX Runtime CPU the practical default. Burn remains an explicit selectable backend. “Warm” means model sessions were reused; it does not guarantee a shorter measured time on a desktop with variable system load.

Numerical output comparison checked skin, under-eye, eye, blemish, neural blend and repair maps. The neural blend map's mean absolute difference was `1.05e-7` and maximum `1.41e-5`. An isolated segmentation threshold crossing produced a larger blemish-mask difference (maximum `0.177`).

The fresh integrated-renderer Auto Retouch overview comparison completed at 20:12. It included the under-eye dark-pixel correction: the 1122×1402 outputs differed at only 7 of 1,573,044 pixels, with a maximum 1-byte channel difference and overall mean `0.00000111` bytes. This establishes close output agreement on the measured fixture, not equivalence on every portrait.

Artifacts: `output/portrait-validation/backend-{cpu,burn}/results.txt`, `maps.bin`, `auto-overview.png`, and `output/portrait-validation/backend-comparison.json`.

Reproduce after building:

```powershell
$env:CARGO_PROFILE_RELEASE_LTO = 'false'
cargo build --release --locked --example runtime_validation
$env:HASTUR_AI_TIMINGS = '1'
target/release/examples/runtime_validation.exe cpu assets/demo-portrait.png output/portrait-validation/backend-cpu 2
target/release/examples/runtime_validation.exe burn assets/demo-portrait.png output/portrait-validation/backend-burn 2
.ai-tools/Scripts/python.exe scripts/compare-ai-backends.py output/portrait-validation/backend-cpu output/portrait-validation/backend-burn output/portrait-validation/backend-comparison.json
```

The comparison script requires NumPy and Pillow (already installed in this workspace's AI tooling environment).

## Varied native-resolution portrait review

Fixtures and licenses are recorded in [portrait-validation-sources.md](../scripts/portrait-validation-sources.md). The set includes the existing studio demo and supplied portrait, NASA's Eileen Collins portrait, the U.S. Navy's Grace Hopper portrait (glasses and fine wrinkles), and William Stitt's CC0 macro portrait (strong under-eye shadows and facial hair). No new training or model-quality claim is based on this visual-review set.

Local fixture SHA-256 values are `c794fcf6f62b868484dd52b8ecd7e204f693ead64c23ba3bb45bb47a982c1ba4` for the bundled demo and `bb4855db2bb4e5ab6879a20c19b909fdeea4756a36f49615b99c4022b83627ea` for the supplied JPEG. External fixture hashes are recorded in the source manifest and checked by the validation script.

The final integrated release fixture run completed at 20:55–20:57 on 2026-10-01, using `opt-level=3` and whole-profile `CARGO_PROFILE_RELEASE_LTO=false`. Native left/right eye, hair-edge and head-halo outputs were checked without resizing, using original image detail and byte comparisons; the final flyaway review completed on 2026-10-02. The full-resolution source images were used for the retouch renders; the smaller analysis overview was used only for inference.

All five fixtures retained their exact source dimensions. Facial-only Auto Retouch, under-eye 50/100 and skin 100 had zero changed pixels outside expanded face bounds. Both under-eye settings had zero changed pixels outside the quantized under-eye mask, and 100 produced a larger overall pixel effect than 50. Auto Retouch left contrast and exposure at zero.

| Fixture | Native dimensions | Confident under-eye mean brightness change, 50 / 100 | Native visual findings |
| --- | ---: | ---: | --- |
| Studio demo | 1122×1402 | +4.37 / +8.35 bytes | Local under-eye cleanup; eyelashes remain distinct. Skin 100 visibly softens cheek texture while retaining fine detail. |
| Eileen Collins | 512×512 | −0.41 / −0.80 bytes | Changes are subtle in the small native eye crops; eyelid and eye shape remain coherent. This fixture does not demonstrate under-eye brightening. |
| Grace Hopper | 512×600 | +3.53 / +6.76 bytes | Under-eye lines soften while the glasses frames and eye detail remain coherent. |
| William Stitt | 5184×3456 | +6.03 / +11.24 bytes | Strong under-eye shadows visibly lift with eyelashes and skin texture retained. A separate native lip/stubble crop retains facial hair and lip texture under Auto Retouch and skin 100. |
| Supplied portrait | 3840×5760 | +3.11 / +6.00 bytes | Under-eye bags soften and lift while eyelid/iris detail remains intact. Auto Retouch, under-eye 100 and skin 100 preserve the dark skin/hair boundary and individual strands in the previously problematic area. |

Brightness deltas are measured RGB-byte luminance changes within pixels whose under-eye mask is at least 0.2; they are not exposure adjustments or a perceptual quality score. The astronaut's slightly negative result shows that stronger under-eye smoothing does not guarantee brightening on every photo. The native checks establish registration and localized effect, while the observed quality remains specific to these fixtures.

Each fixture's `output/portrait-validation/<name>/results.txt` contains the counts, timing and mask-scope checks. The eye and hair artifacts are `left-eye-<case>-native.png`, `right-eye-<case>-native.png` and `hair-edge-<case>-native.png`. William Stitt's supplemental 640×480 lip/stubble review crops are in `output/portrait-validation/review/beard-<case>-native.png`; `beard-crop.txt` records the crop coordinates. A 501×768 crop centered on face-oval landmark 356 specifically checks the supplied portrait's previously circled temple/cheek hair boundary, in `review/supplied-temple-<case>-native.png`; `supplied-temple-crop.txt` records its coordinates. Auto Retouch, under-eye 100 and skin 100 retain that edge and overlapping wisps without the previously reported smear. These supplemental images are untouched rectangular samples of the native face renders, with no resizing.

## Final flyaway cleanup review

The detector requires agreeing background donors, broad background support around the candidate, and majority color-family agreement with the outer head-halo boundary. These guards prevent interior hair reflections from being treated as background. The final native review found no remaining visible hair speckling or contour smearing in these fixtures.

| Fixture | Changed pixels at flyaway 100 | Maximum channel difference | Final head-halo result |
| --- | ---: | ---: | --- |
| Studio demo | 76 | 65 bytes | Small exterior-strand segments lighten toward the gray background; coherent hair reflections are retained. |
| Eileen Collins | 0 | 0 | Native head halo is byte-identical to the original. |
| Grace Hopper | 0 | 0 | Native head halo is byte-identical; glasses, hat and textured backdrop are preserved. |
| William Stitt | 0 | 0 | Native head halo is byte-identical, including facial hair. |
| Supplied portrait | 14 | 14 bytes | A few exterior strand pixels above and beside the head lighten toward the studio backdrop; the temple/hair boundary stays intact. |

All seven specifically reported internal demo hair/reflection pixels now have zero byte difference, and four visually verified exterior-strand pixels retain nonzero corrections. The demo's entire saved right hair-edge crop is byte-identical. Flyaway-only left/right eye crops are byte-identical for all five fixtures. The refreshed supplemental supplied-temple and William Stitt lip/stubble flyaway crops are also byte-identical to their originals. The full eligible head-halo artifacts are `head-halo-{original,flyaway-100}-native.png`; `output/portrait-validation/review/final-native-qa.json` records dimensions, counts and the seven/four regression points.

Flyaway cleanup remains a conservative local strand heuristic guided by detected heads, not a semantic hair model or a retrained network. Its verified improvement here is partial thinning of a few exterior strand segments; many wisps remain even at 100, and three fixtures intentionally produce no change. Complex backgrounds, weak contrast and large hair locks still need the manual cleanup tools. These native results support safe localized behavior on this set, not automatic removal of every stray hair or quality across every skin tone, pose and background.

Reproduce native validation with the release build:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-portraits.ps1
```

Use `-SkipDownload` when the three hash-verified external fixtures already exist locally. Use `-SkipBuild` to validate an already-built helper with its existing build settings. Add `-Backends` only to repeat the CPU/Burn inference benchmark. Each native fixture directory contains `results.txt`, `segmentation.ron`, raw inference maps, unresized original/Auto Retouch/under-eye 50 and 100/skin 100/flyaway 100 face and detail crops, and original/flyaway head-halo crops.

The native run asserts unchanged source dimensions, a larger overall under-eye pixel effect at 100 than 50, zero changed pixels outside the quantized under-eye mask, and zero changed pixels outside expanded face bounds for facial-only edits. Auto Retouch starts with zero contrast and exposure and retains those values. These checks establish registration and effect scope; visual review is still required for eyelashes, glasses, facial hair, the skin/hair boundary and texture quality within the edited area. The saved hair-edge detail is one side of the head; the additional head-halo samples cover the detector's complete eligible area after clipping to the source bounds.
