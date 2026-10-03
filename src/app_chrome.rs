//! App-wide preferences, command search and small workflow conveniences.
use super::layer_panel::{Icon, paint_icon};
use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum PreferencesTab {
    #[default]
    Workspace,
    Editing,
    Workflow,
    Export,
    Shortcuts,
}
impl PreferencesTab {
    const ALL: [Self; 5] = [
        Self::Workspace,
        Self::Editing,
        Self::Workflow,
        Self::Export,
        Self::Shortcuts,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Workspace => "Workspace",
            Self::Editing => "Editing",
            Self::Workflow => "Workflow",
            Self::Export => "Export",
            Self::Shortcuts => "Shortcuts",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AppCommand {
    Preferences,
    Import,
    ImportFolder,
    ExportQueue,
    Commands,
    AutoRetouch,
    Preset(usize),
    OpenSession,
    SaveSession,
    Export,
    Undo,
    Redo,
    Fit,
    Native,
    Compare,
    Focus,
    Filmstrip,
    Tool(Tool),
    Brush(Target),
    StopBrush,
    History,
    Layers,
    ToolPanel,
    HidePanels,
    DeleteAsset,
    Layer(LayerAction),
    NewLayer(LayerType),
    Previous,
    Next,
    SelectAll,
    InvertSelection,
    Shortcuts,
}
struct Command {
    action: AppCommand,
    name: &'static str,
    shortcut: &'static str,
    icon: Icon,
}
fn commands() -> Vec<Command> {
    use AppCommand::*;
    let mut commands: Vec<_> = [
        (Preferences, "Preferences", "Ctrl+,", Icon::Gear),
        (Import, "Import photos", "Ctrl+O", Icon::Image),
        (ImportFolder, "Import folder", "", Icon::Image),
        (ExportQueue, "Show export queue", "", Icon::Image),
        (Commands, "Find a command", "Ctrl+K", Icon::Command),
        (AutoRetouch, "Auto retouch", "A", Icon::Retouch),
        (
            Preset(0),
            "Apply Natural Headshot preset",
            "",
            Icon::Retouch,
        ),
        (Preset(1), "Apply Wedding Soft preset", "", Icon::Retouch),
        (
            Preset(2),
            "Apply Studio Editorial preset",
            "",
            Icon::Retouch,
        ),
        (Preset(3), "Apply E-commerce preset", "", Icon::Retouch),
        (Preset(4), "Apply Moody Portrait preset", "", Icon::Retouch),
        (OpenSession, "Open saved session", "", Icon::Image),
        (SaveSession, "Save session", "Ctrl+S", Icon::Image),
        (Export, "Export photos", "Ctrl+E", Icon::Image),
        (Undo, "Undo last edit", "Ctrl+Z", Icon::Reset),
        (Redo, "Redo edit", "Ctrl+Shift+Z", Icon::Reset),
        (Fit, "Fit photo in workspace", "0", Icon::Focus),
        (Native, "View at 100% · native detail", "", Icon::Search),
        (
            Compare,
            "Toggle before / after comparison",
            "B",
            Icon::Adjust,
        ),
        (Focus, "Toggle focus workspace", "F", Icon::Focus),
        (Filmstrip, "Show / hide filmstrip", "F6", Icon::Filmstrip),
        (History, "Show history panel", "", Icon::Stack),
        (Layers, "Show layers panel", "", Icon::Stack),
        (ToolPanel, "Show tools panel", "", Icon::Retouch),
        (HidePanels, "Hide panels", "", Icon::Focus),
        (DeleteAsset, "Delete from Project", "", Icon::Trash),
        (
            NewLayer(LayerType::Retouch),
            "New retouch layer",
            "Ctrl+Shift+N",
            Icon::Retouch,
        ),
        (
            NewLayer(LayerType::Adjustment),
            "New adjustment layer",
            "",
            Icon::Adjust,
        ),
        (
            Brush(Target::Liquify),
            "Use liquify brush",
            "",
            Icon::Liquify,
        ),
        (
            Brush(Target::Heal),
            "Use spot healing brush",
            "",
            Icon::Heal,
        ),
        (
            Brush(Target::Clone),
            "Use clone stamp brush",
            "",
            Icon::Clone,
        ),
        (Brush(Target::Patch), "Use patch tool", "", Icon::Patch),
        (StopBrush, "Stop active brush", "Escape", Icon::Close),
        (Previous, "Previous photo", "Left", Icon::Up),
        (Next, "Next photo", "Right", Icon::Down),
        (SelectAll, "Select all photos", "Ctrl+A", Icon::Filmstrip),
        (
            InvertSelection,
            "Invert photo selection",
            "Ctrl+I",
            Icon::Filmstrip,
        ),
        (Shortcuts, "Keyboard shortcuts", "", Icon::Keyboard),
        (
            Layer(LayerAction::Duplicate),
            "Duplicate layer",
            "Ctrl+J",
            Icon::Duplicate,
        ),
        (
            Layer(LayerAction::Delete),
            "Delete layer",
            "Delete",
            Icon::Trash,
        ),
        (
            Layer(LayerAction::Rename),
            "Rename layer",
            "F2",
            Icon::Rename,
        ),
        (
            Layer(LayerAction::Move(true)),
            "Move layer up",
            "Ctrl+]",
            Icon::Up,
        ),
        (
            Layer(LayerAction::Move(false)),
            "Move layer down",
            "Ctrl+[",
            Icon::Down,
        ),
        (
            Layer(LayerAction::Visibility),
            "Show / hide active layer",
            "",
            Icon::Eye,
        ),
        (
            Layer(LayerAction::Lock),
            "Lock / unlock active layer",
            "",
            Icon::Lock,
        ),
        (
            Layer(LayerAction::Solo),
            "Solo / restore active layer",
            "",
            Icon::Solo,
        ),
        (
            Layer(LayerAction::RestoreSolo),
            "Restore all layers",
            "",
            Icon::Stack,
        ),
        (
            Layer(LayerAction::CopySettings),
            "Copy layer adjustments",
            "",
            Icon::Copy,
        ),
        (
            Layer(LayerAction::PasteSettings),
            "Paste layer adjustments",
            "",
            Icon::Paste,
        ),
        (
            Layer(LayerAction::ResetSettings),
            "Reset layer adjustments",
            "",
            Icon::Reset,
        ),
    ]
    .into_iter()
    .map(|(action, name, shortcut, icon)| Command {
        action,
        name,
        shortcut,
        icon,
    })
    .collect();
    for (color, name) in [
        (
            crate::layer_stack::LayerColor::None,
            "Clear layer color label",
        ),
        (crate::layer_stack::LayerColor::Red, "Label layer red"),
        (crate::layer_stack::LayerColor::Orange, "Label layer orange"),
        (crate::layer_stack::LayerColor::Yellow, "Label layer yellow"),
        (crate::layer_stack::LayerColor::Green, "Label layer green"),
        (crate::layer_stack::LayerColor::Blue, "Label layer blue"),
        (crate::layer_stack::LayerColor::Violet, "Label layer violet"),
    ] {
        commands.push(Command {
            action: Layer(LayerAction::Color(color)),
            name,
            shortcut: "",
            icon: Icon::Tag,
        });
    }
    for (tool, name, icon) in [
        (super::Tool::Portrait, "Open portrait tools", Icon::Retouch),
        (super::Tool::Skin, "Open skin tone tools", Icon::Retouch),
        (
            super::Tool::Color,
            "Open color and light tools",
            Icon::Adjust,
        ),
        (
            super::Tool::Background,
            "Open background tools",
            Icon::Image,
        ),
        (super::Tool::Presets, "Open presets", Icon::Stack),
        (super::Tool::Batch, "Open batch tools", Icon::Filmstrip),
        (super::Tool::Models, "Open AI models", Icon::Gear),
    ] {
        commands.push(Command {
            action: AppCommand::Tool(tool),
            name,
            shortcut: "",
            icon,
        });
    }
    commands
}

fn binding(command: &Command, prefs: &Preferences) -> Option<crate::shortcuts::Shortcut> {
    prefs
        .shortcuts
        .get(command.name)
        .cloned()
        .unwrap_or_else(|| crate::shortcuts::Shortcut::parse(command.shortcut))
}
fn shortcut_editor(
    ui: &mut egui::Ui,
    prefs: &mut Preferences,
    capture: &mut Option<String>,
    error: &mut Option<String>,
) {
    note(
        ui,
        "Click a binding, then press a key combination. Space, Tab, backslash and Escape remain workspace navigation keys.",
    );
    let catalog = commands();
    if let Some(name) = capture.clone()
        && let Some((key, modifiers)) = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    modifiers,
                    pressed: true,
                    repeat: false,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
        })
    {
        if key == egui::Key::Escape {
            *capture = None;
        } else {
            let candidate = crate::shortcuts::Shortcut::capture(key, modifiers);
            if !candidate.valid() {
                *error = Some("That key is reserved for workspace navigation.".into());
            } else if let Some(other) = catalog
                .iter()
                .find(|c| c.name != name && binding(c, prefs).as_ref() == Some(&candidate))
            {
                *error = Some(format!(
                    "{} is already assigned to {}. Clear or change that binding first.",
                    candidate.label(),
                    other.name
                ));
            } else {
                prefs.shortcuts.insert(name, Some(candidate));
                *capture = None;
                *error = None;
            }
        }
    }
    for command in catalog {
        ui.push_id(command.name, |ui| {
            ui.horizontal(|ui| {
                ui.add_sized(vec2(220., 30.), egui::Label::new(command.name).wrap());
                let text = if capture.as_deref() == Some(command.name) {
                    "Press keys…".into()
                } else {
                    binding(&command, prefs).map_or("Unassigned".into(), |b| b.label())
                };
                let recording = capture.as_deref();
                let r = ui.add_enabled(
                    recording.is_none() || recording == Some(command.name),
                    egui::Button::new(text).min_size(vec2(116., 30.)),
                );
                hit(ui, &format!("binding-{}", command.name), r.rect);
                if r.clicked() && capture.is_none() {
                    *capture = Some(command.name.into());
                    *error = None;
                }
                if ui.small_button("Clear").clicked() {
                    prefs.shortcuts.insert(command.name.into(), None);
                }
                if ui.small_button("Default").clicked() {
                    prefs.shortcuts.remove(command.name);
                }
            })
        });
    }
}

