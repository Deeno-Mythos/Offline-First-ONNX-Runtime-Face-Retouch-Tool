# Sessions and workspace recovery

Use **File → Save session…** (`Ctrl+S`) to save a portable RON editing session, and **File → Open session…** to reopen it. The session contains editing instructions and original-file paths rather than copies of the photographs. Keep originals at those paths; moving the session itself does not relocate its sources. The embedded demo needs no external source file.

Sessions restore adjustments, disabled values, local masks and cleanup strokes, liquify gestures, clone sources, patch selections, ratings, selected photos, the active photo, zoom/pan, export crop centers, and before/after comparison. Both undo and redo branches are saved. Generated neural maps reload from the local AI cache or are recomputed; explicit masks and face metadata remain in the session. Older version 1 sessions open with empty history and default workspace state.

## Automatic recovery

Autosave defaults to a background write after two seconds without workspace changes, or after ten seconds of continuous changes. **Preferences → Workflow** lets you pause autosave or set a quiet interval of 1–10 seconds; periodic saves during continued changes use five times that interval, with a ten-second minimum. Changes made during a pending save remain dirty and trigger a later save. File and the wide-window status bar show whether recovery is paused, pending, saving, or up to date. When autosave is enabled, normal shutdown synchronously flushes the current workspace, and an older queued write cannot replace that final generation. Pausing autosave preserves existing saved files and leaves manual session saving available.

On Windows, Hastur Retouch writes recovery files to `%LOCALAPPDATA%\HasturRetouch\recovery`. It also discovers existing workspaces in `%LOCALAPPDATA%\AstraRetouch\recovery` and opens them in place. The rename does not move or delete older recoveries; their previous generations and undo/redo history remain usable.

`HASTUR_RECOVERY_DIR` overrides recovery storage and discovery. The older `ASTRA_RECOVERY_DIR` remains supported when the new variable is absent. If both are set, the Hastur variable wins. An override restricts discovery to that directory. Each running app uses its own recovery file; workspaces still open in another Windows instance, including an older Astra instance, are excluded from the recovery list. Autosave retains the preceding completed generation as a `.previous` file and tries it automatically if the latest recovery cannot be read.

Starting without command-line photos automatically restores the newest usable saved workspace, including its undo and redo history. **Preferences → Workflow** can disable startup restoration. There is no recovery-choice dialog. Empty, corrupt, or wholly unavailable workspaces fall through to the previous generation and older unlocked recoveries. If none can load, the bundled demo opens. Explicit command-line photos and screenshot capture bypass automatic recovery. Skipped recovery files remain untouched, and recovered data is retired only after the replacement workspace has been saved successfully.

If an original is unavailable, opening reports the failure and restores the photos it can load. The saved recovery containing the missing photos remains available. Restore the missing files to their saved paths and recover again; there is no source-relocation picker. Keep a manually saved session and the originals together when archiving work.

## Undo and redo

Use `Ctrl+Z` and `Ctrl+Shift+Z`, or the toolbar arrows. A brush stroke or slider drag creates one undo step. Editing after undo creates a new branch and clears redo.

History holds up to 100 steps within a soft 64 MiB budget for unique action-vector allocations. It shares unchanged instruction lists, stores growing lists as shared prefixes in saved sessions, and retains the nearest step even if one instruction set exceeds that budget. Large changing stroke lists can retain fewer than 100 steps to stay within the budget. Source images and preview caches have separate memory costs. History benchmark timings measure instruction operations, not the time required to render the restored edit.

Manual sessions and presets keep their existing RON formats. AI caches are now written to `.hastur-cache/` beside `models/`; existing `.astra-cache/` files are read as a fallback, including when a newer cache is unreadable. Legacy cache files are retained rather than moved or pruned during this rename.

Regression coverage includes older sessions, all local tools, both history branches, atomic replacement, previous-generation recovery, legacy recovery discovery and cache fallback, override precedence, missing and duplicate sources, saves during continued editing, and shutdown write ordering. See [goal validation](goal-validation.md) for final build and benchmark evidence.
