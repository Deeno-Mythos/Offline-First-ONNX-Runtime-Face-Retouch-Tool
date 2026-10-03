# Hastur Retouch identity

The crown is a hand-authored vector refinement of the user's supplied logo at
`C:/Users/subop/.codex/attachments/d5bc8a33-91a1-4bdc-8332-a1a1b097adf2/image-2.png`.
It keeps all five spires, the tallest center, medium outer points, short inner
points, ink/gold identity, and detached golden base. The silhouette is exactly
symmetrical, with even negative space and clean geometry shared by every format.

`hastur-mark.svg` is the 512-unit master. `hastur-mark-light.svg` uses the ink
crown on a transparent surface, and `hastur-mark-dark.svg` uses ivory. All bases
use Hastur gold `#F2C526`. `hastur-mark.png` and `hastur-mark-dark.png` are 1024px
transparent versions. `hastur-icon.png` adds a rounded ink backing for legibility
in window chrome. `hastur.ico` includes 16, 24, 32, 48, 64, 128, and 256px frames.
`brand-preview.png` shows light/dark use and the icon at native small sizes.

The splash embeds the user's chosen `example-img/demo-portrait.png` through its
byte-identical bundled copy, `assets/demo-portrait.png`, at the original 1122×1402
dimensions. Their SHA-256 is
`c794fcf6f62b868484dd52b8ecd7e204f693ead64c23ba3bb45bb47a982c1ba4`.
The full portrait is displayed without cropping or stretching. This integration
applies no retouching, grading, face edits, model training, or regenerated imagery
to the supplied PNG. The asset generator prepares crown derivatives only.

Rebuild the assets with a Pillow-enabled Python:

```powershell
.\.ai-tools\Scripts\python.exe assets\branding\generate_assets.py
```

The programmatic `src/branding.rs` painter uses the same crown coordinates and
explicit triangles for the concave silhouette, retaining its aspect ratio.

Windows executable builds compile `icon.rc` with the Windows SDK resource
compiler, then link the result into the binary. The SDK can be on `PATH`, in the
normal Windows Kits installation, or selected using the `RC` environment
variable. GNU Windows targets use `WINDRES` or the target's `windres` executable.
The resource is also rebuilt when its ICO or compiler environment changes.
See the [Microsoft resource workflow](https://learn.microsoft.com/en-us/windows/desktop/menurc/about-resource-files)
and [Cargo linker directives](https://doc.rust-lang.org/cargo/reference/build-scripts.html#rustc-link-arg-bins).

Verify an actual built executable without launching it:

```powershell
.\.ai-tools\Scripts\python.exe assets\branding\verify_pe_icon.py target\release\hastur-retouch.exe
```

The verifier checks that all seven embedded icon frames match the ICO payloads
byte for byte, rather than relying on Explorer's icon cache.