fn check(ui: &mut egui::Ui, value: &mut bool, label: &str, key: &str) {
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size.y = 34.;
            ui.add(egui::Checkbox::new(value, label))
        })
        .inner;
    hit(ui, key, response.rect);
}
fn numeric(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    min: f32,
    max: f32,
    suffix: &str,
    key: &str,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        let response = ui.add(
            egui::DragValue::new(value)
                .range(min..=max)
                .speed(0.25)
                .suffix(suffix),
        );
        hit(ui, key, response.rect);
    });
}
fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(10.).color(MUTED));
}
fn pressed(ctx: &egui::Context, modifiers: egui::Modifiers, key: egui::Key) -> bool {
    ctx.input_mut(|i| i.modifiers.matches_exact(modifiers) && i.consume_key(modifiers, key))
}

impl HasturApp {
    pub(super) fn command_label(&self, action: AppCommand, text: &str) -> String {
        commands()
            .iter()
            .find(|c| c.action == action)
            .and_then(|c| binding(c, &self.preferences))
            .map_or_else(|| text.to_owned(), |b| format!("{text}    {}", b.label()))
    }
    pub(super) fn apply_preferences(&mut self, ctx: &egui::Context) {
        self.preferences = self.preferences.clone().normalized();
        ctx.set_zoom_factor(self.preferences.ui_scale);
        ctx.style_mut(|s| {
            s.interaction.tooltip_delay = if self.preferences.tooltips {
                0.5
            } else {
                f32::INFINITY
            };
            s.interaction.tooltip_grace_time = if self.preferences.tooltips { 0.3 } else { 0. };
            s.animation_time = if self.preferences.reduced_motion {
                0.
            } else {
                0.18
            }
        });
        if let Some((_, _, filmstrip)) = &mut self.focus_restore {
            *filmstrip = self.preferences.show_filmstrip;
        } else {
            self.filmstrip_visible = self.preferences.show_filmstrip;
        }
        self.show_mask = self.preferences.mask_overlay;
        self.brush_radius = self.preferences.brush_radius_percent / 100.;
        self.brush_softness = self.preferences.brush_softness / 100.;
        self.brush_strength = self.preferences.brush_strength;
        self.export_png = self.preferences.export_png;
        self.export_size = self.preferences.export_size;
        self.quality = self.preferences.jpeg_quality;
        ctx.request_repaint();
    }
    pub(super) fn open_preferences(&mut self) {
        self.shortcut_capture = None;
        self.end_canvas_gesture();
        self.command_open = false;
        self.export_open = false;
        self.help = false;
        self.preferences_draft = self.preferences.clone();
        self.preferences_error = None;
        self.preferences_open = true;
    }
    fn commit_preferences(&mut self, ctx: &egui::Context) -> bool {
        let draft = self.preferences_draft.clone().normalized();
        if let Some(path) = &self.preferences_path
            && let Err(error) = preferences::save(path, &draft)
        {
            self.preferences_error = Some(format!("Could not save preferences: {error}"));
            return false;
        }
        self.preferences = draft;
        self.apply_preferences(ctx);
        self.preferences_open = false;
        self.notify("Preferences applied");
        true
    }
    pub(super) fn open_commands(&mut self) {
        self.end_canvas_gesture();
        self.preferences_open = false;
        self.export_open = false;
        self.help = false;
        self.command_open = true;
        self.command_query.clear();
        self.command_index = 0;
        self.command_focus = true;
    }
    pub(super) fn toggle_focus(&mut self) {
        self.end_canvas_gesture();
        if let Some((panel, compact, filmstrip)) = self.focus_restore.take() {
            self.inspector = panel;
            self.compact_open = compact;
            self.filmstrip_visible = filmstrip;
        } else {
            self.focus_restore = Some((self.inspector, self.compact_open, self.filmstrip_visible));
            self.inspector = None;
            self.compact_open = false;
            self.filmstrip_visible = false;
        }
    }
    pub(super) fn select_photo(&mut self, index: usize, shift: bool, control: bool) {
        if index >= self.photos.len() {
            return;
        }
        self.end_canvas_gesture();
        let mut flags: Vec<_> = self.photos.iter().map(|p| p.selected).collect();
        interaction::select(
            &mut flags,
            &mut self.current,
            &mut self.selection_anchor,
            index,
            shift,
            control,
        );
        for (photo, selected) in self.photos.iter_mut().zip(flags) {
            photo.selected = selected;
        }
        self.patch_draft = PatchDraft::default();
        self.photo_focus = true;
        if self
            .detail_region
            .as_ref()
            .is_some_and(|(id, _, _, _)| *id != self.photos[self.current].id)
        {
            self.detail_region = None;
            self.detail_tile = None;
        }
    }
    fn command_available(&self, action: AppCommand) -> bool {
        use AppCommand::*;
        let photo = self.photos.get(self.current);
        match action {
            Import => !self.importing && self.photos.len() < 64,
            OpenSession => !self.importing,
            SaveSession => !self.importing && photo.is_some(),
            Export => photo.is_some(),
            AutoRetouch => photo.is_some_and(|p| !p.edit.layer_locked()),
            Preset(index) => {
                index < self.presets.len() && photo.is_some_and(|p| !p.edit.layer_locked())
            }
            Undo => photo.is_some_and(|p| p.history.can_undo()),
            Redo => photo.is_some_and(|p| p.history.can_redo()),
            Fit | Native | Compare | Previous | Next | SelectAll | InvertSelection => {
                photo.is_some()
            }
            Brush(_) => photo.is_some_and(|p| !p.edit.layer_locked()),
            StopBrush => self.brush.is_some(),
            NewLayer(_) => photo.is_some_and(|p| p.edit.stack.layers.len() < 32),
            DeleteAsset => photo.is_some(),
            Layer(action) => {
                self.inspector == Some(Inspector::Layers)
                    && photo.is_some_and(|p| {
                        p.edit.active_layer().is_some()
                            && match action {
                                LayerAction::Duplicate => p.edit.stack.layers.len() < 32,
                                LayerAction::PasteSettings => {
                                    !p.edit.layer_locked() && self.layer_clipboard.is_some()
                                }
                                LayerAction::Delete
                                | LayerAction::Move(_)
                                | LayerAction::ResetSettings => !p.edit.layer_locked(),
                                _ => true,
                            }
                    })
            }
            _ => true,
        }
    }
    pub(super) fn execute_command(&mut self, ctx: &egui::Context, action: AppCommand) {
        if !self.command_available(action) {
            return;
        }
        use AppCommand::*;
        self.command_open = false;
        match action {
            Preferences => self.open_preferences(),
            Import => self.import(),
            ImportFolder => self.import_folder(),
            ExportQueue => self.export_queue_open = true,
            Commands => self.open_commands(),
            ToolPanel => {
                self.inspector = Some(Inspector::Tools);
                self.last_inspector = Inspector::Tools;
                self.compact_open = true;
            }
            HidePanels => {
                self.inspector = None;
                self.compact_open = false;
            }
            DeleteAsset => self.delete_from_project(self.photos[self.current].id),
            Layer(action) => self.layer_action(self.photos[self.current].edit.stack.active, action),
            AutoRetouch => {
                self.end_canvas_gesture();
                let photo = &mut self.photos[self.current];
                photo.history.record(photo.edit.clone());
                photo.edit.settings.apply_auto_retouch();
                photo.edit.preset = None;
                self.edited();
            }
            Preset(index) => self.apply_preset(index),
            OpenSession => self.open_session(),
            SaveSession => self.save_session(),
            Export => self.export_open = true,
            Undo => self.undo(ctx),
            Redo => self.redo(ctx),
            Fit => {
                if let Some(p) = self.photos.get_mut(self.current) {
                    p.view = View::default();
                }
            }
            Native => {
                let viewport = self.canvas_viewport.unwrap_or(ctx.content_rect());
                if let Some(p) = self.photos.get_mut(self.current) {
                    p.view.zoom_at(1., viewport.center(), viewport);
                }
            }
            Compare => self.compare = !self.compare,
            Focus => self.toggle_focus(),
            Filmstrip => self.filmstrip_visible = !self.filmstrip_visible,
            History | Layers => {
                let panel = if matches!(action, History) {
                    Inspector::History
                } else {
                    Inspector::Layers
                };
                self.inspector = Some(panel);
                self.last_inspector = panel;
                self.compact_open = true;
                self.history_focus = true;
            }
            Tool(tool) => {
                self.tool = tool;
                self.inspector = Some(Inspector::Tools);
                self.last_inspector = Inspector::Tools;
                self.compact_open = true;
                self.brush = None;
            }
            Brush(target) => {
                self.end_canvas_gesture();
                if self.brush != Some(target) {
                    self.toggle_brush(target);
                }
                self.tool = super::Tool::Portrait;
                self.inspector = Some(Inspector::Tools);
                self.last_inspector = Inspector::Tools;
                self.compact_open = true;
            }
            StopBrush => {
                self.brush = None;
                self.end_canvas_gesture();
                self.patch_draft = PatchDraft::default();
            }
            NewLayer(kind) => {
                let id = self.photos[self.current].edit.stack.active;
                self.layer_action(id, LayerAction::New(kind));
                self.inspector = Some(Inspector::Layers);
                self.last_inspector = Inspector::Layers;
                self.compact_open = true;
            }
            Previous => self.select_photo(self.current.saturating_sub(1), false, false),
            Next => self.select_photo((self.current + 1).min(self.photos.len() - 1), false, false),
            SelectAll => {
                for photo in &mut self.photos {
                    photo.selected = true;
                }
            }
            InvertSelection => {
                for photo in &mut self.photos {
                    photo.selected = !photo.selected;
                }
            }
            Shortcuts => {
                self.open_preferences();
                self.preferences_tab = PreferencesTab::Shortcuts;
            }
        }
    }

