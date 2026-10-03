# Hastur Retouch branding and startup

The application, native window, session/preset dialogs, launcher, and Rust
package now use **Hastur Retouch**. Run `Launch-Hastur.ps1`, or build and open
`target/release/hastur-retouch.exe`. The launcher forwards photograph paths and
CLI options, including paths containing spaces.

## Crown and portrait

The refined crown retains the supplied logo's five spires, tallest center,
medium outer points, short inner points, and detached gold base. The silhouette
is symmetrical and available as transparent SVG/PNG variants. A rounded ink
backing gives the window/Explorer icon contrast at small sizes; its ICO contains
16, 24, 32, 48, 64, 128, and 256px frames. The egui painter uses the same geometry
without stretching it. Both runtime icons and splash imagery are embedded, so
launching does not require the original attachment files.

The side portrait is the user's chosen `example-img/demo-portrait.png`, embedded
through the byte-identical `assets/demo-portrait.png` at its full 1122×1402
resolution. This integration preserves the original PNG pixels and applies no
cropping, retouching, grading, face edits, regeneration, or model training. The
splash uniformly fits the full image as the window resizes. See
[asset provenance and formats](../assets/branding/README.md)
and [light/dark preview](../assets/branding/brand-preview.png).

## Startup and AI preparation

The splash displays actual workspace restoration and import events received
from the background workers. When restored edits require portrait AI, it also
shows cache reads, model execution and GPU qualification. The bar represents
preparation milestones, rather than a time-based animation or an estimated
remaining duration. It proceeds to the workspace after the current photo's
first preview and any requested AI work complete. **Open workspace** allows editing to begin
while portrait analysis continues. AI errors or a missing confident face are
reported, and manual editing remains available.

Startup automatically resumes the newest usable saved workspace without a
recovery-choice dialog. Failed or empty generations fall through to the
previous generation and older unlocked recoveries; if none load, the demo
opens. Saved edits and both history branches are restored, and skipped or
partially available recoveries remain on disk. Explicit photograph paths and
screenshot capture bypass this automatic restoration.

**Auto · GPU when qualified** compares each model's GPU output with a CPU
reference on the current request and verifies GPU kernels through ONNX Runtime
profiling. A failed provider, unacceptable output difference, or later GPU
failure selects CPU for that model and reports the reason. Qualified choices
are reused during the running process. CPU and Burn remain selectable. The
canvas adapter label describes graphics rendering; portrait status reports the
inference providers used. Imported photographs and the bundled generated demo
open with neutral settings. Under [NullState](nullstate.md), portrait models
remain dormant until an enabled portrait edit, Auto Retouch, explicit reanalysis
or restored active edits require them. Color and manual cleanup do not prepare
portrait AI. Model instances return to dormancy after 90 idle seconds; completed
photo maps and edits remain usable.

## Compatibility and verification

`Launch-Astra.ps1` forwards to the new launcher. Existing session/preset formats
remain usable. New recovery files use `%LOCALAPPDATA%\HasturRetouch\recovery`,
while older `%LOCALAPPDATA%\AstraRetouch\recovery` workspaces remain discoverable.
`HASTUR_RECOVERY_DIR` takes precedence over the supported `ASTRA_RECOVERY_DIR`.
New neural caches use `.hastur-cache`; `.astra-cache` remains a read fallback.
Stable cache signatures and generated model-pack paths preserve compatibility.
See [recovery details](workspace-recovery.md).

Windows builds compile the ICO into the executable's PE resources using the
installed resource compiler. Verify every embedded frame against the source
ICO without launching the app:

```powershell
.\.ai-tools\Scripts\python.exe assets\branding\verify_pe_icon.py target\release\hastur-retouch.exe
```

Capture the retained startup screen with the current executable:

```powershell
target\release\hastur-retouch.exe --splash-preview --screenshot output\splash.png
```

The 2026-10-02 verification run passed 117 optimized-release regression tests,
including startup input protection, AI failure completion, compact-window
workspace access during import, photo-local status, legacy sessions/recovery,
GPU qualification rejection checks, and fully visible/clickable zoom controls
at the minimum window height. Automatic recovery tests also cover preserved
history, no competing demo import, empty/corrupt generations, invalid images,
previous-generation/older fallback, and retained skipped recovery files.
`cargo clippy --locked --all-targets
--features onnx -- -D warnings` and `cargo fmt --check` passed.

Native captures at 1100×700 and 600×560 splash window sizes, plus the normal
workspace, were inspected in `output/branding-validation/`. Controls remain
readable at the minimum size, the portrait retains its proportions, and the
workspace uses the new crown/name/footer. The built executable passes the
seven-frame PE icon check. The first image is a retained ready-state preview;
actual startup displays worker preparation stages and then opens automatically.
Measured AI acceleration and output agreement are documented in
[the hardware report](hardware-acceleration.md).
