# Developer verification tools

These Cargo examples are developer diagnostics and are excluded from the Windows installer.

Keep `ai_smoke`, `runtime_validation`, `hardware_validation`, `nullstate_validation`, and `paired_model_validation` for AI checks. Keep `feedback_latency`, `live_tools_performance`, `viewport_performance`, `gpu_upload_profile`, `performance`, `high_resolution`, and `history_performance` for measured performance and pixel parity. `asset_pipeline`, `manual_tools_review`, `layer_stack_case`, and `portrait_case` support workflow and visual reviews. `training_faces` is used by the paired-training workflow.

The old `smoke` export example is superseded by the installed executable's verification mode. The one-off `display_profile` and `color_consistency` programs were removed. The duplicate `example-img/demo-portrait.png` was removed; maintained reviews use the embedded source in `assets/demo-portrait.png`. That selected portrait remains the startup splash asset.
