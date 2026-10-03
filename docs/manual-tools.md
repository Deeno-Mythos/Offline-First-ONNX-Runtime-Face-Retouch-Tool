# Manual cleanup tools

The 2026-10-02 repair addresses an actual recovered workspace in which the active Retouch layer was hidden. The workspace already contained healing, liquify, clone and patch instructions, so the cursor and history moved while the composite correctly remained unchanged. Saved instructions are preserved; the recovery file is never rewritten by diagnosis.

Selecting a manual tool shows the entire edited photograph. The canvas header names the tool and explains the next action. A hidden layer, zero opacity, locked layer, disabled legacy tool group, disabled healing or zero brush strength produces a visible warning and blocks invisible strokes. **Show layer**, **Set opacity to 100%**, **Enable tool** and **New retouch layer** offer explicit remedies. An opaque normal image-copy above the target offers a new layer at the top. These layer changes are undoable. The toolbar keeps its height when source/selection controls appear, preserving image registration.

## Editing

- **Liquify:** drag to push or pull the photograph. Size, softness and strength control each saved warp. **Reset liquify** clears the active layer's warps in one undo step.
- **Spot heal:** click or paint over a defect. Nearby intact texture is selected using boundary color and texture evidence. Brush strength is now saved independently for each stroke; changing the next stroke's strength does not increase previous corrections. Older sessions retain their original global healing setting, with new per-stroke opacity defaulting to 100%.
- **Clone stamp:** click **Choose source**, then click a clean area; Alt+click also selects a source. Source selection never paints, including when Alt is released before the mouse button. **Aligned source across strokes** retains the source offset between gestures. Turn it off to start each gesture from the selected source. Sampling the original side of comparison uses the original coordinates even after liquify. The source crosshair follows the sampled texture, and resampling or changing layers resets alignment.
- **Patch:** trace around a repair area, release, then drag inside the outline onto clean texture. The destination displays the rendered result during the source drag. Releasing commits one patch and one undo step; **Clear selection** or Escape cancels the draft. Feather and strength control the blend. Boundary samples fit a local lighting gradient, replacing a constant color offset that could leave rectangular shading seams. Feathering reaches farther into the selection. Pixel sampling uses the same grid as clone and the renderer, eliminating an unintended half-pixel offset that softened transferred texture.

Liquify, healing and clone expose **Diameter** in original-image pixels. Bracket shortcuts change size; Shift+brackets change strength. Hold Space and drag to pan temporarily. Original-view comparison prevents painting into a view that cannot show edits. Fast same-frame press/move/release events preserve the original press position and commit visible changes.

## Preview and persistence

Patch feedback shares the independent native preview worker, rather than waiting behind AI, masks or presets. One candidate is outstanding at a time; obsolete requests are cancelled and late results are ignored. A matching final frame remains visible while the committed revision renders. At detail zoom, the candidate renders from the original photograph into the padded visible source region, reusing its immutable source crop and upload while the donor moves. At overview zoom it uses the normal preview size. Comparison and final edits use one GPU canvas callback with a matched before/after pair.

Draft feedback is separate from the photo's edit, history and autosave. A completed patch or brush gesture uses normal session serialization and layer compositing. Old sessions load without requiring a format migration. Originals retain their dimensions and alpha; native sharpness is preserved through ordinary stroke, adjustment and save updates.

These are deterministic CPU texture-transfer and deformation tools. Healing uses local donor selection and boundary matching; Patch uses a feathered transfer with local lighting compensation. They do not provide semantic inpainting or Photoshop's full toolset. Preview speed still depends on image size and the existing edit stack.

## Reproduce validation

```powershell
$env:CARGO_PROFILE_RELEASE_LTO='false'
cargo test --release --locked --lib -- --test-threads=2
cargo run --release --locked --example manual_tools_review
```

The example checks both example portraits, saves four overview results and four original-pixel crops per portrait, and verifies visible pixel changes, cached/fresh overview agreement, undo/redo, serialization, dimensions and alpha. `output/manual-tools/validation.csv` reports pixel counts and cold render timings. These timings are not warmed interactive frame-rate claims. Pass a saved workspace path to the example for read-only layer diagnosis; any illustrated result is written only to `output/manual-tools/`.

UI regression coverage includes unavailable targets, restoration of hidden existing work, fast click registration, source-pick input ownership, aligned and unaligned sampling, reversible gestures across five zooms and four DPI scales, temporary Patch feedback, cancellation, sharp native feedback and a single GPU callback. Texture tests cover exact identity, integer Patch transfer and preservation of a changing light gradient and fine texture during defect repair. Worker tests deliberately block background work while overview, Patch and native previews finish.

Native screenshot review can select a manual tool on launch:

```powershell
target/release/hastur-retouch.exe --manual-tool clone --screenshot output/manual-tools/clone-ui.png
target/release/hastur-retouch.exe --session output/manual-tools/hidden-layer.ron --manual-tool liquify --screenshot output/manual-tools/hidden-ui.png
target/release/hastur-retouch.exe --patch-preview --screenshot output/manual-tools/patch-feedback-ui.png
```

`--patch-preview` seeds a temporary selection only when `--screenshot` is present, to review the rendered GPU feedback without changing saved edits. Screenshot runs skip preferences and recovery autosave.

## Completed checks, 2026-10-02

All **179 optimized tests** passed with ONNX/Burn enabled. All-target Clippy with warnings denied, formatting and whitespace checks passed. The release app and the review example were rebuilt. Desktop, compact, hidden-layer, live Patch and native Patch screenshots exited successfully with empty stderr; the final lighting refinement was reviewed again in both portraits and the Patch screenshots.

The 3840×5760 example's original-pixel review crops recorded 165,508 changed pixels for Liquify, 3,080 for one healing stamp, 5,907 for one clone stamp, and 15,792 for one patch. These counts demonstrate that each tool applies pixels; they do not measure aesthetic retouch quality. Inspect the before/after PNGs with the CSV in `output/manual-tools/`. The example also verified cached/fresh agreement, reversible instructions and session round trips, without modifying either original photograph.
