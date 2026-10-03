//! Compact, accessible layer controls. Vector icons stay crisp at every display scale.
use super::*;
use crate::layer_stack::{Layer, LayerColor};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum LayerFilter {
    #[default]
    All,
    Retouch,
    Adjustment,
    Image,
}
impl LayerFilter {
    const ALL: [Self; 4] = [Self::All, Self::Retouch, Self::Adjustment, Self::Image];
    fn label(self) -> &'static str {
        match self {
            Self::All => "All kinds",
            Self::Retouch => "Retouch",
            Self::Adjustment => "Adjustments",
            Self::Image => "Images",
        }
    }
    fn matches(self, kind: LayerType) -> bool {
        match self {
            Self::All => true,
            Self::Retouch => kind == LayerType::Retouch,
            Self::Adjustment => kind == LayerType::Adjustment,
            Self::Image => kind == LayerType::OriginalCopy,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LayerAction {
    New(LayerType),
    Duplicate,
    Delete,
    Move(bool),
    Rename,
    Visibility,
    Solo,
    RestoreSolo,
    Lock,
    Color(LayerColor),
    CopySettings,
    PasteSettings,
    ResetSettings,
    AddMask,
    PaintMask,
    InvertMask,
    RemoveMask,
}

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Liquify,
    Heal,
    Clone,
    Patch,
    Undo,
    Redo,
    Export,
    Stack,
    Eye,
    EyeOff,
    Lock,
    Unlock,
    Retouch,
    Adjust,
    Image,
    Duplicate,
    Trash,
    Up,
    Down,
    Rename,
    Search,
    Close,
    Copy,
    Paste,
    Reset,
    Solo,
    Tag,
    Gear,
    Command,
    Focus,
    Filmstrip,
    Keyboard,
}
fn kind_icon(kind: LayerType) -> Icon {
    match kind {
        LayerType::Retouch => Icon::Retouch,
        LayerType::Adjustment => Icon::Adjust,
        LayerType::OriginalCopy => Icon::Image,
    }
}
fn label_color(color: LayerColor) -> Color32 {
    match color {
        LayerColor::None => BORDER,
        LayerColor::Red => Color32::from_rgb(205, 108, 109),
        LayerColor::Orange => Color32::from_rgb(205, 150, 93),
        LayerColor::Yellow => YELLOW,
        LayerColor::Green => Color32::from_rgb(110, 172, 143),
        LayerColor::Blue => Color32::from_rgb(115, 151, 205),
        LayerColor::Violet => Color32::from_rgb(165, 132, 199),
    }
}

pub(super) fn paint_icon(p: &egui::Painter, r: Rect, icon: Icon, color: Color32) {
    let c = r.center();
    let point = |x: f32, y: f32| c + vec2(x, y) * (r.width().min(r.height()) / 24.0);
    let line = Line::new(1.45_f32, color);
    let segment = |a, b| {
        p.line_segment([a, b], line);
    };
    let path = |points: &[(f32, f32)]| {
        p.add(egui::Shape::line(
            points.iter().map(|&(x, y)| point(x, y)).collect(),
            line,
        ));
    };
    let outline = |x: f32, y: f32, w: f32, h: f32, rounding| {
        p.rect_stroke(
            Rect::from_min_max(point(x, y), point(x + w, y + h)),
            rounding,
            line,
            egui::StrokeKind::Inside,
        );
    };
    match icon {
        Icon::Liquify => {
            path(&[(-9., -7.), (-4., -7.), (0., -3.), (4., -7.), (9., -7.)]);
            path(&[(-9., 0.), (-4., 0.), (0., 4.), (4., 0.), (9., 0.)]);
            path(&[(-9., 7.), (-4., 7.), (0., 11.), (4., 7.), (9., 7.)]);
        }
        Icon::Heal => {
            path(&[(-9., 3.), (3., -9.), (9., -3.), (-3., 9.), (-9., 3.)]);
            path(&[(-2., -4.), (4., 2.)]);
            path(&[(-5., -1.), (1., 5.)]);
        }
        Icon::Clone => {
            path(&[(-8., 4.), (8., 4.), (9., 9.), (-9., 9.), (-8., 4.)]);
            path(&[
                (-4., 4.),
                (-4., 0.),
                (-2., -3.),
                (-3., -7.),
                (0., -9.),
                (3., -7.),
                (2., -3.),
                (4., 0.),
                (4., 4.),
            ]);
        }
        Icon::Patch => {
            path(&[
                (-8., -8.),
                (5., -9.),
                (9., -3.),
                (7., 7.),
                (-5., 9.),
                (-9., 3.),
                (-8., -8.),
            ]);
            path(&[(-4., -4.), (4., 4.)]);
            path(&[(-4., 4.), (4., -4.)]);
        }
        Icon::Undo => {
            path(&[(-8., -5.), (2., -5.), (8., 0.), (6., 7.), (-3., 8.)]);
            path(&[(-3., -10.), (-8., -5.), (-3., 0.)]);
        }
        Icon::Redo => {
            path(&[(8., -5.), (-2., -5.), (-8., 0.), (-6., 7.), (3., 8.)]);
            path(&[(3., -10.), (8., -5.), (3., 0.)]);
        }
        Icon::Export => {
            path(&[(-8., -1.), (-8., 9.), (8., 9.), (8., -1.)]);
            path(&[(0., 4.), (0., -10.)]);
            path(&[(-5., -5.), (0., -10.), (5., -5.)]);
        }
        Icon::Gear => {
            p.circle_stroke(c, r.width() * 0.25, line);
            p.circle_stroke(c, r.width() * 0.08, line);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.;
                let d = vec2(a.cos(), a.sin());
                segment(c + d * r.width() * 0.25, c + d * r.width() * 0.4);
            }
        }
        Icon::Command => {
            outline(-9., -8., 18., 16., 3.);
            path(&[(-5., -3.), (-1., 0.), (-5., 3.)]);
            segment(point(2., 4.), point(6., 4.));
        }
        Icon::Focus => {
            for (x, y) in [(-1., -1.), (1., -1.), (-1., 1.), (1., 1.)] {
                path(&[(x * 9., y * 3.), (x * 9., y * 9.), (x * 3., y * 9.)]);
            }
        }
        Icon::Filmstrip => {
            outline(-9., -7., 18., 14., 2.);
            segment(point(-3., -7.), point(-3., 7.));
            segment(point(3., -7.), point(3., 7.));
        }
        Icon::Keyboard => {
            outline(-10., -7., 20., 14., 2.);
            for y in [-3., 0.] {
                for x in [-6., -2., 2., 6.] {
                    p.circle_filled(point(x, y), r.width() / 32., color);
                }
            }
            segment(point(-5., 4.), point(5., 4.));
        }
        Icon::Eye | Icon::EyeOff => {
            path(&[
                (-9., 0.),
                (-6., -4.),
                (0., -6.),
                (6., -4.),
                (9., 0.),
                (6., 4.),
                (0., 6.),
                (-6., 4.),
                (-9., 0.),
            ]);
            p.circle_stroke(c, r.width() / 8., line);
            if matches!(icon, Icon::EyeOff) {
                segment(point(-8., -8.), point(8., 8.));
            }
        }
        Icon::Lock | Icon::Unlock => {
            outline(-6., -1., 12., 10., 2.);
            if matches!(icon, Icon::Lock) {
                path(&[
                    (-4., -1.),
                    (-4., -6.),
                    (-2., -8.),
                    (2., -8.),
                    (4., -6.),
                    (4., -1.),
                ]);
            } else {
                path(&[(-4., -1.), (-4., -6.), (-2., -8.), (2., -8.), (4., -6.)]);
            }
            segment(point(0., 3.), point(0., 6.));
        }
        Icon::Stack => {
            path(&[(-9., -3.), (0., -8.), (9., -3.), (0., 2.), (-9., -3.)]);
            path(&[(-9., 2.), (0., 7.), (9., 2.)]);
            path(&[(-9., 6.), (0., 11.), (9., 6.)]);
        }
        Icon::Retouch => {
            path(&[
                (-8., 6.),
                (4., -6.),
                (8., -2.),
                (-4., 10.),
                (-8., 10.),
                (-8., 6.),
            ]);
            segment(point(0., -2.), point(4., 2.));
            segment(point(-6., -8.), point(-6., -2.));
            segment(point(-9., -5.), point(-3., -5.));
        }
        Icon::Adjust => {
            p.circle_stroke(c, r.width() * 0.34, line);
            p.add(egui::Shape::convex_polygon(
                (0..=12)
                    .map(|i| {
                        let a = i as f32 * std::f32::consts::PI / 12. - std::f32::consts::FRAC_PI_2;
                        c + vec2(a.cos(), a.sin()) * r.width() * 0.28
                    })
                    .collect(),
                color,
                Line::NONE,
            ));
        }
        Icon::Image => {
            outline(-9., -8., 18., 16., 2.);
            path(&[(-7., 5.), (-2., -1.), (2., 3.), (5., 0.), (7., 5.)]);
            p.circle_filled(point(4., -4.), r.width() / 12., color);
        }
        Icon::Duplicate | Icon::Copy => {
            outline(-7., -4., 12., 13., 2.);
            path(&[(-3., -8.), (9., -8.), (9., 5.)]);
        }
        Icon::Paste => {
            outline(-7., -6., 14., 16., 2.);
            outline(-3., -9., 6., 5., 1.);
            segment(point(-3., 1.), point(3., 1.));
            segment(point(-3., 5.), point(3., 5.));
        }
        Icon::Trash => {
            path(&[(-6., -4.), (-5., 9.), (5., 9.), (6., -4.)]);
            segment(point(-8., -5.), point(8., -5.));
            path(&[(-3., -5.), (-3., -9.), (3., -9.), (3., -5.)]);
            segment(point(-2., 0.), point(-2., 5.));
            segment(point(2., 0.), point(2., 5.));
        }
        Icon::Up | Icon::Down => {
            let d = if matches!(icon, Icon::Up) { -1. } else { 1. };
            segment(point(0., -8. * d), point(0., 8. * d));
            path(&[(-5., 3. * d), (0., 8. * d), (5., 3. * d)]);
        }
        Icon::Rename => {
            path(&[
                (-8., 5.),
                (5., -8.),
                (9., -4.),
                (-4., 9.),
                (-9., 10.),
                (-8., 5.),
            ]);
            segment(point(2., -5.), point(6., -1.));
        }
        Icon::Search => {
            p.circle_stroke(point(-2., -2.), r.width() / 4., line);
            segment(point(3., 3.), point(9., 9.));
        }
        Icon::Close => {
            segment(point(-5., -5.), point(5., 5.));
            segment(point(-5., 5.), point(5., -5.));
        }
        Icon::Reset => {
            path(&[
                (-7., -3.),
                (-4., -7.),
                (2., -8.),
                (7., -4.),
                (8., 2.),
                (4., 7.),
                (-2., 8.),
                (-7., 4.),
            ]);
            path(&[(-8., -9.), (-8., -2.), (-1., -2.)]);
        }
        Icon::Solo => {
            outline(-9., -9., 18., 18., 3.);
            p.circle_stroke(c, r.width() / 6., line);
        }
        Icon::Tag => {
            path(&[
                (-8., -8.),
                (0., -8.),
                (9., 1.),
                (1., 9.),
                (-8., 0.),
                (-8., -8.),
            ]);
            p.circle_filled(point(-4., -4.), r.width() / 16., color);
        }
    }
}

