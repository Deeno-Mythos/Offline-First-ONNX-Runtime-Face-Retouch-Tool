//! Active-photo residency and project removal. Source files are never deleted.
use super::*;

pub(super) struct ExportQueueItem {
    pub id: u64,
    pub directory: PathBuf,
    pub finished: usize,
    pub total: usize,
    pub errors: usize,
    pub state: String,
    pub cancel: Arc<AtomicBool>,
}
impl ExportQueueItem {
    pub fn done(&self) -> bool {
        matches!(self.state.as_str(), "Done" | "Cancelled")
    }
}

impl HasturApp {
    pub(super) fn export_queue_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.export_queue_open;
        if !open {
            return;
        }
        egui::Window::new("Export queue")
            .open(&mut open)
            .default_width(440.)
            .show(ctx, |ui| {
                ui.label(
                    "Jobs keep their own photo edits and export settings. Editing can continue.",
                );
                egui::ScrollArea::vertical()
                    .max_height(340.)
                    .show(ui, |ui| {
                        for job in &self.export_jobs {
                            ui.separator();
                            ui.label(job.directory.display().to_string());
                            ui.horizontal(|ui| {
                                ui.label(format!(
                                    "{} · {} / {} · {} failures",
                                    job.state, job.finished, job.total, job.errors
                                ));
                                if !job.done()
                                    && ui
                                        .add_enabled(
                                            !job.cancel.load(Ordering::Relaxed),
                                            egui::Button::new("Cancel"),
                                        )
                                        .clicked()
                                {
                                    job.cancel.store(true, Ordering::Relaxed);
                                }
                            });
                            ui.add(egui::ProgressBar::new(
                                job.finished as f32 / job.total.max(1) as f32,
                            ));
                        }
                    });
                if ui.button("Clear completed jobs").clicked() {
                    self.export_jobs.retain(|j| !j.done());
                }
            });
        self.export_queue_open = open;
    }
    pub(super) fn release_canvas_memory(&mut self) {
        if let Some((_, _, cancel)) = self.preview_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        for cancel in [&mut self.detail_cancel, &mut self.hover_cancel] {
            if let Some(cancel) = cancel.take() {
                cancel.store(true, Ordering::Relaxed);
            }
        }
        self.render_busy = false;
        self.detail_region = None;
        self.detail_region_pending = None;
        self.detail_tile = None;
        self.hover_image = None;
        self.hover_pending = None;
        self.hover_region = None;
        self.hover_region_pending = None;
        self.mask_texture = None;
        self.patch_feedback.clear();
        self.preset_key = None;
        self.preset_thumbs.clear();
        self.layer_thumbs.clear();
        self.layer_thumbs_pending = None;
        self.preview_history.clear();
        self.preview_history_bytes = 0;
        self.gpu_release = true;
        let _ = self.tx.send(Job::ReleaseMemory);
    }

    pub(super) fn ensure_active_asset(&mut self, ctx: &egui::Context) {
        let current = self.photos.get(self.current).map(|p| p.id);
        if current != self.active_asset {
            self.end_canvas_gesture();
            self.release_canvas_memory();
            if let Some((_, _, cancel)) = self.asset_loading.take() {
                cancel.store(true, Ordering::Relaxed);
            }
            self.asset_error = None;
            self.active_asset = current;
            for photo in &mut self.photos {
                if Some(photo.id) != current && photo.photo.path.is_some() && photo.photo.resident {
                    if let Some(cancel) = photo.ai_cancel.take() {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    photo
                        .asset_cache_paths
                        .extend(crate::ai_cache::release_source(&photo.photo.preview));
                    // Preserve an edited filmstrip thumbnail before dropping native
                    // and working-size buffers. Reopening regenerates exact edits.
                    let (w, h) = crate::assets::thumbnail_dimensions(
                        photo.edited.width(),
                        photo.edited.height(),
                    );
                    photo.edited = Arc::new(image::imageops::resize(
                        photo.edited.as_ref(),
                        w,
                        h,
                        image::imageops::FilterType::Lanczos3,
                    ));
                    photo.original_texture = photo_texture(
                        ctx,
                        &format!("staged-{}", photo.id),
                        &photo.photo.thumbnail,
                        self.gpu,
                    );
                    photo.edited_texture = photo_texture(
                        ctx,
                        &format!("staged-edited-{}", photo.id),
                        &photo.edited,
                        self.gpu,
                    );
                    photo.photo.release_pixels();
                    photo.seg = photo.seg.as_ref().map(|s| Arc::new(s.session_copy()));
                    photo.ai_request += 1;
                    photo.ai_processing = false;
                    photo.ai_provider = None;
                    photo.ai_seg_provider = None;
                    photo.ai_requested = PortraitDemand::NONE;
                    photo.ai_failed = PortraitDemand::NONE;
                    photo.processing = None;
                }
            }
        }
        let Some(photo) = self.photos.get(self.current) else {
            return;
        };
        if !photo.photo.resident
            && self.asset_loading.is_none()
            && self.asset_error != Some(photo.id)
            && let Some(path) = &photo.photo.path
        {
            self.asset_serial += 1;
            let cancel = Arc::new(AtomicBool::new(false));
            self.asset_loading = Some((photo.id, self.asset_serial, cancel.clone()));
            let _ = self.tx.send(Job::LoadAsset {
                id: photo.id,
                request: self.asset_serial,
                path: path.clone(),
                cancel,
            });
        }
        if self.asset_loading.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    pub(super) fn accept_asset(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        request: u64,
        result: anyhow::Result<Photo>,
    ) {
        if !self
            .asset_loading
            .as_ref()
            .is_some_and(|(old, serial, cancel)| {
                *old == id && *serial == request && !cancel.load(Ordering::Relaxed)
            })
            || !self.photos.get(self.current).is_some_and(|p| p.id == id)
        {
            return;
        }
        self.asset_loading = None;
        match result {
            Ok(photo) => {
                let p = &mut self.photos[self.current];
                p.photo = photo;
                p.original_texture =
                    photo_texture(ctx, &format!("original-{id}"), &p.photo.preview, self.gpu);
                p.edited = p.photo.preview.clone();
                p.edited_texture = p.original_texture.clone();
                p.revision += 1;
                p.rendered_revision = 0;
                self.last_edit = Instant::now() - Duration::from_millis(70);
            }
            Err(error) => {
                self.asset_error = Some(id);
                self.notify(format!("Could not open original: {error:#}"));
            }
        }
    }

    pub(super) fn delete_from_project(&mut self, id: u64) {
        let Some(index) = self.photos.iter().position(|p| p.id == id) else {
            return;
        };
        self.end_canvas_gesture();
        if let Some((loading, _, cancel)) = &self.asset_loading
            && *loading == id
        {
            cancel.store(true, Ordering::Relaxed);
            self.asset_loading = None;
        }
        let removed = self.photos.remove(index);
        if let Some(cancel) = removed.ai_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        for path in removed
            .asset_cache_paths
            .into_iter()
            .chain(crate::ai_cache::release_source(&removed.photo.preview))
        {
            let _ = std::fs::remove_file(path);
        }
        if let Some(path) = &removed.photo.path {
            crate::assets::purge_thumbnail(path);
        }
        self.current = if index < self.current {
            self.current - 1
        } else {
            self.current.min(self.photos.len().saturating_sub(1))
        };
        self.selection_anchor = self.current;
        self.release_canvas_memory();
        self.patch_draft = PatchDraft::default();
        self.clone_anchor = None;
        self.clone_offset = None;
        self.pending_reference = None;
        self.notify(format!(
            "Removed {} from project · source file preserved",
            removed.photo.name
        ));
        if self.preferences.autosave
            && let Some(store) = &self.recovery_store
        {
            self.autosave_generation += 1;
            let generation = self.autosave_generation;
            let session = self.session_snapshot(false);
            if self
                .tx
                .send(Job::SaveRecovery {
                    path: store.path.clone(),
                    session,
                    generation,
                })
                .is_ok()
            {
                self.autosave_pending = Some(generation);
            }
            self.autosave_observed = Some(self.workspace_fingerprint());
        }
        // Autosave fingerprint now excludes this asset, its strokes and history.
    }

    pub(super) fn import_folder(&mut self) {
        if self.pending_session.is_some() || self.automatic_recovery {
            self.notify("Workspace recovery is still loading metadata.");
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Stage a photo folder")
            .pick_folder()
        {
            self.importing = true;
            self.import_batches += 1;
            let _ = self.tx.send(Job::ImportFolder(path));
        }
    }
}
