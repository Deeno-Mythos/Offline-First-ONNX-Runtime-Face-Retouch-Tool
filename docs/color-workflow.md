# Color & light workflow

Color edits are explicit, non-destructive adjustments. Auto Retouch does not enable HSL, curves, or reference matching.

## Selective HSL

Choose one of the eight hue ranges, then adjust hue (degrees), saturation, and luminance. Neighboring hue ranges blend smoothly; gray pixels remain neutral. **HSL amount** blends all selective edits. Its checkbox disables the effect while keeping each range's values. **Reset selected range** clears only the displayed range.

## Luminance and RGB curves

Choose **Luma**, **Red**, **Green**, or **Blue**. Drag any of the nine large curve points vertically. The horizontal inputs are fixed at eighth intervals so selecting a point remains predictable. Smooth shape-preserving interpolation avoids overshoot between adjacent points. The Luma curve adjusts linear luminance by a common RGB scale, preserving hue until a channel reaches the output gamut limit. Individual RGB curves operate on sRGB-encoded channel values.

**Gentle S** applies a starting curve to the selected channel. **Reset curve**, or double-clicking the graph, restores identity for that channel. **Curves amount** blends all four curves; its checkbox keeps the points while disabling the grade. A curve drag is one undo step.

## Reference matching

**Choose reference photo…** reads an image asynchronously, honoring its embedded input ICC profile through the normal photo loader. A reusable profile of lightness quantiles and perceptual color balance is embedded in the current photo's settings. The source file is no longer needed after loading, including when reopening a saved session or preset.

Adjust **Reference match amount** to blend the result. **Match selected to reference** copies only reference matching to other selected photos; their other settings, local strokes, and unselected photos remain intact. Each portrait is independently measured against the same target at render time. **Use current look for selected** uses the current fully rendered preview as the target for other selected photos; it does not change the current photo. A pending preview disables that action so stale color is never used.

Normal Batch **Sync settings to selected** copies HSL, curves, and the embedded reference alongside the other settings. Source statistics come from the complete render input: the whole preview for previews, and the whole original for full-resolution tiled exports and native regions. Per-tile profiles are never used. Thumbnail resampling can produce small preview/export differences. Reference statistics use a bounded normalized sampling grid rather than a full-resolution floating-point buffer.

Reference matching is statistical color grading, not semantic scene or skin-tone matching. Similar lighting, framing, and backgrounds produce the closest match. Highly different scenes can still need individual HSL, white balance, or curve adjustments. Very dark, clipped, or transparent references without enough visible midtones are rejected. Extreme brightness changes and chroma variation are bounded to avoid an excessive single-step correction.

## Verification

`cargo test --no-default-features color::tests` checks neutral identity, gray/other-sector isolation, smooth interpolation without overshoot, RGB channel isolation, alpha preservation, profile serialization, and improved target matching across two independently measured sources. UI tests exercise actual pointer drags, undo/redo, effect disabling, obsolete asynchronous results, and selected-photo reference synchronization.

Fresh optimized measurements compared preview versus original source profiles on five real photographs at 65% and 100% matching, using each original as the identity reference and a normalized 128×128 sample grid. At 100%, the macro portrait's mean RGB-average difference was 0.1694 bytes; the supplied 22.118 MP portrait measured 0.2597 bytes mean, 0.6363 p95, and 1.1608 maximum sampled RGB average. Smaller images with identical preview/original inputs measured zero. These values isolate profile/resampling consistency and do not measure full export or perceptual batch quality. See [measurement details](goal-validation.md) and [raw results](../output/performance/color-consistency.txt).
