//! Selection-scoped synchronization and cancellable face-focus review.
use super::*;

pub(super) struct FocusRun {
    request: u64,
    cancel: Arc<AtomicBool>,
    pending: HashSet<u64>,
    total: usize,
}

#[derive(Clone, Copy)]
pub(super) struct SyncOptions {
    pub portrait: bool,
    pub color: bool,
    pub background: bool,
    pub match_exposure: bool,
}
impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            portrait: true,
            color: true,
            background: true,
            match_exposure: false,
        }
    }
}
impl SyncOptions {
    pub(super) fn active(self) -> bool {
        self.portrait || self.color || self.background
    }
    pub(super) fn apply(
        self,
        source: &Settings,
        target: &mut Settings,
        source_mean: f32,
        target_mean: f32,
    ) {
        let portrait = [
            Adjustment::Smoothing,
            Adjustment::Blemishes,
            Adjustment::ToneEvenness,
            Adjustment::SkinTone,
            Adjustment::Redness,
            Adjustment::Teeth,
            Adjustment::Eyes,
            Adjustment::UnderEyes,
            Adjustment::Forehead,
            Adjustment::LaughLines,
            Adjustment::Contour,
            Adjustment::FaceHighlight,
            Adjustment::EyeSize,
            Adjustment::NoseWidth,
            Adjustment::LipPlumpness,
            Adjustment::Jawline,
            Adjustment::Healing,
            Adjustment::FlyawayHairs,
            Adjustment::ClothingWrinkles,
        ];
        let color = [
            Adjustment::Exposure,
            Adjustment::Contrast,
            Adjustment::Shadows,
            Adjustment::Highlights,
            Adjustment::Whites,
            Adjustment::Blacks,
            Adjustment::DodgeBurn,
            Adjustment::Warmth,
            Adjustment::Tint,
            Adjustment::Saturation,
            Adjustment::Sharpening,
            Adjustment::Vignette,
            Adjustment::Hsl,
            Adjustment::Curves,
            Adjustment::ReferenceMatch,
        ];
        let mut source_values = source.clone();
        for (enabled, adjustments) in [(self.portrait, &portrait[..]), (self.color, &color[..])] {
            if !enabled {
                continue;
            }
            for &a in adjustments {
                *target.value_mut(a) = *source_values.value_mut(a);
                target.disabled.retain(|old| *old != a);
                if source.disabled.contains(&a) {
                    target.disabled.push(a);
                }
            }
        }
        if self.portrait {
            target.skin_tone.target = source.skin_tone.target.clone();
            target.skin_detail_retention = source.skin_detail_retention;
        }
        if self.color {
            target.color = source.color.clone();
            if self.match_exposure && !source.disabled.contains(&Adjustment::Exposure) {
                let linear = |v: f32| engine::srgb_to_linear(v.clamp(0.02, 0.98));
                let offset = (linear(source_mean) / linear(target_mean))
                    .log2()
                    .clamp(-1.5, 1.5);
                target.exposure = (source.exposure + offset).clamp(-2., 2.);
            }
        }
        if self.background {
            target.background = source.background;
            target.background_blur = source.background_blur;
            target.background_color = source.background_color;
            target.background_opacity = source.background_opacity;
            target.disabled.retain(|a| *a != Adjustment::BackgroundBlur);
            if source.disabled.contains(&Adjustment::BackgroundBlur) {
                target.disabled.push(Adjustment::BackgroundBlur);
            }
        }
    }
}