fn icon_at(
    ui: &egui::Ui,
    rect: Rect,
    icon: Icon,
    selected: bool,
    enabled: bool,
    key: &str,
    tooltip: &str,
) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.id().with(key),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let enabled = enabled && ui.is_enabled();
    if selected || (enabled && response.hovered()) {
        ui.painter().rect_filled(
            rect.shrink(2.),
            6.,
            if selected {
                Color32::from_rgb(48, 42, 18)
            } else {
                RAISED
            },
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(2.),
            6.,
            Line::new(1_f32, YELLOW),
            egui::StrokeKind::Inside,
        );
    }
    paint_icon(
        ui.painter(),
        Rect::from_center_size(rect.center(), vec2(20., 20.)),
        icon,
        if !enabled {
            MUTED.gamma_multiply(0.35)
        } else if selected {
            YELLOW
        } else {
            MUTED
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tooltip));
    hit(ui, key, rect);
    response.on_hover_text(tooltip)
}
pub(super) fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    selected: bool,
    enabled: bool,
    key: &str,
    tooltip: &str,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(vec2(32., 34.), Sense::hover());
    icon_at(ui, rect, icon, selected, enabled, key, tooltip)
}
fn menu_action(ui: &mut egui::Ui, icon: Icon, text: &str, enabled: bool, key: &str) -> bool {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(format!("       {text}")).min_size(vec2(190., 30.)),
    );
    paint_icon(
        ui.painter(),
        Rect::from_center_size(
            pos2(response.rect.left() + 14., response.rect.center().y),
            vec2(18., 18.),
        ),
        icon,
        if enabled {
            MUTED
        } else {
            MUTED.gamma_multiply(0.35)
        },
    );
    hit(ui, key, response.rect);
    if response.clicked() {
        ui.close();
        true
    } else {
        false
    }
}

