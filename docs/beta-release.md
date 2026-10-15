# October 6 beta preparation

Target: Hastur Retouch `0.1.0-beta.1`, Windows x64 installer for a public beta. Feature additions are frozen while packaging and release qualification finish. Feedback goes to `suboptimald1@gmail.com`. The publisher label is Hastur Retouch; it does not imply a verified Windows signing identity.

## Build the installer

Install the official [Inno Setup 6.7.3](https://jrsoftware.org/isdl.php) compiler and use Python 3.11+ only on the build machine. Its original download was checked against SHA-256 `9c73c3bae7ed48d44112a0f48e66742c00090bdb5bef71d9d3c056c66e97b732` and a valid Pyrsys B.V. Authenticode signature before execution. The local compiler is under `output/beta-prep/tools/inno`.

```powershell
$env:CARGO_PROFILE_RELEASE_LTO = 'false'
.\scripts\package-beta.ps1 -Iscc 'C:\path\to\Inno Setup 6\ISCC.exe' -Python python
```

`-SkipBuild` can package an already verified binary; the script requires its embedded ProductVersion to match Cargo.toml. Every build has a fresh output directory under `output/beta/0.1.0-beta.1/`. The artifacts directory contains the setup EXE, SHA256SUMS, release-manifest, quick start, beta notes, and installed verification script. The payload is a staging directory, not an extra download requirement.

`release/package-files.json` is an explicit allowlist with pinned SHA-256 values for ONNX models, runtimes and their notices. Build-generated Burn packs are checked against `models/burn/provenance.txt`, which records both the ONNX input hash and resulting pack hash. Every payload file is then hashed in `installed-files.json`. The package excludes `examples`, loose portraits, checkpoints, training datasets, archives of setup tools, developer scripts, caches and saved workspaces. The selected splash portrait and icon are embedded in the application. The trained generator is unchanged from the authorized DAY 2 adaptation; distribution rights were confirmed by the project owner on October 4.

The installer runs per user without administrator elevation, installs under LocalAppData/Programs, and offers optional shortcuts. Microsoft VC143 x64 runtime DLLs are deployed beside the EXE from the licensed Visual Studio redistributable folder, with signature verification and PE import checks. [Microsoft documents app-local deployment](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files?view=msvc-170); these copies must be kept current by rebuilding the installer. No VC runtime installation or reboot is required for this package.

`HASTUR_MODELS_DIR` explicitly overrides all model discovery. Installed binaries otherwise use their own models folder. Only binaries within the actual source checkout's target directory retain the developer fallback. AI caches now use AppData; older caches remain read fallbacks. Uninstall removes installer-owned program files and preserves AppData workspace data and external photos/exports.

Dependency notices are collected from the active locked Windows Cargo graph, including build dependencies. Missing local license files are fetched from the exact Git revision recorded in the published crate, not a moving branch. Three crates without their own license text use explicitly identified SPDX standard texts plus their original license declarations. Unmodified MPL dependency source archives are included. The collected text, JSON inventory and package hashes provide reviewable provenance rather than a legal certification.

## Signing

No code-signing certificate was available during preparation. An unsigned build records `NotSigned` in its release manifest and can trigger Windows protection prompts. It does not alter security settings. Do not describe it as signed or instruct testers to disable Windows protection.

When an owned, trusted code-signing certificate is available in the user's Windows certificate store, use:

```powershell
.\scripts\package-beta.ps1 -Iscc 'C:\path\to\ISCC.exe' -Python python `
  -SignTool 'C:\path\to\signtool.exe' -CertificateThumbprint 'YOUR_CERTIFICATE_THUMBPRINT'
```

The build signs the application and uses Inno's SignTool hook for setup and uninstall, with SHA-256 and a timestamp. There is no self-signed publisher substitution. A new signed release can still receive SmartScreen reputation warnings; see [Microsoft's guidance](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).

## Qualification

Install the generated EXE into a new folder with spaces, using a normal Windows account. Run this on a clean Windows PC/VM with no source checkout, Rust or Python. Disconnect the network while testing local editing and AI.

```powershell
.\verify-installed-beta.ps1 -InstallDirectory "$env:LOCALAPPDATA\Programs\Hastur Retouch Beta" `
  -OutputDirectory "$env:USERPROFILE\Desktop\Hastur beta QA"
```

Use a new empty output directory. `-IncludeBurn` also verifies actual Burn inference; it can take several minutes. The script validates every installed payload hash and runs the installed executable from an unrelated directory. It isolates recovery, preferences and caches from real saved workspaces, checks CPU inference, Auto with a CPU-only runtime (GPU fallback), Auto with the bundled DirectML runtime, and captures native/compact workspaces. The executable check covers each manual tool at original dimensions, mask hide/reveal, exact undo/redo restoration, saved history and sessions, lossless PNG export, and interrupted-generation recovery. Only the embedded generated portrait is used. It writes no user photo.

`verification.json` deliberately does not claim clean-machine qualification: a local script cannot establish that development tools are absent or the host is a clean VM. Record the clean test environment and result separately.

Finish these interactive checks on that clean host:

- Launch from Start as a standard user; confirm startup and local AI work offline.
- Import a personal test JPEG/PNG/TIFF and a high-resolution portrait; check EXIF orientation and file paths containing spaces/non-ASCII text.
- Zoom/pan and inspect at 100%. Drag all four tools at Fit and 100%; confirm feedback appears during the gesture, aligns with the cursor, and settles without blur.
- For Clone, select a source; for Patch, finish the lasso and move to the donor. Check visible-layer sampling, masks, opacity, locking and hidden targets.
- Undo/redo gestures; save, close, reopen and inspect identical pixels/history. Interrupt an editing session after autosave and verify automatic restoration.
- Export original-size PNG and JPEG, inspect dimensions/sharpness and compare originals' hashes. PNG must preserve rendered pixels; JPEG is intentionally lossy.
- Reinstall/upgrade, launch again, then uninstall. Confirm shortcuts/installer registration are removed, while photos, exports and AppData data remain.
- Repeat on another GPU if available. Record actual provider/fallback, driver and available RAM; do not extrapolate local Intel UHD results to untested hardware.

Review the installer and its quick-start HTML, model/runtime licenses, feedback address, public release text and hashes before sharing. Keep development QA outputs and private fixtures out of the download. Commit and tag the verified source state; manifests must identify that state. No automatic public publishing is part of packaging.

## Scope and known limits

JPEG, PNG, TIFF, WebP and BMP; no camera RAW. Up to 64 photos per workspace. Auto Retouch is facial cleanup and does not change global contrast or shape. Shape/background controls remain explicit. First AI qualification and initial stroke/cache work can pause, and Burn CPU is experimental and substantially slower on the locally tested hardware. Complex scars/hair and difficult backgrounds can require manual repair. Color-managed preview/native equivalence and performance checks describe the measured fixtures, not every photograph or computer.

Schedule: October 4 freeze/version/package; October 5 clean-machine qualification and blocker fixes; October 6 public beta only after the applicable release gates pass.

The completed local preparation and explicitly pending checks are recorded in [beta validation](beta-validation.md).