impl HasturApp {
    pub(super) fn queue_selected_portrait_ai(&mut self) {
        let selected: Vec<_> = self
            .photos
            .iter()
            .enumerate()
            .filter(|(_, p)| p.selected)
            .map(|(i, p)| (i, p.edit.portrait_demand()))
            .collect();
        for (i, demand) in selected {
            self.request_portrait(i, demand, false);
        }
    }
    pub(super) fn start_focus_review(&mut self) {
        if let Some(run) = self.focus_run.take() {
            run.cancel.store(true, Ordering::Relaxed);
        }
        let selected: Vec<_> = self
            .photos
            .iter()
            .filter(|p| p.selected)
            .map(|p| p.id)
            .collect();
        if selected.is_empty() {
            return;
        }
        self.focus_serial += 1;
        let request = self.focus_serial;
        let cancel = Arc::new(AtomicBool::new(false));
        let pending = selected.iter().copied().collect();
        self.focus_run = Some(FocusRun {
            request,
            cancel: cancel.clone(),
            pending,
            total: selected.len(),
        });
        for photo in self.photos.iter().filter(|p| selected.contains(&p.id)) {
            self.face_focus_errors.remove(&photo.id);
            if self
                .tx
                .send(Job::ScoreFace {
                    id: photo.id,
                    request,
                    source: photo.canonical_photo().clone(),
                    base: photo.canonical_seg().cloned(),
                    provider: self.provider,
                    cancel: cancel.clone(),
                })
                .is_err()
            {
                self.face_focus_errors
                    .insert(photo.id, "Face analysis worker is unavailable".into());
                self.focus_run.as_mut().unwrap().pending.remove(&photo.id);
            }
        }
        if self
            .focus_run
            .as_ref()
            .is_some_and(|r| r.pending.is_empty())
        {
            self.focus_run = None;
        }
    }
    pub(super) fn accept_face_focus(
        &mut self,
        id: u64,
        request: u64,
        result: anyhow::Result<crate::focus::FocusReport>,
    ) {
        let Some(run) = self.focus_run.as_mut().filter(|r| r.request == request) else {
            return;
        };
        if !run.pending.remove(&id) {
            return;
        }
        if self.photos.iter().any(|p| p.id == id) {
            match result {
                Ok(report) => {
                    self.face_focus.insert(id, report);
                    self.face_focus_errors.remove(&id);
                }
                Err(e) => {
                    self.face_focus.remove(&id);
                    self.face_focus_errors.insert(id, e.to_string());
                }
            }
        }
        if run.pending.is_empty() {
            self.focus_run = None;
            self.notify("Face focus review complete · compare scores within the same shoot");
        }
    }
    fn rate_focused_selection(&mut self) {
        let mut scores: Vec<_> = self
            .photos
            .iter()
            .filter(|p| p.selected)
            .filter_map(|p| {
                self.face_focus
                    .get(&p.id)
                    .and_then(|r| r.score)
                    .map(|s| (p.id, s))
            })
            .collect();
        scores.sort_by(|a, b| b.1.total_cmp(&a.1));
        if scores.is_empty() {
            return;
        }
        for photo in self.photos.iter_mut().filter(|p| p.selected) {
            photo.rating = 0;
        }
        for (id, _) in scores.into_iter().take(5) {
            if let Some(photo) = self.photos.iter_mut().find(|p| p.id == id) {
                photo.rating = 5;
            }
        }
        self.notify(
            "Rated up to five selected face-focus picks · inspect expressions before export",
        );
    }
    pub(super) fn batch_sync_options(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let r = ui.checkbox(&mut self.sync_options.portrait, "Portrait");
            hit(ui, "sync-portrait", r.rect);
            let r = ui.checkbox(&mut self.sync_options.color, "Light + color");
            hit(ui, "sync-color", r.rect);
            let r = ui.checkbox(&mut self.sync_options.background, "Background");
            hit(ui, "sync-background", r.rect);
        });
        let r = ui.add_enabled(
            self.sync_options.color,
            egui::Checkbox::new(
                &mut self.sync_options.match_exposure,
                "Adapt exposure per photo",
            ),
        );
        hit(ui, "sync-match-exposure", r.rect);
        r.on_hover_text("Compensate source brightness differences, up to 1.5 EV. Review photos with different lighting.");
        let r = ui.add_enabled(
            self.photos.iter().any(|p| p.selected),
            secondary("Auto Retouch selected"),
        );
        hit(ui, "batch-auto-retouch", r.rect);
        if r.clicked() {
            self.end_canvas_gesture();
            let mut count = 0;
            for p in self.photos.iter_mut().filter(|p| p.selected) {
                let before = p.edit.clone();
                p.edit.settings.apply_auto_retouch();
                if p.edit != before {
                    p.history.record(before);
                    p.revision += 1;
                    count += 1;
                }
            }
            self.last_edit = Instant::now();
            self.queue_selected_portrait_ai();
            self.notify(format!("Facial cleanup applied to {count} selected photos"));
        }
        let wanted: Vec<_> = self
            .photos
            .iter()
            .filter(|p| p.selected)
            .filter(|p| !p.edit.portrait_demand().is_empty())
            .collect();
        if !wanted.is_empty() {
            let ready = wanted
                .iter()
                .filter(|p| {
                    p.ai_seg_provider == Some(self.provider)
                        && p.seg
                            .as_ref()
                            .is_some_and(|s| s.prepared.contains(p.edit.portrait_demand()))
                })
                .count();
            let processing = wanted.iter().filter(|p| p.ai_processing).count();
            ui.add(
                egui::ProgressBar::new(ready as f32 / wanted.len() as f32).text(format!(
                    "Portrait AI: {ready} / {} ready · {processing} processing",
                    wanted.len()
                )),
            );
            if processing > 0 {
                let r = ui.button("Pause selected portrait preparation");
                hit(ui, "batch-ai-pause", r.rect);
                if r.clicked() {
                    for p in self
                        .photos
                        .iter_mut()
                        .filter(|p| p.selected && p.ai_processing)
                    {
                        if let Some(cancel) = p.ai_cancel.take() {
                            cancel.store(true, Ordering::Relaxed);
                        }
                        p.ai_request += 1;
                        p.ai_processing = false;
                        p.ai_failed = p.ai_failed.union(p.ai_requested);
                        p.note = Some("Portrait preparation paused · resume in Batch".into());
                    }
                }
            } else if ready < wanted.len() {
                let r = ui.button("Prepare selected portraits");
                hit(ui, "batch-ai-resume", r.rect);
                if r.clicked() {
                    for p in self.photos.iter_mut().filter(|p| p.selected) {
                        p.ai_failed = PortraitDemand::NONE;
                    }
                    self.queue_selected_portrait_ai();
                }
            }
        }
    }
    pub(super) fn focus_review_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("FACE FOCUS").strong());
        if let Some(run) = &self.focus_run {
            let done = run.total - run.pending.len();
            ui.add(
                egui::ProgressBar::new(done as f32 / run.total as f32)
                    .text(format!("{done} / {} portraits", run.total)),
            );
            let r = ui.button("Cancel face review");
            hit(ui, "focus-cancel", r.rect);
            if r.clicked()
                && let Some(run) = self.focus_run.take()
            {
                run.cancel.store(true, Ordering::Relaxed);
            }
        } else {
            let r = ui.add_enabled(
                self.photos.iter().any(|p| p.selected),
                secondary("Analyze selected faces"),
            );
            hit(ui, "focus-analyze", r.rect);
            if r.clicked() {
                self.start_focus_review();
            }
        }
        let available = self.photos.iter().filter(|p| p.selected).any(|p| {
            self.face_focus
                .get(&p.id)
                .is_some_and(|r| r.score.is_some())
        });
        let r = ui.add_enabled(
            available && self.focus_run.is_none(),
            secondary("Rate sharpest faces"),
        );
        hit(ui, "focus-rate", r.rect);
        if r.clicked() {
            self.rate_focused_selection();
        }
        ui.label(RichText::new("Scores use detected faces and eyes. Compare within a shoot; review expressions and very small faces manually.")
            .size(11.).color(MUTED));
        for p in self.photos.iter().filter(|p| p.selected) {
            if let Some(report) = self.face_focus.get(&p.id) {
                ui.label(format!("{} · {}", p.photo.name, report.summary()));
                for (i, face) in report.faces.iter().enumerate() {
                    if let Some(score) = face.score {
                        ui.label(
                            RichText::new(format!(
                                "Face {}: {score:.0} / 100 · eyes {} / {}",
                                i + 1,
                                face.eyes[0].map_or("—".into(), |s| format!("{s:.0}")),
                                face.eyes[1].map_or("—".into(), |s| format!("{s:.0}"))
                            ))
                            .size(11.)
                            .color(MUTED),
                        );
                    }
                }
            }
            if let Some(error) = self.face_focus_errors.get(&p.id) {
                ui.label(RichText::new(format!("{} · {error}", p.photo.name)).color(YELLOW));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selective_sync_preserves_omitted_groups_and_disabled_adjustments() {
        let source = Settings {
            exposure: 0.5,
            smoothing: 60.,
            background: Background::Solid,
            disabled: vec![Adjustment::Smoothing],
            ..Default::default()
        };
        let mut target = Settings {
            exposure: -0.7,
            warmth: 14.,
            smoothing: 20.,
            background: Background::Transparent,
            disabled: vec![Adjustment::Exposure],
            ..Default::default()
        };
        SyncOptions {
            portrait: true,
            color: false,
            background: false,
            match_exposure: false,
        }
        .apply(&source, &mut target, 0.5, 0.2);
        assert_eq!(target.exposure, -0.7);
        assert_eq!(target.warmth, 14.);
        assert_eq!(target.background, Background::Transparent);
        assert_eq!(target.smoothing, 60.);
        assert!(target.disabled.contains(&Adjustment::Exposure));
        assert!(target.disabled.contains(&Adjustment::Smoothing));
    }
}