impl HasturApp {
    pub(super) fn layer_action(&mut self, id: u64, action: LayerAction) {
        let Some(photo) = self.photos.get_mut(self.current) else {
            return;
        };
        photo.edit.ensure_stack();
        // Eye and label actions do not change the editing target.
        if !matches!(
            action,
            LayerAction::Visibility
                | LayerAction::Solo
                | LayerAction::RestoreSolo
                | LayerAction::Color(_)
                | LayerAction::Lock
        ) {
            photo.edit.select_layer(id);
        }
        if matches!(action, LayerAction::Rename) {
            if let Some(layer) = photo.edit.active_layer() {
                self.layer_rename = Some((photo.id, layer.id, layer.name.clone()));
            }
            return;
        }
        if matches!(action, LayerAction::CopySettings) {
            if photo.edit.active_layer().is_some() {
                self.layer_clipboard = Some(photo.edit.settings.clone());
                self.toast = Some(("Layer adjustments copied".into(), Instant::now()));
            }
            return;
        }
        if matches!(action, LayerAction::PaintMask) {
            if photo
                .edit
                .active_layer()
                .is_some_and(|layer| layer.mask.is_some() && !layer.locked)
            {
                self.brush = Some(Target::LayerMask);
                self.show_mask = true;
                self.erase = false;
                self.end_canvas_gesture();
            }
            return;
        }
        let before = photo.edit.clone();
        let room = photo.edit.stack.layers.len() < 32;
        match action {
            LayerAction::New(kind) if room => {
                photo.edit.add_layer(kind);
                self.layer_query.clear();
                self.layer_filter = LayerFilter::All;
            }
            LayerAction::Duplicate if room => {
                photo.edit.duplicate_layer();
                self.layer_query.clear();
                self.layer_filter = LayerFilter::All;
            }
            LayerAction::Delete => {
                photo.edit.delete_layer();
            }
            LayerAction::Move(up) => {
                photo.edit.move_layer(up);
            }
            LayerAction::Visibility => {
                photo.edit.toggle_layer_visibility(id);
            }
            LayerAction::Solo => {
                photo.edit.solo_layer(id);
            }
            LayerAction::RestoreSolo => {
                photo.edit.restore_solo();
            }
            LayerAction::Lock => {
                if let Some(layer) = photo.edit.stack.layers.iter_mut().find(|l| l.id == id) {
                    layer.locked = !layer.locked;
                }
            }
            LayerAction::Color(color) => {
                if let Some(layer) = photo.edit.stack.layers.iter_mut().find(|l| l.id == id) {
                    layer.color = color;
                }
            }
            LayerAction::PasteSettings if !photo.edit.layer_locked() => {
                if let Some(settings) = &self.layer_clipboard {
                    photo.edit.settings = settings.clone();
                }
            }
            LayerAction::ResetSettings if !photo.edit.layer_locked() => {
                photo.edit.settings = Settings::default();
            }
            LayerAction::AddMask if !photo.edit.layer_locked() => {
                if let Some(layer) = photo.edit.active_layer_mut() {
                    layer
                        .mask
                        .get_or_insert_with(crate::layer_mask::LayerMask::default);
                }
            }
            LayerAction::InvertMask if !photo.edit.layer_locked() => {
                if let Some(mask) = photo
                    .edit
                    .active_layer_mut()
                    .and_then(|layer| layer.mask.as_mut())
                {
                    mask.invert();
                }
            }
            LayerAction::RemoveMask if !photo.edit.layer_locked() => {
                if let Some(layer) = photo.edit.active_layer_mut() {
                    layer.mask = None;
                }
                if self.brush == Some(Target::LayerMask) {
                    self.brush = None;
                }
            }
            _ => {}
        }
        if photo.edit != before {
            photo.history.record(before);
            self.end_canvas_gesture();
            self.end_layer_interaction();
            self.edited();
        }
    }

