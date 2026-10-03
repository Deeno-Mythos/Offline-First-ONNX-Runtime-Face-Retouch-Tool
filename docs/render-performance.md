# Display profiling and SIMD, 2026-10-03

The native CPU renderer already retains a sharp viewport and redraws local brush damage. This pass profiles the next stages: CPU display preparation, changed-pixel detection, GPU upload and conversion into the linear working texture. Liquify, Spot Heal, Clone Stamp and moving Patch donors use this display path.

## Kept changes

- Native and Patch GPU canvases consume their original-resolution `Arc<RgbaImage>` directly. Their unused egui fallback thumbnails are replaced with one transparent pixel, and existing placeholder handles are reused. The non-GPU painter still receives the complete sharp image. Overview/filmstrip thumbnail behavior is unchanged.
- Same-size frames are compared against the previously uploaded frame, including alpha. Only the exact bounding rectangle of changed pixels is uploaded and converted from sRGB into `Rgba16Float`. Unchanged linear pixels remain in the existing texture. Identical frames skip both operations, even when wrapped in a new `Arc`.
- The changed-pixel scanner uses Rust AVX2 intrinsics after runtime CPU detection, with a portable scalar fallback. Unaligned loads stay within equal-length pixel slices; scalar tails cover smaller images and trailing bytes. Extra `ImageBuffer` storage after the last pixel is excluded.
- First frames and resized images upload and convert completely. Global changes still upload the complete affected area. Each original/edited working image has its own conversion-region uniform, so both can update in the same callback.

This introduces no resizing, image stretching, contrast adjustment, model change, or handwritten assembly. The optimization preserves the existing conversion math. Sparse edits far apart can produce a large bounding rectangle, reducing the upload saving without changing correctness.

## Profiling method

The CPU candidate profile uses a crop of the 3840×5760 JPEG `example-img/CTU DUMANJUG ORG 09.28.26_JAMESBRO-522.JPG` at 1536×1024 and 2048×1536. It measures 80 samples of scalar/AVX2 scanning and frame copying, plus 32 samples of the formerly generated 320-pixel fallback thumbnail. Local changes occupy a 73×91 rectangle; unchanged and global-change cases are measured separately. Original measurements remain in `output/render-profile/display-baseline.csv`; the integrated scanner writes `display-after.csv`.

The original CPU candidate measurements on an Intel Core i3-1215U were:

| Viewport | Scalar local scan | AVX2 local scan | Unused thumbnail generation |
| --- | ---: | ---: | ---: |
| 1536×1024 | 2.039 ms | 0.513 ms | 3.478 ms |
| 2048×1536 | 4.546 ms | 0.938 ms | 4.289 ms |

`gpu_upload_profile` compares full and partial upload/conversion using the production pipeline, alternating measurement order. It tests local, unchanged and global frames at both sizes. GPU readbacks compare complete `Rgba16Float` textures byte for byte. A separate correctness sequence covers alpha-only edits, corners, odd origins and row strides, disconnected changes, identical new allocations, resize and global changes.

`viewport_performance --gpu-profile` uses actual growing Liquify, Heal and Clone instructions plus a moving Patch donor, with 48 prepared instructions and 16 updates per tool/size. It compares every CPU frame with the complete native renderer, and checks GPU texture parity every fourth update. Its timing CSVs and final frames are stored separately under `output/render-profile/manual/`, preserving the earlier viewport benchmarks.

CPU encoding includes changed-pixel scanning, texture writes and compute dispatch setup. Completed GPU-update timings additionally include submission and a synchronous completion wait used only by the benchmark. They exclude input latency and screen refresh; these measurements do not promise a fixed frame rate. Application rendering continues to submit asynchronously.

## Final measurements and validation

The integrated scanner's local-change median was 2.120 → 0.356 ms at 1536×1024 and 4.585 → 1.157 ms at 2048×1536 (scalar → runtime AVX2). This is approximately 6×/4× faster. Unchanged-frame scans also improved, while dense global changes return full bounds immediately. The redundant thumbnail still measured 2.594/4.327 ms in this run; native/Patch GPU updates now omit that operation altogether.

