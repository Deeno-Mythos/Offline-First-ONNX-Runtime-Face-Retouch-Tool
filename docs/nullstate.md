# NullState portrait AI

NullState makes portrait inference demand driven. Importing a neutral photo, panning, zooming, changing color or using manual Liquify, Spot heal, Clone stamp and Patch does not prepare the bundled portrait models. The demo also opens with neutral settings. Restoring saved active portrait edits requests their required analysis so the restored preview remains correct. An enabled portrait adjustment asks for the smallest set of analysis it needs. Auto Retouch asks for all three portrait stages because its cleanup uses all three.

| Requested stage | Adjustments that need it | Models that can run |
| --- | --- | --- |
| Facial geometry and masks | Face shape, eyes, teeth, tone/redness, contour/highlight and flyaway cleanup | BlazeFace detector and 478-point face mesh |
| Neural skin retouch | Skin smoothing, under-eye bags, forehead wrinkles and laugh lines | Retouch generator, plus missing facial geometry |
| Automatic blemish cleanup | Automatic blemish removal | Blemish detection and local inpainting, plus missing facial geometry |

The stages share face geometry rather than detecting the same face separately for each feature. Once a stage is ready, moving its sliders uses the existing maps. Enabling another feature prepares only its missing dependencies. Disabled or zero-strength adjustments do not request inference. **Reanalyze portrait** is an explicit request to recompute all stages, regardless of current slider demand.

Project staging does not load AI or native pixels for inactive photographs. Switching away cancels that photo's preparation, releases generated RAM maps and native pixels, and retains its edits, explicit masks and reusable disk results. Reopening it restores only the stages required by its enabled edits. Export jobs prepare their own snapshot independently of the selected workspace photo. See [asset pipeline](asset-pipeline.md).

## Dormancy and resource lifetime

Model sessions are created on their first real use. The dedicated portrait worker releases its ONNX sessions and Burn model instances after 90 seconds without portrait work. This releases owned model resources; it does not erase edits, undo history or generated maps. The next required inference recreates the relevant model. Completed maps retained by a photo can still be used without reloading a model. The ONNX runtime library itself remains initialized, and allocator or graphics-driver caches can retain memory; zero resident model instances is not a promise that process memory returns to zero.

Existing full portrait caches remain readable. Partial analyses stay with their photo rather than replacing a complete disk-cache entry. Only a completed full analysis is written to the portrait disk cache. A neutral photo does not load that cache merely to prepare unused portrait effects.

Changing provider while portrait edits are active creates a new preparation request. Results and errors carry request identities so a late response from a superseded request cannot replace current preparation. Export waits for the analysis required by the enabled edits. Custom face/background segmentation remains an explicit separate operation and is not automatically loaded by the portrait pipeline.

## Reproducible validation

`examples/nullstate_validation.rs` uses the supplied demo portrait and real installed models. It verifies these transitions in one inference thread:

1. No demand creates no resident models and performs no inference.
2. Geometry runs only the detector and mesh.
3. Adding skin runs only the retouch generator.
4. Adding blemishes runs only detection and inpainting.
5. Repeating a completed demand performs no additional model calls.
6. Incremental results match a fresh full analysis, including rendered pixels.
7. Releasing idle resources drops resident model instances while retained analysis stays usable.
8. Full cache reuse after release performs no inference.
9. Reloading after release preserves the result.

Run the CPU path first, with other heavy builds stopped:

```powershell
$env:CARGO_PROFILE_RELEASE_LTO = 'false'
cargo run --release --locked -j 2 --example nullstate_validation
cargo run --release --locked -j 2 --example nullstate_validation -- --auto
```

The example writes phase timings, model call traces and resident counts to `output/nullstate-validation/cpu.md` or `auto.md`. Auto can include first-use GPU qualification and its CPU reference runs. Its first-use timings should not be compared directly with an already-warmed model.

## Verified on 2026-10-02

The real CPU and Auto/DirectML runs passed every assertion. Each request ran
only its expected model names; incremental, fresh-full and reloaded analysis
had exactly matching face metadata and map values. Custom-mask upgrades also
preserved the original model-space eye/hair protections. Incremental and fresh-full
renders were byte-identical. Releasing resources reduced the calling thread's
resident model count from five to zero. Reusing photo maps and the full cache
afterward ran no models and left the resident count at zero.

On this Intel UHD system, no-demand work measured 0.010 ms on CPU and 0.005 ms on Auto.
Reusing completed photo maps measured 0.002 ms on CPU and 0.005 ms on Auto.
The first Auto requests included CPU comparisons and GPU qualification;
fresh full analysis with qualified warm models took 2.058 seconds. These are
individual local measurements, rather than a guaranteed speedup. Cold first
use still has loading and inference costs, particularly during qualification.

The regression suite passed 123 tests, including neutral imports/manual/color
work, disabled adjustments, real under-eye slider demand, incremental upgrades,
undo/redo reuse, stale provider results, failed-request suppression, no-face
completion and preserved custom masks. The example and full reports are kept
with the project for rerunning on other hardware.

Clippy with warnings denied and formatting checks passed. The rebuilt desktop
executable was captured at startup and in the AI models panel: the demo opened
untouched, the splash retained the selected woman, and the models panel showed
the dormant state. Captures and reports are in `output/nullstate-validation/`.