    pub(super) fn preferences_dialog(&mut self, ctx: &egui::Context) {
        if !self.preferences_open {
            return;
        }
        if !egui::Popup::is_any_open(ctx)
            && self.shortcut_capture.is_none()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.preferences_open = false;
            return;
        }
        let mut apply = false;
        let mut cancel = false;
        let mut reset = false;
        let modal_id = egui::Id::new("app-preferences");
        let modal_width = (ctx.content_rect().width() - 70.).clamp(260., 540.) + 36.;
        // A stable origin keeps key-binding rows under the pointer when help or
        // error text changes the dialog height.
        let area = egui::Area::new(modal_id)
            .kind(egui::UiKind::Modal)
            .sense(Sense::hover())
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(
                ctx.content_rect().center().x - modal_width / 2.,
                ctx.content_rect().top() + 24.,
            ));
        let response=egui::Modal::new(modal_id).area(area).frame(egui::Frame::popup(&ctx.style()).fill(PANEL).inner_margin(18).corner_radius(14)).show(ctx,|ui| {
            ui.set_width((ctx.content_rect().width()-70.).clamp(260.,540.));
            ui.horizontal(|ui| {
                let (r,_)=ui.allocate_exact_size(vec2(24.,24.),Sense::hover());paint_icon(ui.painter(),r,Icon::Gear,YELLOW);
                ui.label(RichText::new("Preferences").size(19.).strong());
            });
            note(ui,"App settings are separate from photo adjustments.");ui.add_space(8.);
            ui.horizontal_wrapped(|ui| {
                for tab in PreferencesTab::ALL {
                    let response=ui.add(egui::Button::new(tab.label()).selected(self.preferences_tab==tab).min_size(vec2(64.,32.)));
                    hit(ui,&format!("preferences-tab-{}",tab.label()),response.rect);
                    if response.clicked() {self.preferences_tab=tab;}
                }
            });ui.separator();
            // Reserve a stable scroll viewport independently of Area's previous
            // frame size. Otherwise its available height can grow every frame,
            // moving the footer away while the pointer is clicking Apply.
            let body_height = (ctx.content_rect().height()-280.).clamp(90.,460.);
            egui::ScrollArea::vertical().id_salt("preferences-scroll").auto_shrink([false,false]).min_scrolled_height(body_height).max_height(body_height).show(ui,|ui| {
                let draft=&mut self.preferences_draft;
                match self.preferences_tab {
                    PreferencesTab::Workspace=>{
                        let mut scale=draft.ui_scale*100.;numeric(ui,"Interface size",&mut scale,85.,135.,"%","preferences-scale");draft.ui_scale=scale/100.;
                        note(ui,"100% photo zoom still uses native display pixels at every interface size.");
                        check(ui,&mut draft.tooltips,"Show hover tooltips","preferences-tooltips");
                        check(ui,&mut draft.reduced_motion,"Reduce UI animations","preferences-motion");
                        check(ui,&mut draft.show_filmstrip,"Show filmstrip when opening a workspace","preferences-filmstrip");
                        ui.separator();check(ui,&mut draft.wheel_zoom,"Mouse wheel zooms the photo","preferences-wheel");
                        note(ui,"Turn off to pan vertically at enlarged zoom.");
                        let mut speed=draft.zoom_speed*100.;numeric(ui,"Wheel zoom speed",&mut speed,25.,200.,"%","preferences-zoom-speed");draft.zoom_speed=speed/100.;
                        check(ui,&mut draft.invert_scroll,"Reverse scroll direction","preferences-invert-scroll");
                    }
                    PreferencesTab::Editing=>{
                        check(ui,&mut draft.brush_outline,"Show brush outline","preferences-outline");
                        check(ui,&mut draft.mask_overlay,"Show painted mask overlay by default","preferences-overlay");
                        ui.separator();ui.label(RichText::new("Brush defaults").strong());
                        numeric(ui,"Radius · short image edge",&mut draft.brush_radius_percent,0.2,9.,"%","preferences-radius");
                        numeric(ui,"Softness",&mut draft.brush_softness,0.,100.,"%","preferences-softness");
                        numeric(ui,"Strength",&mut draft.brush_strength,1.,100.,"%","preferences-strength");
                        note(ui,"[ / ]: size  ·  Shift+[ / ]: strength  ·  M: mask overlay  ·  X: add/erase masks");
                    }
                    PreferencesTab::Workflow=>{
                        check(ui,&mut draft.autosave,"Save workspace automatically","preferences-autosave");
                        ui.add_enabled_ui(draft.autosave,|ui| {
                            ui.horizontal(|ui| {ui.label("Save after editing pauses for");let r=ui.add(egui::DragValue::new(&mut draft.autosave_seconds).range(1..=10).suffix(" s"));hit(ui,"preferences-autosave-seconds",r.rect);});
                        });
                        note(ui,"Continued edits also save periodically. File → Save session is always available.");
                        check(ui,&mut draft.restore_workspace,"Continue the latest saved workspace at startup","preferences-restore");
                        note(ui,"Startup changes take effect on the next launch. Existing workspaces remain saved.");
                    }
                    PreferencesTab::Shortcuts=>shortcut_editor(ui,draft,&mut self.shortcut_capture,&mut self.preferences_error),
                    PreferencesTab::Export=>{
                        ui.horizontal(|ui| {ui.label("Simultaneous export jobs");ui.add(egui::DragValue::new(&mut draft.export_concurrency).range(1..=2));});
                        note(ui,"One job uses less memory. Two jobs export different projects concurrently.");
                        ui.label(RichText::new("Default export options").strong());
                        ui.horizontal(|ui| {ui.label("Format");for (name,png) in [("JPEG",false),("PNG",true)] {let r=ui.selectable_value(&mut draft.export_png,png,name);hit(ui,&format!("preferences-format-{name}"),r.rect);}});
                        let size=egui::ComboBox::from_id_salt("preferences-export-size").selected_text(draft.export_size.label()).show_ui(ui,|ui| {
                            for size in [ExportSize::Original,ExportSize::Web,ExportSize::Instagram,ExportSize::Story,ExportSize::LinkedIn] {ui.selectable_value(&mut draft.export_size,size,size.label());}
                        });hit(ui,"preferences-export-size",size.response.rect);
                        ui.horizontal(|ui| {ui.label("JPEG quality");let r=ui.add(egui::DragValue::new(&mut draft.jpeg_quality).range(50..=100).suffix("%"));hit(ui,"preferences-quality",r.rect);});
                        note(ui,"Exports keep the original file intact and choose a new versioned filename.");
                    }
                }
                ui.add_space(6.);
            });
            if let Some(error)=&self.preferences_error {
                ui.label(RichText::new(error).size(12.).color(YELLOW));
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                let defaults=ui.button("Reset defaults");hit(ui,"preferences-reset",defaults.rect);reset=defaults.clicked();
                let close=ui.button("Cancel");hit(ui,"preferences-cancel",close.rect);cancel=close.clicked();
                let save=ui.add(primary("Apply").min_size(vec2(78.,34.)));hit(ui,"preferences-apply",save.rect);apply=save.clicked();
            });
        });
        if reset {
            self.preferences_draft = Preferences::default();
            self.preferences_error = None;
        }
        if apply {
            self.commit_preferences(ctx);
        } else if cancel || response.should_close() {
            self.preferences_open = false;
        }
    }

    pub(super) fn command_dialog(&mut self, ctx: &egui::Context) {
        if !self.command_open {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.command_open = false;
            return;
        }
        let down = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let up = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let all = commands();
        let mut run = None;
        let response = egui::Modal::new(egui::Id::new("command-menu"))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .fill(PANEL)
                    .inner_margin(16)
                    .corner_radius(14),
            )
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 70.).clamp(250., 500.));
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22., 24.), Sense::hover());
                    paint_icon(ui.painter(), r, Icon::Command, YELLOW);
                    ui.label(RichText::new("Find a command").size(18.).strong());
                });
                let search = ui.add_sized(
                    vec2(ui.available_width(), 34.),
                    egui::TextEdit::singleline(&mut self.command_query)
                        .hint_text("Search tools, panels and actions…"),
                );
                hit(ui, "command-search", search.rect);
                let mut scroll_selection = self.command_focus || down || up;
                if self.command_focus {
                    search.request_focus();
                    self.command_focus = false;
                }
                if search.changed() {
                    self.command_index = 0;
                    scroll_selection = true;
                }
                let query = self.command_query.to_lowercase();
                let terms: Vec<_> = query.split_whitespace().collect();
                let filtered: Vec<_> = all
                    .iter()
                    .filter(|c| {
                        terms
                            .iter()
                            .all(|term| c.name.to_lowercase().contains(term))
                    })
                    .collect();
                self.command_index = self.command_index.min(filtered.len().saturating_sub(1));
                if down {
                    self.command_index =
                        (self.command_index + 1).min(filtered.len().saturating_sub(1));
                }
                if up {
                    self.command_index = self.command_index.saturating_sub(1);
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("command-results")
                    .max_height((ctx.content_rect().height() - 200.).clamp(90., 320.))
                    .show(ui, |ui| {
                        for (index, command) in filtered.iter().enumerate() {
                            let enabled = self.command_available(command.action);
                            let response = ui.add_enabled(
                                enabled,
                                egui::Button::new(format!("       {}", command.name))
                                    .selected(index == self.command_index)
                                    .min_size(vec2(ui.available_width(), 36.)),
                            );
                            paint_icon(
                                ui.painter(),
                                Rect::from_center_size(
                                    pos2(response.rect.left() + 15., response.rect.center().y),
                                    vec2(18., 18.),
                                ),
                                command.icon,
                                if enabled {
                                    MUTED
                                } else {
                                    MUTED.gamma_multiply(0.35)
                                },
                            );
                            hit(ui, &format!("command-{}", command.name), response.rect);
                            if scroll_selection && index == self.command_index {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            if response.clicked() {
                                run = Some(command.action);
                            }
                            response.on_hover_text(if enabled {
                                binding(command, &self.preferences)
                                    .map_or(String::new(), |b| b.label())
                            } else {
                                "Unavailable in the current workspace".into()
                            });
                        }
                        if filtered.is_empty() {
                            note(ui, "No matching commands. Try a tool or panel name.");
                        }
                    });
                if enter
                    && let Some(command) = filtered.get(self.command_index)
                    && self.command_available(command.action)
                {
                    run = Some(command.action);
                }
                ui.separator();
                note(ui, "↑ / ↓ to choose  ·  Enter to run  ·  Escape to close");
            });
        if let Some(action) = run {
            if let Some(id) = ctx.memory(|m| m.focused()) {
                ctx.memory_mut(|m| m.surrender_focus(id));
            }
            self.execute_command(ctx, action);
        } else if response.should_close() {
            self.command_open = false;
        }
    }

    pub(super) fn command_shortcuts(&mut self, ctx: &egui::Context) {
        if egui::Popup::is_any_open(ctx) || self.export_open || self.help {
            return;
        }
        for command in commands() {
            if let Some(binding) = binding(&command, &self.preferences)
                && binding.consume(ctx)
            {
                self.execute_command(ctx, command.action);
                break;
            }
        }
    }

    pub(super) fn micro_shortcuts(&mut self, ctx: &egui::Context) {
        if egui::Popup::is_any_open(ctx) || self.export_open || self.help {
            return;
        }
        if self.brush.is_some() {
            for (key, factor, delta) in [
                (egui::Key::OpenBracket, 0.8, -5.),
                (egui::Key::CloseBracket, 1.25, 5.),
            ] {
                if pressed(ctx, egui::Modifiers::NONE, key) {
                    self.brush_radius = (self.brush_radius * factor).clamp(0.002, 0.09);
                }
                if pressed(ctx, egui::Modifiers::SHIFT, key) {
                    self.brush_strength = (self.brush_strength + delta).clamp(1., 100.);
                }
            }
            if pressed(ctx, egui::Modifiers::NONE, egui::Key::M) {
                self.show_mask = !self.show_mask;
            }
            if !matches!(
                self.brush,
                Some(Target::Heal | Target::Liquify | Target::Clone | Target::Patch)
            ) && pressed(ctx, egui::Modifiers::NONE, egui::Key::X)
            {
                self.erase = !self.erase;
            }
        }
        if !self.photos.is_empty() {
            for (key, index) in [
                (egui::Key::ArrowLeft, self.current.saturating_sub(1)),
                (
                    egui::Key::ArrowRight,
                    (self.current + 1).min(self.photos.len() - 1),
                ),
            ] {
                if pressed(ctx, egui::Modifiers::SHIFT, key) {
                    self.select_photo(index, true, false);
                }
            }
            for (key, rating) in [
                (egui::Key::Num1, 1),
                (egui::Key::Num2, 2),
                (egui::Key::Num3, 3),
                (egui::Key::Num4, 4),
                (egui::Key::Num5, 5),
            ] {
                if pressed(ctx, egui::Modifiers::NONE, key) {
                    self.photos[self.current].rating = rating;
                }
            }
            if pressed(ctx, egui::Modifiers::SHIFT, egui::Key::Num0) {
                self.photos[self.current].rating = 0;
            }
        }
    }
}
