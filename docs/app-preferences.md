# Preferences and everyday shortcuts

Open **File → Preferences**, the header gear, or **Ctrl+,**. Settings are saved separately from photograph edits; applying them does not create an undo step or change exported pixels. **Apply** persists the draft and updates the app, **Cancel** and **Escape** leave the current settings in place, and **Reset defaults** resets the draft until applied. A failed write leaves the dialog and draft open with an error.

| Tab | Settings |
| --- | --- |
| Workspace | Interface size 85–135%, hover tooltips, reduced animation, filmstrip visibility, wheel zoom or vertical pan, zoom speed 25–200%, reversed scroll |
| Editing | Brush outline, mask-overlay default, radius 0.2–9% of the image's short edge, softness, strength |
| Workflow | Autosave on/off, 1–10 second quiet interval, automatic startup restoration |
| Export | JPEG/PNG, output size preset, JPEG quality 50–100, one or two simultaneous export jobs |
| Shortcuts | Record a key/modifier combination, clear it or restore its default for tools, file/view commands, built-in presets and layer actions |

Changing interface size preserves the native-pixel meaning of 100% photo zoom. Wheel pan moves vertically at enlarged zoom; Space+drag remains available for both axes. Applying brush defaults also updates the active brush. Mask-overlay and export controls can still be changed in the workspace for the current run. Startup restoration takes effect on the next launch.

Preferences live at `%LOCALAPPDATA%\HasturRetouch\preferences.ron` on Windows, or the XDG configuration directory when available. `HASTUR_PREFERENCES_PATH` overrides the complete file path. Saves use a synchronized temporary file and atomic replacement. Missing files use defaults; invalid files remain untouched until preferences are explicitly applied. Old or partial preference files inherit new defaults, and numeric values are bounded before use. Screenshot runs bypass saved preferences and recovery for reproducible captures.

## Commands and workspace

**Ctrl+K**, the header command icon, or **View → Find a command** opens the searchable command catalog. Search terms match tools, panels, file operations, presets, navigation, layers and comparison. Use **Up/Down**, **Enter**, or a mouse click. Unavailable actions are disabled; **Escape** dismisses the menu. Text input and dialogs suppress photo and brush shortcuts. Dialogs also block canvas pointer input.

In **Preferences → Shortcuts**, click a binding and press the desired combination. Conflicts with another command are rejected. **Clear** disables that command's binding; **Default** restores it. Saved bindings replace the old ones and appear in file menus and command hover help. Layer bindings remain scoped to the Layers panel. Space, Tab, backslash and Escape remain reserved for workspace navigation/cancellation. Other context controls such as ratings and brush size retain their documented keys.

| Shortcut | Action |
| --- | --- |
| F | Focus workspace: hide inspector and filmstrip; press again to restore their previous state |
| F6 | Toggle the filmstrip |
| [ / ] | Decrease/increase active brush radius |
| Shift+[ / Shift+] | Decrease/increase brush strength by 5 percentage points |
| M | Toggle the active brush's mask overlay |
| X | Toggle add/erase for mask brushes |
| Left / Right | Previous/next photograph, scrolling it into the filmstrip |
| Shift+Left / Shift+Right | Extend the selected photo range |
| 1–5 | Rate the active photograph |
| Shift+0 | Clear its rating |
| Ctrl+A / Ctrl+I | Select all photographs / invert photo selection |

Ratings appear on filmstrip labels and persist in saved sessions. **View** also provides focus and filmstrip actions in narrow windows. The keyboard-help window scrolls in small windows. File and the wide-window status bar show whether autosave is paused, saving, dirty or saved. Disabling autosave also disables the shutdown recovery write; existing saved workspaces remain available and manual **Ctrl+S** remains available.

## Validation

Raw-egui integration tests exercise Apply/Cancel/Reset, write failure, compact dialog bounds, modal stroke protection, command typing and execution, disabled locked-layer actions, command scrolling, focus restoration, actual warp radius/strength, text-editor shortcut isolation, range selection, ratings/session persistence, wheel speed/pan, native photo pixels after UI scaling, and paused/configured autosave. Preference tests cover atomic replacement, partial files, invalid-file preservation and numeric bounds.

Native screenshot options include `--preferences-preview`, the additional tab flags `--editing-preferences`, `--workflow-preferences`, `--export-preferences`, `--shortcuts-preferences`, and `--commands-preview`. `--focus-workspace` and `--hide-filmstrip` allow workspace layout captures.

On Windows, all 168 optimized tests passed with the default ONNX/Burn build, along with formatting and all-target Clippy checks with warnings denied. The release executable was rebuilt. Desktop captures of all four preference tabs, commands, focus mode and the 600×560 compact window are in `output/app-preferences/`; capture processes exited successfully with empty error logs. Release test and build logs are stored in the same directory.
