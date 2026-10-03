# History, layers and a larger photo workspace

Tools, History and Layers share one inspector. Switching between them replaces its contents, so opening History or Layers does not add another sidebar beside the photo.

## Show or hide a panel

Use **View → Tools**, **View → History** or **View → Layers** at any window width. Wide windows also have **History** and **Layers** buttons in the top bar; clicking the active button again hides the inspector. The inspector's tabs switch panels, and **×** closes it. **View → Hide panels** gives the photo the available sidebar space. Press **Tab** to hide the current panel or reopen the last selected panel; while typing in a field, Tab keeps its normal input behavior.

Narrow windows show the inspector in a floating sheet. **Done** dismisses the sheet, and View or Tab can reopen it. Selecting an editing tool returns to its controls. Opening, closing or switching panels does not change the photograph or create an undo step.

## Navigate history

History belongs to the current photograph. Its rows run from the oldest retained state through the current state to the available redo states. The current row has a dot; redo rows use quieter text. Click a row to restore that state, or continue using **Ctrl+Z** and **Ctrl+Shift+Z**.

Row names describe differences between saved states. A slider drag or continuous brush gesture is one edit, rather than a row for every pointer movement. A jump restores the final state without rendering every intervening step. Returning to an older state preserves the redo branch until a new edit replaces it.

The history keeps up to 100 undo steps within its instruction-storage budget. **Starting state** means the oldest retained snapshot; after older history has been trimmed, it may already contain edits. Saved sessions and automatic workspace recovery retain both undo and redo states.

## Work with an editable layer stack

Layers now contains an ordered stack for each photograph, with thumbnails and a permanently locked original **Background** beneath it. Existing flat edits migrate into a **Retouch** layer without changing their rendered pixels. Opening the panel does not add an undo step.

- The **retouch icon** creates a blank layer for manual repairs and portrait adjustments.
- The **half-circle icon** creates a separate adjustment layer; select it and open Color to edit exposure, HSL or curves.
- The **image icon** adds a full-resolution copy of the original, which can cover edits below it at Normal/100%.
- Click a row to select the layer being edited. Tools display its name and load its own settings and strokes. The original Background can be selected for inspection, and remains read-only.
- Use the **duplicate**, **trash** and **arrow** icons to copy, delete or reorder the selected layer. You can also drag its row to reorder it. Duplicated adjustment layers apply their effect again.
- Double-click a row name to rename it. An **eye** icon hides the layer; **Opacity** fades it from 0–100%, with double-click restoring 100%. The numeric value accepts an exact percentage.
- **Lock** protects a layer's contents and order. Its tool controls and canvas painting are disabled until unlocked. Copying a locked layer produces an unlocked copy.
- Blend modes are **Normal**, **Multiply**, **Screen** and **Difference**. Empty retouch pixels remain unchanged when blending.

Layers process from bottom to top at the source resolution, sampling the composite underneath. Opacity cross-fades in linear light. Layer actions, independent settings, strokes, order, names and blend modes are undoable and saved with sessions and automatic recovery. Selecting a layer does not add a history step. Up to 32 editable layers are supported. Hidden layers do not request unprepared portrait AI under [NullState](nullstate.md).

### Layer workflow refinements

The panel uses vector icons, large mouse targets, tooltips and disabled states for unavailable actions. Rows show the layer type, opacity, proportional thumbnail and inline lock. Long names truncate with an ellipsis and remain readable in their tooltip.

