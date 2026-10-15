//! Non-destructive document size and orientation controls.
use super::*;
use crate::image_transform::{self, ImageTransform};

pub(super) struct PendingTransform {
    id: u64,
    request: u64,
    transform: ImageTransform,
    cancel: Arc<AtomicBool>,
}
impl PhotoState {
    pub(super) fn document_ready(&self) -> bool {
        self.photo.applied_transform == self.edit.transform
    }
    pub(super) fn canonical_photo(&self) -> &Photo {
        if self.photo.applied_transform == ImageTransform::default() {
            &self.photo
        } else {
            &self.source_photo
        }
    }
    pub(super) fn canonical_seg(&self) -> Option<&Arc<Segmentation>> {
        if self.photo.applied_transform == ImageTransform::default() {
            self.seg.as_ref()
        } else {
            self.source_seg.as_ref()
        }
    }
}

impl HasturApp {
    fn cancel_document(&mut self) {
        if let Some(pending) = self.document_loading.take() {
            pending.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(super) fn ensure_document(&mut self, ctx: &egui::Context) {
        let target = self
            .photos
            .get(self.current)
            .map(|p| (p.id, p.edit.transform));
        if self
            .document_loading
            .as_ref()
            .is_some_and(|pending| target != Some((pending.id, pending.transform)))
        {
            self.cancel_document();
        }
        let Some(p) = self.photos.get_mut(self.current) else {
            return;
        };
        if p.document_ready() || !p.photo.resident {
            return;
        }
        if p.photo.applied_transform == ImageTransform::default() {
            p.source_photo = p.photo.clone();
            p.source_seg = p.seg.clone();
        }
        if self.document_error == target {
            return;
        }
        if self.document_loading.is_none() {
            let (id, transform, source) = (p.id, p.edit.transform, p.source_photo.clone());
            self.document_serial += 1;
            let request = self.document_serial;
            let cancel = Arc::new(AtomicBool::new(false));
            self.release_canvas_memory();
            self.document_loading = Some(PendingTransform {
                id,
                request,
                transform,
                cancel: cancel.clone(),
            });
            if self
                .tx
                .send(Job::TransformPhoto {
                    id,
                    request,
                    source,
                    transform,
                    cancel,
                })
                .is_err()
            {
                self.document_loading = None;
                self.document_error = target;
                self.notify("Image preparation worker is unavailable. Undo to return to the previous image.");
            }
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }
    pub(super) fn accept_transform(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        request: u64,
        transform: ImageTransform,
        result: anyhow::Result<Photo>,
    ) {
        if !self.document_loading.as_ref().is_some_and(|p| {
            p.id == id
                && p.request == request
                && p.transform == transform
                && !p.cancel.load(Ordering::Relaxed)
        }) || !self
            .photos
            .get(self.current)
            .is_some_and(|p| p.id == id && p.edit.transform == transform)
        {
            return;
        }
        self.document_loading = None;
        match result {
            Ok(photo) => {
                self.document_error = None;
                let p = &mut self.photos[self.current];
                p.photo = photo;
                p.seg = p
                    .source_seg
                    .as_ref()
                    .map(|s| Arc::new(s.transformed(transform, p.source_photo.dimensions())));
                p.original_texture =
                    photo_texture(ctx, &format!("original-{id}"), &p.photo.preview, self.gpu);
                p.edited = p.photo.preview.clone();
                p.edited_texture = p.original_texture.clone();
                p.revision += 1;
                p.rendered_revision = 0;
                p.processing = None;
                self.last_edit = Instant::now() - Duration::from_millis(100);
            }
            Err(error) => {
                self.document_error = Some((id, transform));
                self.notify(format!(
                    "Could not prepare image: {error:#}. Undo or retry from the workspace."
                ));
            }
        }
    }
    pub(super) fn set_document_transform(&mut self, transform: ImageTransform) {
        let transform = transform.normalized();
        if transform.validate().is_err() {
            return;
        }
        self.transform_draft = None;
        self.end_canvas_gesture();
        let Some(p) = self.photos.get_mut(self.current) else {
            return;
        };
        if p.edit.transform == transform {
            return;
        }
        p.edit.sync_active_layer();
        p.history.record(p.edit.clone());
        let old = p.edit.transform;
        let source = p.canonical_photo().dimensions();
        p.edit.transform_actions(old, transform, source);
        p.edit.transform = transform;
        p.edit.sync_active_layer();
        p.crop_center = image_transform::remap_point(p.crop_center, old, transform, source);
        p.view = View::default();
        self.clone_anchor = None;
        self.clone_offset = None;
        self.patch_draft = PatchDraft::default();
        self.document_error = None;
        self.edited();
    }
    pub(super) fn image_menu(&mut self, ui: &mut egui::Ui) {
        let menu = ui.menu_button(RichText::new("Image").color(MUTED), |ui| {
            ui.add_enabled_ui(!self.photos.is_empty(), |ui| {
                let r = ui
                    .add(egui::Button::new("Crop frame  ·  C / Ctrl+T").min_size(vec2(220., 34.)));
                hit(ui, "image-crop", r.rect);
                if r.clicked() {
                    self.open_crop();
                    ui.close();
                }
                ui.separator();
                for (key, label, turns) in [
                    ("image-rotate-left", "Rotate frame 90° left", 3),
                    ("image-rotate-right", "Rotate frame 90° right", 1),
                    ("image-rotate-180", "Rotate frame 180°", 2),
                ] {
                    let r = ui.add(egui::Button::new(label).min_size(vec2(200., 34.)));
                    hit(ui, key, r.rect);
                    if r.clicked() {
                        self.rotate_frame(turns);
                        ui.close();
                    }
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("Reset crop frame").min_size(vec2(200., 34.)))
                    .clicked()
                {
                    if let Some(p) = self.photos.get(self.current) {
                        let mut target = p.edit.transform;
                        target.crop = None;
                        self.set_document_transform(target);
                    }
                    ui.close();
                }
            });
        });
        hit(ui, "image-menu", menu.response.rect);
    }
}
