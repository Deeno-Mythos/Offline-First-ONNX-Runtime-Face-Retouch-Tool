# AI efficiency and smaller installer

The October 14, 2026 update preserves both ONNX Runtime and Burn support, all five models, and their original precision. No model was quantized or retrained. The source photographs, edit format, history and image-rendering formulas are unchanged.

## Implementation

The installer previously used LZMA2's fast setting with a 32 KiB dictionary. The compact profile uses a 256 MiB solid dictionary, one block stream, and adjacent ONNX/Burn model pairs. Matching tensor bytes can then share archive space. Extraction restores every original file. This follows [Inno Setup's compression settings and memory tradeoffs](https://jrsoftware.org/ishelp/topic_setup_compression.htm) and [solid block behavior](https://jrsoftware.org/ishelp/topic_setup_lzmanumblockthreads.htm). The builder uses roughly 3 GB of compression memory; extraction needs approximately 256 MiB for the dictionary. `scripts/package-beta.ps1` defaults to Compact; `-CompressionProfile Fast` remains available for quick development packages.

Face-only and skin-only AI results previously were not persisted. Reopening these portraits repeated model loading, face detection and/or neural skin inference. The new cache records actual stage readiness and reuses complete partial results. A later request initializes only its missing stages; a skin cache never masquerades as completed blemish repair.

Partial maps use lossless DEFLATE compression, preserving every float bit. Complete maps remain raw to preserve fast SSD reads. The reader accepts the existing V2/V3 formats and the staged V4 envelope, rejects corrupt/truncated data, limits decompression size and disallows nested envelopes. Cache writes use unique temporary files to avoid simultaneous writers sharing a temporary name. The existing four-entry / 128 MiB memory limit and 512 MiB disk quota remain. Explicit skin masks are not saved as generic shared AI results.

## Local measurements

`examples/ai_efficiency.rs` runs in separate processes against the embedded/bundled 1122×1402 portrait, with the CPU runtime and isolated caches. The timer includes AI cache lookup/read or inference plus cache write. Image import and the subsequent render are outside the timer. These are local samples; they do not predict every machine or photograph.

| Reopened analysis | Before | After | Resident model instances before → after |
| --- | ---: | ---: | ---: |
| Face geometry | 196.257 ms | 13.258 ms | 2 → 0 |
| Neural skin | 1305.419 ms | 45.928 ms | 3 → 0 |
| Complete portrait | 22.348 ms | 22.012 ms | 0 → 0 |

The face cache occupies 835,169 bytes and the skin cache 5,977,873 bytes for this portrait. A complete cache is 33,755,910 bytes, ten bytes larger than its previous envelope. First-time inference still takes seconds: complete uncached analysis measured 3.52 s before and 3.86 s after, with normal run-to-run variation. This change targets reuse rather than inference arithmetic.

Upgrading a saved skin result to full retouching ran only `local_detection.onnx` and `local_inpainting.onnx`, in 2.09 s, and produced the same complete-map fingerprint and rendered pixels as a fresh full analysis.

## Verification and package

- 262 release unit/integration tests passed, including legacy cache compatibility, readiness preservation, exact partial floats, corruption/truncation rejection and dimension/crop validation.
- Strict Clippy across all targets, formatting and diff checks passed.
- All six fresh/reopened stage comparisons matched baseline map fingerprints and compared rendered pixels byte for byte. The skin-to-full upgrade also matched the complete baseline. Evidence: `output/efficiency/parity.txt` and `before/` / `after/` results.
- The NullState model-trace review passed: dormant requests loaded no models, incremental requests ran their missing stages, retained maps survived idle release, and a completed disk cache restored without initializing weights. Evidence: `output/efficiency/nullstate.log`.

The previous installer was 506,329,331 bytes, with 735,956,662 bytes of installed files. The verified compact installer is **262,761,397 bytes**, **48.1% smaller**. Installed files total 735,967,926 bytes (approximately 736 MB), with both backend weight formats present.

Verified installer: `output/beta/0.1.0-beta.1/20261014-130244-903/artifacts/Hastur-Retouch-0.1.0-beta.1-windows-x64-setup.exe`.

SHA256: `73200efbe3d760c152e55bb3064b6ad935c26cc80062fc9d486ef7661c21aa6c`.

Install/uninstall passed, verifying all 47 payload hashes. The packaged app passed CPU inference, cache reload, all eight manual tools, saved sessions, history, masks and lossless PNG checks; eight desktop/compact screenshots were captured. Evidence: `output/efficiency/native/`, `native-check.log`, `installer-check.log` and `package-verified.log`.

All five ONNX file hashes match the previous package exactly. Regenerated Burn packs have different generated parameter IDs; their tensor data, shapes and dtypes are byte-identical after excluding those IDs. Evidence: `output/efficiency/burn-weight-parity.json`.

The installer remains unsigned, and no clean Windows host was available. No public release or tag is changed by these local builds. The current source working tree includes earlier uncommitted app work.
