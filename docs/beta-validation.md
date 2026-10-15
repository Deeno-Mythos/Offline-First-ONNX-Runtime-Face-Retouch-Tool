# Beta preparation evidence — October 4, 2026

Version: `0.1.0-beta.1`. Target: Windows x64 public-beta installer. The project owner requested the installer and a clean-machine checklist for now because no second PC or clean VM is available. No public upload, announcement or email was sent.

| Requirement | Evidence and result |
| --- | --- |
| Versioned application and installer | Application PE ProductVersion is `0.1.0-beta.1`; installer numeric/text versions are generated from Cargo.toml. |
| Complete local dependencies | Explicit allowlist/provenance includes five ONNX models, five Burn packs, CPU and DirectML runtimes and application-local VC143 DLLs. 16 x64 PE binaries and VC imports verified. 47 payload files are hashed, plus the installed manifest. No Rust/Python is required on the target computer. |
| Installer behavior | Candidate setup compiled with Inno Setup 6.7.3; install and reinstall succeeded in `Installed beta with spaces` on a standard, unelevated account. Start-menu target and working directory matched. All reinstalled payload hashes matched. |
| Actual installed AI | Installed EXE ran from an unrelated folder with fresh isolated state. CPU, Auto with a CPU-only runtime, Auto with bundled DirectML, and Burn each produced one detected face plus neural/repair maps and passed. Auto visibly reported CPU fallback when the DML provider was unavailable. |
| Editing and sharp output | Each installed provider check also exercised all four manual tools, mask hide/reveal, exact undo/redo, saved sessions/history, native dimensions and byte-identical lossless PNG export. Native and compact workspace screenshots were captured and visually inspected. |
| Persistence/recovery | Interrupted recovery used the previous complete generation. AI cache save/reload succeeded outside program files. A missing runtime file was rejected before launching the app. |
| Uninstall preservation | Uninstaller exited 0 and removed program files, registration and shortcuts. Both an extra photo inside the installation and an AppData sentinel survived with identical hashes. The original splash portrait stayed unchanged. Temporary sentinels were then removed individually. |
| Regression/build gates | 211 optimized library tests passed; strict all-target release Clippy, formatting and whitespace checks passed. Installed verification correctly returned exit code 0 even after a prior shell command returned 19. |
| Licensing/provenance | 459 active Windows normal/build dependency notice sets collected. Missing upstream text is pinned to the crate's Git revision; standard-text fallbacks are identified. MPL source archives match Cargo.lock checksums. ONNX Runtime/DirectML/VC/Inno notices are included. The owner confirmed distribution rights for the trained model and splash/logo. |
| Example cleanup | Removed obsolete `smoke`, `display_profile`, `color_consistency`, duplicate `example-img/demo-portrait.png`, and tracked Python bytecode. Required review/performance/training helpers remain documented; none are installed. The selected portrait remains embedded from assets. |
| Feedback and guide | Help includes copied build version and feedback email/link; Quick-start.html explains import, tools, shortcuts, layers/masks, export, privacy and known limitations. Address: `suboptimald1@gmail.com`. Nothing is sent automatically. |

Source and installer identity are recorded in each artifact's `release-manifest.json` and `SHA256SUMS.txt`; the final build must report `gitDirty: false` and point to tag `v0.1.0-beta.1`.

Local check artifacts: `output/beta-prep/library-tests-final.log`, `clippy-final.log`, `app-build-final.log`, `Installed QA/verification.json`, provider `installation-check.txt` reports, native/compact PNGs, `verifier-exit-code.log`, `missing-runtime-result.txt`, and `uninstall-verification.json`. These development outputs are excluded from the public download. The final payload comparison ties these checks to the delivered installer.

Observed cold inference during functional qualification was CPU 4.688 s, qualified DirectML Auto 8.242 s, CPU-runtime Auto fallback 4.067 s and Burn CPU 118.943 s. These are single local functional runs, not a hardware benchmark. They support keeping Burn opt-in.

Still pending before claiming broad public-beta qualification: clean Windows/VM testing, disconnected-network testing on that host, another GPU/RAM configuration, and an owned code-signing certificate. The current installer is unsigned; the packaging script supports future signing of app/setup/uninstall. These limits are documented in the release checklist and are not represented as passed.

See [beta build and remaining test checklist](beta-release.md), [live feedback measurements](feedback-latency.md), and [portrait quality evidence](portrait-validation.md).