    pub(super) fn layers_panel(&mut self, ui: &mut egui::Ui) {
        let Some(photo) = self.photos.get_mut(self.current) else {
            ui.label("Import a photo to begin.");
            return;
        };
        photo.edit.ensure_stack();
        let photo_id = photo.id;
        let revision = photo.revision;
        let active = photo.edit.stack.active;
        let count = photo.edit.stack.layers.len();
        let solo = photo.edit.stack.solo.is_some();
        let mut action = None;
        let mut selected = None;
        let mut drag_to = None;
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(22., 24.), Sense::hover());
            paint_icon(ui.painter(), r, Icon::Stack, TEXT);
            ui.label(RichText::new("Layers").size(16.).strong());
            ui.label(RichText::new(format!("{count}")).size(11.).color(MUTED));
            if solo
                && icon_button(
                    ui,
                    Icon::Solo,
                    true,
                    true,
                    "stack-solo-exit",
                    "Exit solo · restore previous visibility",
                )
                .clicked()
            {
                action = Some((active, LayerAction::RestoreSolo));
            }
        });
        ui.add_space(3.);
        let wide = ui.available_width() >= 420.;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.;
            let (r, _) = ui.allocate_exact_size(vec2(20., 30.), Sense::hover());
            paint_icon(ui.painter(), r, Icon::Search, MUTED);
            let width = (ui.available_width() - 38. - if wide { 180. } else { 0. }).max(60.);
            let search = ui.add_sized(
                vec2(width, 30.),
                egui::TextEdit::singleline(&mut self.layer_query)
                    .hint_text("Find layer…")
                    .desired_width(width),
            );
            hit(ui, "stack-search", search.rect);
            if icon_button(
                ui,
                Icon::Close,
                false,
                !self.layer_query.is_empty(),
                "stack-search-clear",
                "Clear layer search",
            )
            .clicked()
            {
                self.layer_query.clear();
            }
            if wide {
                self.layer_filter_controls(ui);
            }
        });
        if !wide {
            ui.horizontal(|ui| self.layer_filter_controls(ui));
        }
        ui.add_space(3.);
        let before = self.photos[self.current].edit.clone();
        let locked = before.layer_locked();
        let has_active = before.active_layer().is_some();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.;
            ui.add_enabled_ui(!locked, |ui| {
                if let Some(layer) = self.photos[self.current].edit.active_layer_mut() {
                    egui::ComboBox::from_id_salt("layer-blend")
                        .selected_text(layer.blend.label())
                        .width(108.)
                        .show_ui(ui, |ui| {
                            for blend in BlendMode::ALL {
                                ui.selectable_value(&mut layer.blend, blend, blend.label());
                            }
                        });
                } else {
                    ui.label("Original");
                }
            });
            for (icon, enabled, key, tip, op) in [
                (
                    if locked { Icon::Lock } else { Icon::Unlock },
                    has_active,
                    "stack-lock",
                    "Lock or unlock layer",
                    LayerAction::Lock,
                ),
                (
                    Icon::Copy,
                    has_active,
                    "stack-copy-settings",
                    "Copy adjustments · excludes manual strokes",
                    LayerAction::CopySettings,
                ),
                (
                    Icon::Paste,
                    has_active && !locked && self.layer_clipboard.is_some(),
                    "stack-paste-settings",
                    "Paste adjustments · keeps manual strokes",
                    LayerAction::PasteSettings,
                ),
                (
                    Icon::Reset,
                    has_active && !locked,
                    "stack-reset-settings",
                    "Reset adjustments · keeps manual strokes",
                    LayerAction::ResetSettings,
                ),
            ] {
                if icon_button(
                    ui,
                    icon,
                    matches!(op, LayerAction::Lock) && locked,
                    enabled,
                    key,
                    tip,
                )
                .clicked()
                {
                    action = Some((active, op));
                }
            }
        });
        ui.add_enabled_ui(!locked, |ui| {
            if let Some(layer) = self.photos[self.current].edit.active_layer_mut() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.;
                    ui.label(RichText::new("Opacity").size(11.).color(MUTED));
                    let width = (ui.available_width() - 76.).max(40.);
                    ui.spacing_mut().slider_width = width;
                    let opacity = ui.add_sized(
                        vec2(width, 32.),
                        egui::Slider::new(&mut layer.opacity, 0.0..=100.0).show_value(false),
                    );
                    hit(ui, "layer-opacity", opacity.rect);
                    if opacity.double_clicked() {
                        layer.opacity = 100.;
                    }
                    let number = ui.add(
                        egui::DragValue::new(&mut layer.opacity)
                            .range(0.0..=100.0)
                            .speed(0.5)
                            .suffix("%"),
                    );
                    hit(ui, "layer-opacity-number", number.rect);
                });
            } else {
                ui.label(
                    RichText::new("Original background · read only")
                        .size(11.)
                        .color(MUTED),
                );
            }
        });
        if has_active {
            let has_mask = self.photos[self.current]
                .edit
                .active_layer()
                .is_some_and(|layer| layer.mask.is_some());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.;
                if !has_mask {
                    let button = ui.add_enabled(!locked, egui::Button::new("Add layer mask"));
                    hit(ui, "layer-add-mask", button.rect);
                    if button.clicked() {
                        action = Some((active, LayerAction::AddMask));
                    }
                } else {
                    for (text, key, op) in [
                        ("Paint mask", "layer-paint-mask", LayerAction::PaintMask),
                        ("Invert", "layer-invert-mask", LayerAction::InvertMask),
                        ("Remove", "layer-remove-mask", LayerAction::RemoveMask),
                    ] {
                        let button = ui.add_enabled(!locked, egui::Button::new(text));
                        hit(ui, key, button.rect);
                        if button.clicked() {
                            action = Some((active, op));
                        }
                    }
                }
            });
        }
        if self.photos[self.current].edit != before {
            if !self.gesture {
                self.photos[self.current].history.record(before);
            }
            self.gesture = ui.ctx().input(|i| i.pointer.any_down());
            self.end_layer_interaction();
            self.edited();
        }
        ui.separator();
        let query = self.layer_query.trim().to_lowercase();
        let layers = self.photos[self.current].edit.stack.layers.clone();
        let mut shown = 0;
        egui::ScrollArea::vertical()
            .id_salt(("stack-scroll", photo_id))
            .max_height((ui.available_height() - 66.).max(72.))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 3.;
                for layer in layers.iter().rev().filter(|l| {
                    self.layer_filter.matches(l.kind) && l.name.to_lowercase().contains(&query)
                }) {
                    shown += 1;
                    let (r, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 58.), Sense::hover());
                    let body = Rect::from_min_max(r.min + vec2(34., 0.), r.max - vec2(29., 0.));
                    let response = ui.interact(
                        body,
                        ui.id().with(("layer-row", layer.id)),
                        Sense::click_and_drag(),
                    );
                    let row_active = layer.id == active;
                    ui.painter().rect_filled(
                        r,
                        6.,
                        if row_active {
                            Color32::from_rgb(42, 39, 24)
                        } else if response.hovered() {
                            RAISED
                        } else {
                            PANEL
                        },
                    );
                    if row_active {
                        ui.painter().rect_stroke(
                            r,
                            6.,
                            Line::new(1_f32, Color32::from_rgb(89, 76, 27)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    ui.painter().rect_filled(
                        Rect::from_min_size(r.min + vec2(1., 5.), vec2(3., 48.)),
                        2.,
                        if layer.color == LayerColor::None && row_active {
                            YELLOW
                        } else {
                            label_color(layer.color)
                        },
                    );
                    let eye = icon_at(
                        ui,
                        Rect::from_min_size(r.min + vec2(4., 9.), vec2(30., 40.)),
                        if layer.visible {
                            Icon::Eye
                        } else {
                            Icon::EyeOff
                        },
                        self.photos[self.current]
                            .edit
                            .stack
                            .solo
                            .as_ref()
                            .is_some_and(|s| s.layer == layer.id),
                        true,
                        &format!("stack-visible-{}", layer.id),
                        "Show/hide layer · Alt-click to solo, again to restore",
                    );
                    if eye.clicked() {
                        action = Some((
                            layer.id,
                            if ui.input(|i| i.modifiers.alt) {
                                LayerAction::Solo
                            } else {
                                LayerAction::Visibility
                            },
                        ));
                    }
                    let thumb = Rect::from_min_size(r.min + vec2(38., 6.), vec2(36., 46.));
                    for y in 0..6 {
                        for x in 0..5 {
                            let cell = Rect::from_min_size(
                                thumb.min + vec2(x as f32 * 8., y as f32 * 8.),
                                vec2(8., 8.),
                            )
                            .intersect(thumb);
                            ui.painter().rect_filled(
                                cell,
                                0.,
                                Color32::from_gray(if (x + y) % 2 == 0 { 47 } else { 34 }),
                            );
                        }
                    }
                    if let Some((_, _, _, tex)) = self
                        .layer_thumbs
                        .iter()
                        .find(|(id, key, _, _)| *id == photo_id && *key == layer.id)
                    {
                        let size = tex.size_vec2();
                        let fit = size * (thumb.width() / size.x).min(thumb.height() / size.y);
                        ui.painter().image(
                            tex.id(),
                            Rect::from_center_size(thumb.center(), fit),
                            Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                            if layer.visible {
                                Color32::WHITE
                            } else {
                                Color32::from_white_alpha(100)
                            },
                        );
                    }
                    ui.painter().rect_stroke(
                        thumb,
                        2.,
                        Line::new(1_f32, BORDER),
                        egui::StrokeKind::Inside,
                    );
                    if let Some(mask) = &layer.mask {
                        let mask_thumb = Rect::from_min_size(r.min + vec2(77., 9.), vec2(23., 40.));
                        let dims = self.photos[self.current].photo.dimensions();
                        for cy in 0..10 {
                            for cx in 0..6 {
                                let uv = [(cx as f32 + 0.5) / 6., (cy as f32 + 0.5) / 10.];
                                let value = mask.sample_uv(uv, dims);
                                ui.painter().rect_filled(
                                    Rect::from_min_size(
                                        mask_thumb.min + vec2(cx as f32 * 4., cy as f32 * 4.),
                                        vec2(4., 4.),
                                    ),
                                    0.,
                                    Color32::from_gray(value),
                                );
                            }
                        }
                        ui.painter().rect_stroke(
                            mask_thumb,
                            1.0_f32,
                            Line::new(
                                1.0_f32,
                                if row_active && self.brush == Some(Target::LayerMask) {
                                    YELLOW
                                } else {
                                    BORDER
                                },
                            ),
                            egui::StrokeKind::Inside,
                        );
                        let mask_hit = ui.interact(
                            mask_thumb,
                            ui.id().with(("layer-mask-thumb", layer.id)),
                            Sense::click(),
                        );
                        hit(ui, &format!("stack-mask-{}", layer.id), mask_thumb);
                        if mask_hit.clicked() {
                            action = Some((layer.id, LayerAction::PaintMask));
                        }
                        mask_hit.on_hover_text(
                            "Click to paint this layer mask · black hides, white reveals",
                        );
                    }
                    let text_rect = Rect::from_min_max(
                        r.min + vec2(if layer.mask.is_some() { 107. } else { 82. }, 0.),
                        r.max - vec2(30., 0.),
                    );
                    let painter = ui.painter().with_clip_rect(text_rect);
                    let name = egui::WidgetText::from(&layer.name).into_galley(
                        ui,
                        Some(egui::TextWrapMode::Truncate),
                        text_rect.width(),
                        FontId::proportional(12.),
                    );
                    painter.galley(
                        text_rect.min + vec2(0., 10.),
                        name,
                        if layer.visible { TEXT } else { MUTED },
                    );
                    paint_icon(
                        &painter,
                        Rect::from_center_size(text_rect.min + vec2(7., 40.), vec2(14., 14.)),
                        kind_icon(layer.kind),
                        MUTED,
                    );
                    painter.text(
                        text_rect.min + vec2(19., 34.),
                        Align2::LEFT_TOP,
                        format!("{} · {:.0}%", layer.kind.label(), layer.opacity),
                        FontId::proportional(9.),
                        MUTED,
                    );
                    if icon_at(
                        ui,
                        Rect::from_min_size(pos2(r.right() - 29., r.top() + 9.), vec2(28., 40.)),
                        if layer.locked {
                            Icon::Lock
                        } else {
                            Icon::Unlock
                        },
                        layer.locked,
                        true,
                        &format!("stack-row-lock-{}", layer.id),
                        "Lock or unlock this layer",
                    )
                    .clicked()
                    {
                        action = Some((layer.id, LayerAction::Lock));
                    }
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::SelectableLabel,
                            true,
                            row_active,
                            &layer.name,
                        )
                    });
                    hit(ui, &format!("stack-select-{}", layer.id), body);
                    if response.clicked() || response.secondary_clicked() {
                        selected = Some(layer.id);
                    }
                    if response.double_clicked() {
                        action = Some((layer.id, LayerAction::Rename));
                    }
                    if response.drag_started() && !layer.locked {
                        self.layer_drag = Some(layer.id);
                    }
                    if ui.input(|i| i.pointer.primary_released())
                        && r.contains(ui.input(|i| i.pointer.latest_pos()).unwrap_or(Pos2::ZERO))
                    {
                        drag_to = self.layer_drag.map(|from| (from, layer.id));
                    }
                    if self.layer_drag.is_some() && response.contains_pointer() {
                        ui.painter()
                            .hline(r.x_range(), r.top(), Line::new(2_f32, YELLOW));
                    }
                    response.clone().on_hover_text(format!(
                        "{}\n{} · {} · {:.0}%\nDouble-click to rename · drag to reorder",
                        layer.name,
                        layer.kind.label(),
                        layer.blend.label(),
                        layer.opacity
                    ));
                    response.context_menu(|ui| {
                        self.layer_context_menu(ui, layer, &mut action);
                    });
                }
                if shown == 0 {
                    ui.add_space(12.);
                    ui.label(RichText::new("No matching layers").color(MUTED));
                    ui.label(
                        RichText::new("Clear the search or change the kind filter.")
                            .size(10.)
                            .color(MUTED),
                    );
                }
                if (self.layer_filter == LayerFilter::All
                    || self.layer_filter == LayerFilter::Image)
                    && "background".contains(&query)
                {
                    ui.separator();
                    let (r, response) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 54.), Sense::click());
                    ui.painter().rect_filled(
                        r,
                        6.,
                        if active == 0 {
                            Color32::from_rgb(42, 39, 24)
                        } else {
                            PANEL
                        },
                    );
                    paint_icon(
                        ui.painter(),
                        Rect::from_center_size(r.min + vec2(19., 27.), vec2(20., 20.)),
                        Icon::Eye,
                        MUTED.gamma_multiply(0.5),
                    );
                    let size = self.photos[self.current].original_texture.size_vec2();
                    let fit = size * (36. / size.x).min(44. / size.y);
                    ui.painter().image(
                        self.photos[self.current].original_texture.id(),
                        Rect::from_center_size(r.min + vec2(56., 27.), fit),
                        Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                        Color32::WHITE,
                    );
                    ui.painter().text(
                        r.min + vec2(82., 10.),
                        Align2::LEFT_TOP,
                        "Background",
                        FontId::proportional(12.),
                        MUTED,
                    );
                    ui.painter().text(
                        r.min + vec2(82., 31.),
                        Align2::LEFT_TOP,
                        "Original · read only",
                        FontId::proportional(9.),
                        MUTED,
                    );
                    paint_icon(
                        ui.painter(),
                        Rect::from_center_size(pos2(r.right() - 15., r.center().y), vec2(18., 18.)),
                        Icon::Lock,
                        MUTED,
                    );
                    hit(ui, "stack-background", r);
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::SelectableLabel,
                            true,
                            active == 0,
                            "Background · read only",
                        )
                    });
                    if response.clicked() {
                        selected = Some(0);
                    }
                    response.on_hover_text(
                        "Immutable original. Use the image-copy icon below to duplicate it.",
                    );
                }
            });
        ui.separator();
        let room = count < 32;
        let index = layers.iter().position(|l| l.id == active);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.;
            for (icon, enabled, key, tip, op) in [
                (
                    Icon::Retouch,
                    room,
                    "stack-new",
                    "New retouch layer · Ctrl+Shift+N",
                    LayerAction::New(LayerType::Retouch),
                ),
                (
                    Icon::Adjust,
                    room,
                    "stack-adjust",
                    "New adjustment layer",
                    LayerAction::New(LayerType::Adjustment),
                ),
                (
                    Icon::Image,
                    room,
                    "stack-background-copy",
                    "New image copy from original",
                    LayerAction::New(LayerType::OriginalCopy),
                ),
                (
                    Icon::Duplicate,
                    room && has_active,
                    "stack-copy",
                    "Duplicate layer · Ctrl+J",
                    LayerAction::Duplicate,
                ),
                (
                    Icon::Up,
                    !locked && index.is_some_and(|i| i + 1 < count),
                    "stack-up",
                    "Move layer up · Ctrl+]",
                    LayerAction::Move(true),
                ),
                (
                    Icon::Down,
                    !locked && index.is_some_and(|i| i > 0),
                    "stack-down",
                    "Move layer down · Ctrl+[",
                    LayerAction::Move(false),
                ),
                (
                    Icon::Trash,
                    !locked && has_active,
                    "stack-delete",
                    "Delete layer · Delete",
                    LayerAction::Delete,
                ),
            ] {
                if icon_button(ui, icon, false, enabled, key, tip).clicked() {
                    action = Some((active, op));
                }
            }
        });
        ui.label(
            RichText::new(if count == 32 {
                "32-layer limit · remove a layer to add another"
            } else {
                "Alt + eye: solo    •    Right-click: layer options"
            })
            .size(9.)
            .color(MUTED),
        );
        if let Some((from, to)) = drag_to {
            let before = self.photos[self.current].edit.clone();
            if self.photos[self.current].edit.reorder_layer(from, to) {
                self.photos[self.current].history.record(before);
                self.end_canvas_gesture();
                self.end_layer_interaction();
                self.edited();
            }
        }
        if ui.input(|i| i.pointer.primary_released()) {
            self.layer_drag = None;
        }
        if let Some(id) = selected
            && self.photos[self.current].edit.select_layer(id)
        {
            self.end_canvas_gesture();
            self.end_layer_interaction();
            self.photos[self.current].revision += 1;
            self.last_edit = Instant::now() - Duration::from_millis(66);
        }
        if let Some((id, op)) = action {
            self.layer_action(id, op);
        }
        if self.layer_thumbs_pending.is_none()
            && self.canvas_gesture != Gesture::Brush
            && self.photos[self.current].photo.resident
            && self
                .layer_thumbs
                .iter()
                .filter(|(id, _, rev, _)| *id == photo_id && *rev == revision)
                .count()
                != self.photos[self.current].edit.stack.layers.len()
        {
            let photo = &self.photos[self.current];
            let mut layers = photo.edit.stack.layers.to_vec();
            for layer in &mut layers {
                if layer.id == photo.edit.stack.active {
                    *layer.edit = photo.edit.flat_edit();
                }
            }
            if self
                .tx
                .send(Job::LayerThumbnails {
                    id: photo_id,
                    revision: photo.revision,
                    image: photo.photo.preview.clone(),
                    layers,
                    seg: photo.seg.clone(),
                })
                .is_ok()
            {
                self.layer_thumbs_pending = Some((photo_id, photo.revision));
            }
        }
    }

    fn layer_filter_controls(&mut self, ui: &mut egui::Ui) {
        let filter = egui::ComboBox::from_id_salt("layer-kind-filter")
            .selected_text(self.layer_filter.label())
            .width(112.)
            .show_ui(ui, |ui| {
                for filter in LayerFilter::ALL {
                    let response =
                        ui.selectable_value(&mut self.layer_filter, filter, filter.label());
                    hit(
                        ui,
                        &format!("stack-filter-{}", filter.label()),
                        response.rect,
                    );
                }
            });
        hit(ui, "stack-kind-filter", filter.response.rect);
        let layers = &self.photos[self.current].edit.stack.layers;
        let query = self.layer_query.trim().to_lowercase();
        let shown = layers
            .iter()
            .filter(|l| self.layer_filter.matches(l.kind) && l.name.to_lowercase().contains(&query))
            .count();
        ui.label(
            RichText::new(format!("{shown} of {}", layers.len()))
                .size(10.)
                .color(MUTED),
        );
    }

    fn layer_context_menu(
        &self,
        ui: &mut egui::Ui,
        layer: &Layer,
        action: &mut Option<(u64, LayerAction)>,
    ) {
        ui.label(RichText::new(&layer.name).strong());
        ui.separator();
        let room = self.photos[self.current].edit.stack.layers.len() < 32;
        for (icon, text, enabled, key, op) in [
            (
                Icon::Rename,
                "Rename…",
                true,
                "stack-menu-rename",
                LayerAction::Rename,
            ),
            (
                Icon::Duplicate,
                "Duplicate",
                room,
                "stack-menu-copy",
                LayerAction::Duplicate,
            ),
            (
                Icon::Solo,
                "Solo / restore",
                true,
                "stack-menu-solo",
                LayerAction::Solo,
            ),
            (
                if layer.locked {
                    Icon::Unlock
                } else {
                    Icon::Lock
                },
                if layer.locked { "Unlock" } else { "Lock" },
                true,
                "stack-menu-lock",
                LayerAction::Lock,
            ),
            (
                Icon::Copy,
                "Copy adjustments",
                true,
                "stack-menu-copy-settings",
                LayerAction::CopySettings,
            ),
            (
                Icon::Paste,
                "Paste adjustments",
                !layer.locked && self.layer_clipboard.is_some(),
                "stack-menu-paste-settings",
                LayerAction::PasteSettings,
            ),
            (
                Icon::Reset,
                "Reset adjustments",
                !layer.locked,
                "stack-menu-reset",
                LayerAction::ResetSettings,
            ),
        ] {
            let text = self.command_label(super::app_chrome::AppCommand::Layer(op), text);
            if menu_action(ui, icon, &text, enabled, key) {
                *action = Some((layer.id, op));
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(18., 24.), Sense::hover());
            paint_icon(ui.painter(), r, Icon::Tag, MUTED);
            ui.label(RichText::new("Color label").size(11.).color(MUTED));
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.;
            for color in LayerColor::ALL {
                let (r, response) = ui.allocate_exact_size(vec2(25., 28.), Sense::click());
                ui.painter()
                    .circle_filled(r.center(), 7., label_color(color));
                if color == LayerColor::None {
                    paint_icon(
                        ui.painter(),
                        Rect::from_center_size(r.center(), vec2(12., 12.)),
                        Icon::Close,
                        MUTED,
                    );
                }
                if layer.color == color {
                    ui.painter()
                        .circle_stroke(r.center(), 10., Line::new(1_f32, TEXT));
                }
                hit(ui, &format!("stack-color-{}", color.label()), r);
                if response.clicked() {
                    *action = Some((layer.id, LayerAction::Color(color)));
                    ui.close();
                }
                response.on_hover_text(color.label());
            }
        });
        ui.separator();
        if menu_action(
            ui,
            Icon::Trash,
            "Delete    Delete",
            !layer.locked,
            "stack-menu-delete",
        ) {
            *action = Some((layer.id, LayerAction::Delete));
        }
    }
}
