# Clothing and stray-hair brush repair

These are explicitly painted corrections; they do not load or retrain an AI model. The automatic face-aware flyaway slider retains its separate hairline/face safeguards.

- Choosing either brush starts in paint mode with the comparison and mask overlay off. The workspace bar exposes **Erase repair**; X also switches erasing. Switching from a small hair brush to clothing starts with a usable fabric radius. New manual strokes save the strength shown by the brush slider.
- Clothing correction estimates broad shading from nearby fabric with similar chroma, then applies luminance gain to the original texture. The sampling width follows the brush radius. Summed-area source tiles make wide samples inexpensive, exclude transparent donors and keep the same coordinates for full/native rendering. Use a brush wider than the crease; trace the crease and avoid structural seams and printed patterns. Deep folds are reduced, not reconstructed as a new garment.
- Stray-hair repair samples opposing background/skin across several widths. The traced center determines dark/light strand polarity so bright gaps between dark hairs are not mistaken for pale strands. Curved, anti-aliased and lower-contrast hairs are covered by regression cases. Trace individual strands, increasing brush size for thicker hairs; bulk hair masses, lashes and brows need deliberate selection.
- A two-sided continuity pass fills short gaps along detected strands. Incremental canvas updates include its four-pixel neighborhood, preventing stale fragments outside a newly appended stamp.
- Both corrections remain non-destructive, respect layer visibility/opacity and ordered erasing, and share their source-registered correction maps between preview, native detail and export. Clothing no longer requests full-image fine/low blur buffers merely to run its brush.

Reproduce native portrait crops with `cargo run --release --locked --example painted_repair_review -- output/manual-repair/review`. The development machine uses `CARGO_PROFILE_RELEASE_LTO=false` to keep link memory bounded. Review source crops alongside the 50/100 results; a nonzero pixel count alone is insufficient evidence of a useful repair.

Regression coverage is in `src/painted_repair.rs`, `src/workflow_tests.rs` and `src/ui_tests.rs`: measured defect reduction, weave retention, paint bounds/alpha, printed color boundaries, cancellation, native/full parity, history/layer behavior, tool selection after erasing, saved strength and real worker feedback before pointer release.

## Local qualification — October 14, 2026

All **267 release library tests** and strict all-target Clippy passed with the default ONNX/Burn features. Curved dark/light hair fixtures at 256/512/1024 pixels reduced centerline error against their known clean sources by **90.1–96.8%**. Broad dark/light fabric folds reduced center shading error from **63 to 20** and **42 to 13** byte levels, retaining measurable weave contrast. These figures describe the fixtures, not guaranteed performance on every photograph. The colored stripe stays byte-identical, with at most two byte levels of tonal change beside it inside the painted area. Outside paint, pixels remain unchanged.

The complete suite also verifies growing strokes against complete native renders, fitted/native worker updates while the pointer remains held, original-size PNG parity, layer hiding, undo/redo, opaque/transparent edges and cancellation. Physical mouse/display latency and a clean Windows host have not been measured. Model weights were not changed.

The release executable produced the portrait crops in `output/manual-repair/release-review/`: sleeve creases, the broader blouse fold and a traced curved hair, each at 50/100. All six native crops match their full renders byte for byte. These were visually inspected; the earlier patchy directional fabric experiment was rejected. Fine creases and deep structural drape can remain, so Patch/Clone remain useful for larger reconstruction.

Packaged-app qualification passed eight desktop/compact screenshots and the CPU installation check for all eight manual tools, including history, layers, sessions and lossless PNG. Installer installation/uninstallation passed on the development machine with **47 file hashes** verified and the temporary app/registry removed.

Updated unsigned Windows x64 installer:
`output/beta/0.1.0-beta.1/20261014-150223-774/artifacts/Hastur-Retouch-0.1.0-beta.1-windows-x64-setup.exe`

262,854,289 bytes. SHA256: `1b15c640d3e655981891f213260a514a8d3582bd76ea3a7b00ca68b4ac8ca346`.
Logs are in `output/manual-repair/`. No GitHub release was changed.
