//! Presets, tool rail and library share the same editing state as the workspace.
use super::*;

impl HasturApp {
    pub(super) fn canvas_zoom_bar(&mut self, ui: &mut egui::Ui, dimensions: Vec2) {
        if self.filmstrip_visible {
            return;
        }
        ui.add_space(6.);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} × {}  ·  sRGB",
                    dimensions.x as u32, dimensions.y as u32
                ))
                .size(10.)
                .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.zoom_controls(ui);
            });
        });
    }
    pub(super) fn zoom_controls(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.photos.get_mut(self.current) else {
            return;
        };
        let viewport = self.canvas_viewport.unwrap_or(ui.ctx().available_rect());
        let dpi = ui.ctx().pixels_per_point();
        let scale = self
            .transform_draft
            .as_ref()
            .map_or(p.view.scale, |d| d.scale * dpi);
        let mut next = None;
        let fit = small_chip(ui, "Fit", self.transform_draft.is_none() && p.view.fit);
        hit(ui, "fit", fit.rect);
        if fit.clicked() {
            if let Some(d) = &mut self.transform_draft {
                d.request_fit();
            } else {
                p.view = View::default();
            }
        }
        let native = small_chip(ui, "1:1", !p.view.fit && (scale - 1.).abs() < 0.001);
        hit(ui, "native", native.rect);
        if native
            .on_hover_text("One image pixel per screen pixel")
            .clicked()
        {
            next = Some(1.);
        }
        let plus = small_chip(ui, "+", false);
        hit(ui, "zoom-plus", plus.rect);
        if plus.clicked() {
            next = Some((scale * 1.2).min(View::MAX));
        }
        ui.label(
            RichText::new(format!("{:.1}%", scale * 100.))
                .size(11.)
                .color(TEXT),
        );
        let minus = small_chip(ui, "−", false);
        hit(ui, "zoom-minus", minus.rect);
        if minus.clicked() {
            next = Some((scale / 1.2).max(View::MIN));
        }
        if let Some(next) = next {
            if let Some(d) = &mut self.transform_draft {
                d.scale = next / dpi;
                d.center = Some(viewport.center());
            } else {
                p.view.zoom_at(next, viewport.center(), viewport);
                if (next - 1.).abs() < 0.001 {
                    p.view.pan = [0.; 2];
                }
            }
        }
    }
    pub(super) fn workspace_toolbar(&mut self, ui: &mut egui::Ui) {
        let width = ui.ctx().content_rect().width();
        if width >= 1280. {
            ui.separator();
            if layer_panel::icon_button(
                ui,
                layer_panel::Icon::Hand,
                self.brush.is_none() && self.transform_draft.is_none(),
                true,
                "toolbar-hand",
                "Hand · Space + drag to pan",
            )
            .clicked()
            {
                self.end_canvas_gesture();
                self.brush = None;
                self.transform_draft = None;
                self.library_open = false;
            }
            for (target, icon, label) in [
                (Target::Liquify, layer_panel::Icon::Liquify, "Liquify"),
                (Target::Heal, layer_panel::Icon::Heal, "Spot heal"),
                (Target::Clone, layer_panel::Icon::Clone, "Clone stamp"),
                (Target::Patch, layer_panel::Icon::Patch, "Patch"),
            ] {
                if layer_panel::icon_button(
                    ui,
                    icon,
                    self.brush == Some(target),
                    !self.photos.is_empty(),
                    &format!("toolbar-{label}"),
                    label,
                )
                .clicked()
                {
                    self.toggle_brush(target);
                    self.library_open = false;
                }
            }
        }
        if width >= 850. {
            let ready = self
                .photos
                .get(self.current)
                .is_some_and(|p| p.photo.resident && p.document_ready());
            if layer_panel::icon_button(
                ui,
                layer_panel::Icon::Crop,
                self.transform_draft.is_some(),
                ready,
                "toolbar-crop",
                "Crop · trim or rotate the frame while the photo stays upright · C / Ctrl+T",
            )
            .clicked()
            {
                if self.transform_draft.is_some() {
                    self.transform_draft = None;
                } else {
                    self.open_crop();
                }
            }
        }
        if width >= 1050. {
            ui.add_space(12.);
            let r = ui.add(
                egui::Button::new(RichText::new("Library").strong())
                    .frame(false)
                    .selected(self.library_open)
                    .min_size(vec2(65., 32.)),
            );
            hit(ui, "workspace-library", r.rect);
            if r.clicked() {
                self.end_canvas_gesture();
                self.transform_draft = None;
                self.library_open = true;
            }
            let r = ui.add(
                egui::Button::new(RichText::new("Edit").strong())
                    .frame(false)
                    .selected(!self.library_open)
                    .min_size(vec2(55., 32.)),
            );
            hit(ui, "workspace-edit", r.rect);
            if r.clicked() {
                self.library_open = false;
            }
        }
    }
    pub(super) fn presets_sidebar(&mut self, ctx: &egui::Context) {
        if ctx.content_rect().width() < 1280.
            || self.inspector.is_none()
            || self.library_open
            || !self.presets_visible
            || self.photos.is_empty()
            || self.transform_draft.is_some()
        {
            return;
        }
        egui::SidePanel::left("presets-sidebar")
            .exact_width(224.)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(10, 10)),
            )
            .show(ctx, |ui| {
                hit(ui, "presets-sidebar", ui.max_rect());
                ui.horizontal(|ui| {
                    layer_panel::paint_icon(
                        ui.painter(),
                        Rect::from_center_size(ui.cursor().min + vec2(12., 15.), vec2(20., 20.)),
                        layer_panel::Icon::Image,
                        TEXT,
                    );
                    ui.add_space(30.);
                    ui.label(RichText::new("Presets").size(14.).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new("×").frame(false).min_size(vec2(28., 30.)))
                            .on_hover_text("Hide presets · reopen from View")
                            .clicked()
                        {
                            self.presets_visible = false;
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("presets-gallery")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let before = self.photos[self.current].edit.clone();
                        let locked = before.layer_locked();
                        ui.add_enabled_ui(!locked && self.transform_draft.is_none(), |ui| {
                            self.preset_panel(ui)
                        });
                        if before != self.photos[self.current].edit {
                            self.photos[self.current].history.record(before);
                            self.edited();
                        }
                    });
            });
    }
    pub(super) fn library_canvas(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(16))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Photo library");
                    ui.label(RichText::new(format!("{} photos", self.photos.len())).color(MUTED));
                    if ui.add(secondary("Import photos…")).clicked() {
                        self.import();
                    }
                });
                ui.label(
                    RichText::new("Double-click to edit · Ctrl / Shift + click to select")
                        .color(MUTED),
                );
                let columns = (ui.available_width() / 170.).floor().max(1.) as usize;
                let mut select = None;
                let mut edit = false;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("library-grid")
                            .num_columns(columns)
                            .spacing(vec2(12., 12.))
                            .show(ui, |ui| {
                                for (index, p) in self.photos.iter().enumerate() {
                                    let (r, response) =
                                        ui.allocate_exact_size(vec2(158., 188.), Sense::click());
                                    ui.painter().rect_filled(r, 5., PANEL);
                                    let target = r.shrink2(vec2(8., 18.)).translate(vec2(0., -8.));
                                    let size = p.photo.dimensions();
                                    let scale = (target.width() / size.0 as f32)
                                        .min(target.height() / size.1 as f32);
                                    let photo_rect = Rect::from_center_size(
                                        target.center(),
                                        vec2(size.0 as f32, size.1 as f32) * scale,
                                    );
                                    ui.painter().image(
                                        p.edited_texture.id(),
                                        photo_rect,
                                        Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                                        Color32::WHITE,
                                    );
                                    if p.selected {
                                        ui.painter().rect_stroke(
                                            r,
                                            5.,
                                            Line::new(1.5_f32, YELLOW),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    ui.painter().with_clip_rect(r.shrink(6.)).text(
                                        r.center_bottom() - vec2(0., 10.),
                                        Align2::CENTER_BOTTOM,
                                        &p.photo.name,
                                        FontId::proportional(11.),
                                        TEXT,
                                    );
                                    hit(ui, &format!("library-photo-{index}"), r);
                                    if response.clicked() || response.double_clicked() {
                                        select = Some(index);
                                        edit = response.double_clicked();
                                    }
                                    if (index + 1) % columns == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                    });
                if let Some(index) = select {
                    let m = ctx.input(|i| i.modifiers);
                    self.select_photo(index, m.shift, m.ctrl || m.command);
                    if edit {
                        self.library_open = false;
                    }
                }
            });
    }
}
