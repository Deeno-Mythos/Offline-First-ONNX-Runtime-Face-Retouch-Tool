//! Choosing a facial skin reference and matching selected portraits without copying grading.
use super::*;
impl HasturApp {
    pub(super) fn skin_reference_panel(&mut self, ui: &mut egui::Ui) {
        section(ui, "UNIFY SKIN TONE", |ui| {
            let faces = self.photos[self.current]
                .seg
                .as_ref()
                .map_or(0, |s| s.faces.len());
            if faces == 0 {
                let r = ui.add_enabled(
                    !self.photos[self.current].ai_processing,
                    secondary("Detect face skin"),
                );
                hit(ui, "skin-reference-detect", r.rect);
                if r.clicked() {
                    self.request_portrait(self.current, PortraitDemand::GEOMETRY, true);
                }
            } else {
                self.skin_reference_face = self.skin_reference_face.min(faces - 1);
                if faces > 1 {
                    egui::ComboBox::from_id_salt("skin-reference-face")
                        .selected_text(format!("Face {}", self.skin_reference_face + 1))
                        .show_ui(ui, |ui| {
                            for index in 0..faces {
                                ui.selectable_value(
                                    &mut self.skin_reference_face,
                                    index,
                                    format!("Face {}", index + 1),
                                );
                            }
                        });
                }
                let p = &self.photos[self.current];
                let r = ui.add_enabled(p.photo.resident, secondary("Use source face's skin tone"));
                hit(ui, "skin-reference-capture", r.rect);
                if r.clicked() {
                    let p = &self.photos[self.current];
                    let target = crate::skin_tone::SkinTarget::from_face(
                        format!("{} · face {}", p.photo.name, self.skin_reference_face + 1),
                        &p.photo.preview,
                        p.seg.as_deref().unwrap(),
                        self.skin_reference_face,
                    );
                    match target {
                        Ok(target) => {
                            let p = &mut self.photos[self.current];
                            p.edit.settings.skin_tone.target = Some(target);
                            p.edit.settings.skin_tone.strength = 65.;
                            p.edit
                                .settings
                                .disabled
                                .retain(|a| *a != Adjustment::SkinTone);
                        }
                        Err(e) => self.notify(e.to_string()),
                    }
                }
            }
            if let Some(target) = &self.photos[self.current].edit.settings.skin_tone.target {
                ui.label(RichText::new(&target.name).size(11.).color(MUTED));
                adjustment(
                    ui,
                    "Skin tone match",
                    &mut self.photos[self.current].edit.settings,
                    Adjustment::SkinTone,
                    0.0..=100.0,
                    "Unify skin color while preserving brightness and fine detail.",
                );
                let others = self
                    .photos
                    .iter()
                    .enumerate()
                    .any(|(i, p)| i != self.current && p.selected);
                let r = ui.add_enabled(others, secondary("Unify selected skin tones"));
                hit(ui, "skin-reference-sync", r.rect);
                if r.clicked() {
                    self.sync_skin_tone();
                }
                let r = ui.button("Clear skin target");
                hit(ui, "skin-reference-clear", r.rect);
                if r.clicked() {
                    self.photos[self.current].edit.settings.skin_tone = Default::default();
                }
            }
            ui.label(RichText::new("Keeps each person's skin brightness and texture. Refine the skin mask to include the neck or body.").size(11.).color(MUTED));
        });
    }
    fn sync_skin_tone(&mut self) {
        let source = self.photos[self.current].edit.settings.skin_tone.clone();
        if !source.active() {
            return;
        }
        let disabled = self.photos[self.current]
            .edit
            .settings
            .disabled
            .contains(&Adjustment::SkinTone);
        let mut count = 0;
        for (_, p) in self
            .photos
            .iter_mut()
            .enumerate()
            .filter(|(i, p)| *i != self.current && p.selected)
        {
            let before = p.edit.clone();
            p.edit.settings.skin_tone = source.clone();
            p.edit
                .settings
                .disabled
                .retain(|a| *a != Adjustment::SkinTone);
            if disabled {
                p.edit.settings.disabled.push(Adjustment::SkinTone);
            }
            if p.edit != before {
                p.history.record(before);
                p.revision += 1;
                count += 1;
            }
        }
        self.queue_selected_portrait_ai();
        self.last_edit = Instant::now();
        self.notify(format!("Skin target matched to {count} selected portraits"));
    }
}