The actual tool frames produced the following completed GPU-update measurements on Intel UHD Graphics / Vulkan:

| Viewport | Tool | Full upload median | Partial upload median | Full / partial p95 | Pixels uploaded |
| --- | --- | ---: | ---: | ---: | ---: |
| 1536×1024 | Liquify | 3.556 ms | 1.766 ms | 10.292 / 7.135 ms | 0.565% |
| 1536×1024 | Spot Heal | 3.764 ms | 1.937 ms | 10.068 / 2.628 ms | 0.209% |
| 1536×1024 | Clone Stamp | 3.780 ms | 2.474 ms | 7.811 / 7.791 ms | 0.494% |
| 1536×1024 | Patch | 3.777 ms | 1.894 ms | 10.375 / 7.649 ms | 0.810% |
| 2048×1536 | Liquify | 6.935 ms | 3.734 ms | 11.874 / 7.883 ms | 0.283% |
| 2048×1536 | Spot Heal | 6.879 ms | 2.862 ms | 11.623 / 4.712 ms | 0.105% |
| 2048×1536 | Clone Stamp | 6.937 ms | 3.220 ms | 9.098 / 6.442 ms | 0.247% |
| 2048×1536 | Patch | 7.339 ms | 3.231 ms | 8.486 / 7.504 ms | 0.405% |

These stage durations decreased by 35–58%. CPU encoding alone sometimes costs slightly more because it now compares frames (for example, Liquify at 2048×1536: 2.431 → 2.804 ms), while the completed GPU update and omitted thumbnail work provide the measured saving. Global full-frame changes showed no consistent timing improvement and retain full uploads. The 40-sample synthetic local-change comparison measured 3.299 → 1.940 ms and 7.554 → 3.283 ms; unchanged new allocations uploaded zero pixels. All scalar/AVX2 and GPU timing CSVs remain under `output/render-profile/`.

Validation completed:

- 196 optimized library tests passed with ONNX/Burn enabled, including live held-pointer edits for all four tools in fitted/native views, native sharp-frame retention, Patch GPU callback ownership, and independent scalar/SIMD bounds comparisons.
- 68 complete full-vs-partial GPU texture comparisons passed byte for byte: 12 edge/resize/alpha cases, 24 synthetic-profile comparisons and 32 actual-tool comparisons. Every one of the 128 CPU tool frames was byte-identical to its complete native redraw. All eight final tool PNGs match the previous sharp references.
- The rebuilt release app captured the native Liquify workspace and temporary Patch feedback, exited 0 with empty stderr, and both complete screenshots match the previous sharp captures byte for byte. Current captures are `output/render-profile/native-workspace.png` and `patch-feedback.png`; comparison results are in `reference-parity.json`.
- All-target Clippy passed with warnings denied; formatting and whitespace checks passed; the release executable and profiling examples rebuilt successfully.
- The trained ONNX generator retains SHA-256 `348da8ed8114d0951cfb1e5026fd6ed4c70a087e92ab8a7421723b96b666cb40`. Existing build-time Burn conversion regenerated its runtime pack from those same ONNX inputs. No training ran. The external `OneDrive/Desktop/DAY 2` folder was unavailable for a fresh dataset hash check; no source image writes were performed by this pass.

## Reproduce

Stop compilation and other heavy work before recording timings. In PowerShell:

```powershell
$env:CARGO_PROFILE_RELEASE_LTO='false'
cargo build --release --locked -j2 --example display_profile --example gpu_upload_profile --example viewport_performance
./target/release/examples/display_profile.exe --after
./target/release/examples/gpu_upload_profile.exe
./target/release/examples/viewport_performance.exe --gpu-profile
```

Implementation: `src/pixel_changes.rs`, `src/gpu.rs`, `shaders/linear.wgsl`, `src/ui.rs` and `src/manual_tools.rs`. Headless profiling/readback helpers live in `src/gpu_diagnostics.rs`; they are not called by the application's render loop. See [live tools](live-tools.md) for CPU-render reuse, memory bounds and first-stroke limitations.
