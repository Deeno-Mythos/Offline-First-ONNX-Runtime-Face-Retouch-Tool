# Image panning responsiveness

The user's slow interaction was moving the image with the Hand tool or Space-drag. This pass addresses that camera gesture and its native-resolution canvas path.

## Implementation

- Capture the pointer's grab position and starting pan. Apply total displacement immediately, including movements below egui's drag threshold, coalesced events, and the final release position. Starting or releasing Space during a captured gesture does not jump or resume painting accidentally.
- Keep unrestricted X/Y movement at Fit and native zoom, including when the image moves outside the canvas. A stationary click preserves Fit; dragging freezes the current scale.
- Copy native fallback crops and overlapping rendered pixels by contiguous RGBA rows instead of generic per-pixel image operations. The native renderer uses the same exact copies for tile cores and local patches.
- Upload and linearize only the image currently visible on the GPU canvas. Comparison requests both sides; holding the original shortcut updates that side on demand. Photo changes release hidden textures from the previous document. Full native pixels, half-float linear working textures, aspect ratio, and filtering are preserved.

These changes do not alter retouch algorithms, source pixels, edit history, or exported dimensions. New crop copies are checked byte-for-byte against the previous image operations, including transparency, clipped edges, and extra underlying storage.

## Measurement method

`ui::tests::profile_image_panning` replays actual raw egui pointer input through the production UI, real renderer workers, and the application's GPU callback. It measures input processing through completion of an offscreen GPU frame, excluding OS input delivery and physical display refresh. Run it alone after compilation and inference have finished.

The same bundled portrait is tested at 1122×1402, 2244×2804, and 3366×4206 pixels, at Fit, 62%, and 100%. Each case replays 128 frames with a 256-point horizontal excursion and 64-point vertical oscillation. Both Hand and Space with Heal selected are tested; the latter also has an exposure adjustment. The canvas is 1440×940 on Intel UHD / Vulkan. Mask overlay and comparison are off for these timings; comparison has a separate GPU pixel/upload regression test.

The original implementation lost the first 5.09 points of movement and showed 47–64 ms stalls when a large portrait needed a new sharp crop. CPU crop/overlap copies accounted for roughly 28–46 ms and GPU work roughly 17–25 ms on those frames. The baseline test executable and raw CSVs are preserved under `output/panning/`.

## Validation and results

Qualification results and the current installer are recorded here after the measured run.
