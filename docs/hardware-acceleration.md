# Hardware acceleration

Hastur Retouch defaults to **Auto** for the bundled portrait AI. On Windows it tries ONNX Runtime DirectML on the primary hardware GPU. The startup screen and AI panel receive actual preparation stages: cache lookup, face detection, landmarks, neural skin, blemish detection, repair and completion.

Auto compares each model's first actual portrait input with CPU output. Shapes must agree, all values must be finite, and every value must differ by no more than `0.005 + 0.003 × abs(CPU value)`. An ONNX Runtime profile must also contain a kernel event assigned to the GPU execution provider. Provider registration alone never qualifies as GPU execution. Missing runtimes, model errors, missing GPU kernel events or output disagreement select CPU for that model and leave a visible reason. A later GPU inference error also retries that model on CPU. Provider switches release retained sessions; model file changes invalidate qualification. Qualification is repeated in each process, using the first uncached input for each model.

Initial qualification runs both CPU and GPU and can take longer than ordinary analysis. Subsequent uncached photos reuse qualified GPU sessions. Cached analyses load existing maps without running or warming AI, and retain the original provider label with a cached indication. Reanalyze bypasses the image cache. Explicit CPU, DirectML, CUDA, CoreML and experimental Burn CPU remain selectable; CUDA and CoreML require compatible platform runtimes. Imported custom mask models use CPU in Auto mode because they have different contracts; explicit ONNX providers remain selectable for them. This implementation was verified on Windows x64 with Intel UHD Graphics driver `31.0.101.4255`; other hardware still goes through qualification.

The Windows DirectML runtime lives in `models/directml/`, separately from CPU DLLs. It also supports CPU execution, so it is loaded before creating any sessions; ONNX Runtime uses one library per process. `ORT_DYLIB_PATH` explicitly overrides runtime selection. Install only that runtime with `scripts/setup-ai-runtime.ps1`, or install models and runtimes together with `scripts/setup-ai.ps1`. Setup verifies official Microsoft NuGet packages by SHA-256 and retains ONNX Runtime/DirectML licenses. Startup never downloads anything or uploads a photo.

DirectML sessions disable memory pattern optimization and parallel execution as required by [ONNX Runtime's DirectML documentation](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html). Hastur also sets the official `disable_metacommands=true` provider option. The [v1.24.4 provider implementation](https://github.com/microsoft/onnxruntime/blob/v1.24.4/onnxruntime/core/providers/dml/dml_provider_factory.cc) supports that option. It bypasses vendor metacommands while preserving ordinary DirectML GPU kernels.

On this Intel driver, the earlier unconfigured DirectML detector gave a maximum face confidence of `0.31275` versus CPU's `0.94831`, with large tensor errors. Disabling metacommands restored confidence to `0.94831`, with maximum detector tensor difference `0.0001221`. This matches the kind of Intel metacommand correctness issue described in [Microsoft's issue #18652](https://github.com/microsoft/onnxruntime/issues/18652), though that issue alone does not establish the cause on this machine. The actual local controlled comparison is saved in `output/hardware-acceleration/detector-probe.json`.

The following measurements were made on 2026-10-02 in an optimized release build with LTO disabled, with compilers and other heavy jobs stopped. Each provider ran twice in a separate process, without the image-analysis cache; “warm” means the second full analysis with model sessions retained. The CPU and GPU processes ran sequentially, so thermal state and the driver's persistent shader cache can affect timing. These are measured examples, not a universal speed guarantee. Source-resolution PNG rendering followed timing and is excluded from the AI durations.

| Portrait / provider | First complete AI | Warm complete AI | Peak process memory after warm AI |
| --- | ---: | ---: | ---: |
| Demo / CPU | 4.315 s | 5.782 s | 2350.57 MiB |
| Demo / explicit DirectML | 7.782 s | 2.129 s | 2131.82 MiB |
| Demo / Auto, including initial CPU qualification | 8.180 s | 1.884 s | 2544.52 MiB |
| Supplied red-dress portrait / CPU | 12.891 s | 6.303 s | 2394.00 MiB |
| Supplied red-dress portrait / Auto, including initial CPU qualification | 20.224 s | 2.295 s | 2620.62 MiB |

Auto's warm AI took approximately 3.1 times less time than CPU on the demo and 2.7 times less on the supplied portrait. Its peak includes temporary CPU reference sessions during qualification, source/preview images, masks and the runtime; it is not dedicated GPU memory. The supplied photograph remained `3840 × 5760`, while inference used its `960 × 1440` overview and registered maps.

All five models qualified on both portraits. Profiles prove 81 DirectML events for the detector, 219 for landmarks, and one fused DirectML graph event each for neural skin, blemish detection and inpainting. The detector also runs 12 small CPU shape operations, which is expected and does not mean its neural computation fell back to CPU.

A separate negative-path run explicitly loaded the CPU-only runtime through `ORT_DYLIB_PATH`. Auto reported each DirectML registration failure, completed all five models on CPU, found the face and rendered normally. Its registered maps and output pixels were identical to explicit CPU. The fallback run was performed while final compilation was active, so its timing is not included in the performance table.

Registered maps, overview renders and entire native PNG outputs were compared with CPU. Demo native output differed in only four pixels by at most one byte. The supplied 22,118,400-pixel native output differed in only 18 pixels by at most one byte; overview differed in one pixel. Both passed the export-precision gate (maximum difference at most two bytes and mean less than 0.05 byte). Auto-retouch contrast and exposure remained zero in both provider outputs. No model weights or training data changed for this acceleration work.

Reproduce after building `examples/hardware_validation.rs`:

```powershell
$env:CARGO_PROFILE_RELEASE_LTO = 'false'
cargo build --release --locked -j 2 --example hardware_validation
$env:HASTUR_AI_PROFILE = (Join-Path (Get-Location) 'output/hardware-acceleration/profiles')
$env:HASTUR_AI_TIMINGS = '1'
target/release/examples/hardware_validation.exe cpu assets/demo-portrait.png output/hardware-acceleration/cpu 2
target/release/examples/hardware_validation.exe auto assets/demo-portrait.png output/hardware-acceleration/auto 2
python scripts/compare-hardware-output.py output/hardware-acceleration/cpu output/hardware-acceleration/auto output/hardware-acceleration/profiles output/hardware-acceleration/demo-hardware-comparison.json
```

The comparison script needs NumPy and Pillow only for development validation. Profile artifacts are normally temporary and removed after qualification; `HASTUR_AI_PROFILE` retains them in the specified directory. `HASTUR_AI_TIMINGS` prints per-model timing; the legacy `ASTRA_AI_TIMINGS` setting remains accepted. CPU/GPU reports, native PNGs and kernel profiles are under `output/hardware-acceleration/`; `demo-hardware-comparison.json` and `supplied-hardware-comparison.json` contain the map, pixel and kernel evidence.