- **Find layer** filters names without changing the image or history. The kind dropdown filters retouch, adjustment or image-copy layers. Creating or duplicating a layer clears the filters so its new row is visible.
- **Alt-click an eye** to solo that layer over the original Background. Alt-click the same eye again, or use the highlighted solo icon beside the title, to restore the exact prior visibility set. Soloing a different layer retains that original set. Adding, duplicating, deleting or normally toggling visibility exits solo first. Solo state survives session saving and undo/redo.
- **Right-click a row** for rename, duplicate, solo, lock, copy/paste/reset adjustments and delete. Seven color-label choices, including no color, mark rows for organization; labels are saved and undoable without altering rendered pixels.
- **Copy adjustments**, **Paste adjustments** and **Reset adjustments** are the icons beside the blend selector. They affect settings only and preserve manual brush instructions. Paste and reset are disabled for locked layers. The copied settings can be pasted into another photograph's layer during the current app session.
- With the Layers panel open, **Ctrl+Shift+N** creates a retouch layer, **Ctrl+J** duplicates, **Ctrl+[ / ]** moves down/up, **Delete** removes and **F2** renames. Text editing and open menus suppress these shortcuts. **Escape** cancels renaming. Locked layers disable opacity, blending, deletion, movement and editing while keeping their visibility and labels available.

This is a non-destructive retouch/adjustment stack. Arbitrary imported raster assets, painted per-layer alpha masks, Photoshop document interchange, and Channels/Paths panels are not implemented. The older fixed tool-group controls remain supported in saved sessions.

Native requests reuse the composite below the top visible layer. Prefix images are limited to two and 192 MiB; existing source/filter caches keep their own limits. Editing a lower layer invalidates later composites and can take longer than editing the top layer. Session saving stores instructions and references to originals, rather than a resized image.

## Larger photo workspace

The workspace uses smaller panel margins and less unused spacing while retaining large panel tabs, history rows, visibility targets and opacity controls. Hiding the inspector frees its full width for the photo. Fit recalculates within the available viewport; native 100% zoom continues to preserve one source pixel per physical display pixel, including Windows DPI scaling.

## Panel UI verification

Raw-input egui checks cover layer creation and edits, selected-layer isolation, locking, duplication, deletion, opacity gesture coalescing, drag reordering, renaming and the locked background. Engine checks cover migration without pixel changes, ordered composition and all four blend modes, full/native/cached agreement, original-resolution PNG output, and saved history round trips. The quality regression checks native pixels during a stroke, after release, after an overview completes and after a save notification.

The refinement checks exercise actual eye and context-menu clicks, solo restoration with previously hidden layers, label persistence and undo, adjustment copying without removing strokes, name/type filtering, shortcut scope, locked controls and accessible footer targets in a 600×560 logical-point window. Legacy sessions without label or solo fields retain their original defaults.

At a 1440×940 logical-point window the photo viewport remains 1050×619 with the inspector and 1350×619 with it hidden, a 28.6% area increase. Tools, History and Layers share the same dock and preserve photo proportions.

`examples/layer_stack_case.rs` creates a four-layer review workspace and verifies its native-resolution PNG. To reproduce the native panel capture:

```powershell
cargo run --release --locked --example layer_stack_case
target/release/hastur-retouch.exe --session output/layer-stack/review.ron --layers --screenshot output/layer-stack/desktop.png
```

The `--session` option opens a saved workspace directly without invoking automatic recovery. Captures and the review workspace are in `output/layer-stack/`.

The completed follow-up passed all 151 optimized library tests with ONNX/Burn enabled, formatting and whitespace checks, and all-target Clippy with warnings denied. The rebuilt app captured the panel at 1440×940 and 600×560 logical points, and a saved 100% native view, with exit code 0 and empty stderr. The captures are `output/layer-stack/desktop.png`, `compact.png` and `native.png`; the native capture uses `review-native.ron` because opening a session restores its saved zoom.

The icon and workflow refinements passed all 157 optimized tests with ONNX/Burn enabled, including the native-detail and layer export regressions. Formatting, whitespace checks and all-target Clippy with warnings denied passed. Final desktop, compact and 100% native captures from the rebuilt app exited successfully with empty stderr; they are `output/layer-stack/icons-desktop.png`, `icons-compact.png` and `icons-native.png`. Captures wait for the sheet's opening animation to finish. Wide sheets put search and filtering on one row, and the opacity track uses the available width while its numeric entry remains accessible.
