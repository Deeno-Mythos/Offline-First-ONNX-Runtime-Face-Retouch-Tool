# Sessions and workspace recovery

Use **File → Save session…** (`Ctrl+S`) to save a portable RON editing session, and **File → Open session…** to reopen it. The session contains editing instructions and original-file paths rather than copies of the photographs. Keep originals at those paths; moving the session itself does not relocate its sources. The embedded demo needs no external source file.

Sessions restore adjustments, disabled values, local masks and cleanup strokes, liquify gestures, clone sources, patch selections, ratings, selected photos, the active photo, zoom/pan, export crop centers, and before/after comparison. Both undo and redo branches are saved. Generated neural maps reload from the local AI cache or are recomputed; explicit masks and face metadata remain in the session. Older version 1 sessions open with empty history and default workspace state.

## Automatic recovery

Autosave queues a background write after two seconds without workspace changes, or after ten seconds of continuous changes. Changes made during a pending save remain dirty and trigger a later save. File shows whether recovery is pending, saving, or up to date. Normal shutdown synchronously flushes the current workspace, and an older queued write cannot replace that final generation.

On Windows, recovery files are stored in `%LOCALAPPDATA%\AstraRetouch\recovery`. `ASTRA_RECOVERY_DIR` overrides this directory. Each running app uses its own recovery file; workspaces still open in another Windows instance are excluded from the recovery list. Autosave retains the preceding completed generation as a `.previous` file and tries it automatically if the latest recovery cannot be read.

Starting without command-line photos opens **Continue a saved workspace** when recoverable work exists. Choose **Continue**, **Discard**, or **Start with current workspace**. **File → Recover workspace…** opens the same list later. The current workspace is saved before switching to another saved workspace. Recovered data is retired only after the replacement workspace has been saved successfully. Discard removes that recovery and its previous generation.

If an original is unavailable, opening reports the failure and restores the photos it can load. The saved recovery containing the missing photos remains available. Restore the missing files to their saved paths and recover again; there is no source-relocation picker. Keep a manually saved session and the originals together when archiving work.

## Undo and redo

Use `Ctrl+Z` and `Ctrl+Shift+Z`, or the toolbar arrows. A brush stroke or slider drag creates one undo step. Editing after undo creates a new branch and clears redo.

History holds up to 100 steps within a soft 64 MiB budget for unique action-vector allocations. It shares unchanged instruction lists, stores growing lists as shared prefixes in saved sessions, and retains the nearest step even if one instruction set exceeds that budget. Large changing stroke lists can retain fewer than 100 steps to stay within the budget. Source images and preview caches have separate memory costs. History benchmark timings measure instruction operations, not the time required to render the restored edit.

Regression coverage includes older sessions, all local tools, both history branches, atomic replacement, previous-generation recovery, missing and duplicate sources, saves during continued editing, and shutdown write ordering. See [goal validation](goal-validation.md) for final build and benchmark evidence.
