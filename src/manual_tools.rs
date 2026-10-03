//! Manual editing state and visible, reversible remedies for unavailable targets.
use super::*;
use crate::{layer_stack::BlendMode, layers::LayerKind};

#[derive(Default)]
pub(super) struct PatchFeedback {
    serial: u64,
    pending: Option<PatchRequest>,
    pub frame: Option<PatchFrame>,
}
struct PatchRequest {
    id: u64,
    revision: u64,
    request: u64,
    crop: Option<engine::Crop>,
    patch: crate::cleanup::PatchStroke,
    cancel: Arc<AtomicBool>,
}
pub(super) struct PatchFrame {
    pub id: u64,
    pub revision: u64,
    pub request: u64,
    pub crop: Option<engine::Crop>,
    pub image: Arc<RgbaImage>,
    pub original: Arc<RgbaImage>,
    pub texture: egui::TextureHandle,
    pub original_texture: egui::TextureHandle,
    pub committed: bool,
    patch: crate::cleanup::PatchStroke,
}
impl PatchFeedback {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn clear(&mut self) {
        if let Some(pending) = self.pending.take() {
            pending.cancel.store(true, Ordering::Relaxed);
        }
        self.frame = None;
    }
}

#[derive(Clone, Copy)]
enum Blocker {
    Locked,
    Hidden,
    Transparent,
    Covered,
    Group(LayerKind),
    HealingDisabled,
    Strength,
}
impl Blocker {
    fn text(self) -> &'static str {
        match self {
            Self::Locked => "Selected layer is locked.",
            Self::Hidden => "Selected layer is hidden. Show it to see your edits.",
            Self::Transparent => "Selected layer has 0% opacity.",
            Self::Covered => "An original-copy layer above hides this layer's edits.",
            Self::Group(_) => "This tool is disabled in the saved layer settings.",
            Self::HealingDisabled => "Healing is disabled in this layer.",
            Self::Strength => "Brush strength is 0%.",
        }
    }
    fn action(self) -> &'static str {
        match self {
            Self::Locked | Self::Covered => "New retouch layer",
            Self::Hidden => "Show layer",
            Self::Transparent => "Set opacity to 100%",
            Self::Group(_) | Self::HealingDisabled => "Enable tool",
            Self::Strength => "Set strength to 100%",
        }
    }
}
pub(super) fn is_manual(target: Target) -> bool {
    matches!(
        target,
        Target::Liquify | Target::Heal | Target::Clone | Target::Patch
    )
}
impl HasturApp {
    fn manual_blocker(&self) -> Option<Blocker> {
        let target = self.brush.filter(|t| is_manual(*t))?;
        let edit = &self.photos.get(self.current)?.edit;
        if edit.layer_locked() {
            return Some(Blocker::Locked);
        }
        if let Some(layer) = edit.active_layer() {
            if !layer.visible {
                return Some(Blocker::Hidden);
            }
            if layer.factor() == 0. {
                return Some(Blocker::Transparent);
            }
            let index = edit
                .stack
                .layers
                .iter()
                .position(|l| l.id == layer.id)
                .unwrap();
            if edit.stack.layers[index + 1..].iter().any(|l| {
                l.kind == LayerType::OriginalCopy
                    && l.factor() == 1.
                    && l.blend == BlendMode::Normal
            }) {
                return Some(Blocker::Covered);
            }
        }
        let kind = match target {
            Target::Liquify => LayerKind::Liquify,
            Target::Heal => LayerKind::SpotHeal,
            Target::Clone => LayerKind::CloneStamp,
            Target::Patch => LayerKind::Patch,
            _ => unreachable!(),
        };
        if edit.layers.get(kind).factor() == 0. {
            return Some(Blocker::Group(kind));
        }
        if target == Target::Heal && edit.settings.effective().healing <= 0. {
            return Some(Blocker::HealingDisabled);
        }
        if self.brush_strength <= 0. {
            return Some(Blocker::Strength);
        }
        None
    }
    pub(super) fn manual_blocked(&self) -> bool {
        self.manual_blocker().is_some()
    }
    fn repair_manual_target(&mut self, blocker: Blocker) {
        let Some(photo) = self.photos.get_mut(self.current) else {
            return;
        };
        let before = photo.edit.clone();
        match blocker {
            Blocker::Locked | Blocker::Covered => {
                if photo.edit.stack.layers.len() >= 32 {
                    self.notify("The layer limit is 32. Select a visible unlocked layer.");
                    return;
                }
                photo.edit.add_layer(LayerType::Retouch);
                while photo.edit.move_layer(true) {}
            }
            Blocker::Hidden => {
                photo.edit.toggle_layer_visibility(photo.edit.stack.active);
            }
            Blocker::Transparent => {
                photo.edit.active_layer_mut().unwrap().opacity = 100.;
            }
            Blocker::Group(kind) => {
                let group = photo.edit.layers.get_mut(kind);
                group.visible = true;
                group.opacity = 100.;
            }
            Blocker::HealingDisabled => {
                photo
                    .edit
                    .settings
                    .disabled
                    .retain(|a| *a != Adjustment::Healing);
                photo.edit.settings.healing = 100.;
            }
            Blocker::Strength => self.brush_strength = 100.,
        }
        if photo.edit != before {
            photo.history.record(before);
            self.edited();
        }
        self.end_canvas_gesture();
        self.end_layer_interaction();
    }
    pub(super) fn manual_workspace_bar(&mut self, ui: &mut egui::Ui) {
        let Some(target) = self.brush.filter(|t| is_manual(*t)) else {
            return;
        };
        if self.photos.is_empty() {
            return;
        }
        if let Some(blocker) = self.manual_blocker() {
            ui.add_sized(
                vec2((ui.available_width() - 185.).max(70.), 34.),
                egui::Label::new(RichText::new(blocker.text()).size(11.).color(YELLOW)).truncate(),
            )
            .on_hover_text(blocker.text());
            let fix = ui.button(blocker.action());
            hit(ui, "manual-target-fix", fix.rect);
            if fix.clicked() {
                self.repair_manual_target(blocker);
            }
        } else {
            let text = match target {
                Target::Liquify => "Liquify · drag to push/pull",
                Target::Heal => "Spot heal · click or drag over a defect",
                Target::Clone if self.clone_pick_source => "Clone · click a clean source area",
                Target::Clone
                    if !self
                        .clone_anchor
                        .is_some_and(|(id, _)| id == self.photos[self.current].id) =>
                {
                    "Clone · choose a source before painting"
                }
                Target::Clone => "Clone · paint to copy texture · Alt+click resamples",
                Target::Patch if self.patch_draft.drawing => "Patch · trace the repair area",
                Target::Patch if !self.patch_draft.boundary.is_empty() => {
                    "Patch · drag inside the outline onto clean texture"
                }
                Target::Patch => "Patch · draw around the area to repair",
                _ => unreachable!(),
            };
            let reserve = if target == Target::Clone {
                140.
            } else if target == Target::Patch {
                120.
            } else {
                0.
            };
            ui.add_sized(
                vec2((ui.available_width() - reserve).max(70.), 34.),
                egui::Label::new(RichText::new(text).size(11.).color(YELLOW)).truncate(),
            )
            .on_hover_text(text);
            if target == Target::Clone {
                let pick = ui.button("Choose source");
                hit(ui, "clone-pick-source", pick.rect);
                if pick.clicked() {
                    self.end_canvas_gesture();
                    self.clone_pick_source = true;
                }
            }
            if target == Target::Patch && !self.patch_draft.boundary.is_empty() {
                let clear = ui.button("Clear selection");
                hit(ui, "patch-clear", clear.rect);
                if clear.clicked() {
                    self.patch_draft = PatchDraft::default();
                    self.end_canvas_gesture();
                }
            }
        }
    }
    pub(super) fn reject_manual_stroke(&mut self) {
        if let Some(blocker) = self.manual_blocker() {
            self.notify(blocker.text());
        }
        self.end_canvas_gesture();
    }
    fn patch_candidate(&self) -> Option<crate::cleanup::PatchStroke> {
        if self.brush != Some(Target::Patch)
            || self.manual_blocked()
            || self.patch_draft.drawing
            || self.patch_draft.boundary.len() < 3
        {
            return None;
        }
        let (start, end) = self.patch_draft.source_drag?;
        if (end[0] - start[0]).abs() + (end[1] - start[1]).abs() < 0.000001 {
            return None;
        }
        let p = self.photos.get(self.current)?;
        let mapping =
            crate::geometry::Mapping::new(&p.edit, p.seg.as_deref(), p.photo.original.dimensions());
        let a = mapping.source(start);
        let b = mapping.source(end);
        Some(crate::cleanup::PatchStroke {
            boundary: self
                .patch_draft
                .boundary
                .iter()
                .map(|p| mapping.source(*p))
                .collect(),
            offset: [b[0] - a[0], b[1] - a[1]],
            softness: self.brush_softness,
            strength: self.brush_strength,
        })
    }
    pub(super) fn schedule_patch_preview(
        &mut self,
        ctx: &egui::Context,
        crop: Option<engine::Crop>,
    ) {
        let Some(patch) = self.patch_candidate() else {
            if self.patch_feedback.frame.as_ref().is_some_and(|f| {
                f.committed
                    && self.photos.get(self.current).is_some_and(|p| {
                        p.id == f.id
                            && p.revision == f.revision
                            && match crop {
                                Some(crop) => !self.detail_region.as_ref().is_some_and(
                                    |(id, rev, ready, _)| {
                                        *id == p.id
                                            && *rev == p.revision
                                            && crop_contains(*ready, crop)
                                    },
                                ),
                                None => p.rendered_revision != p.revision,
                            }
                    })
            }) {
                return;
            }
            self.patch_feedback.clear();
            return;
        };
        let p = &self.photos[self.current];
        if let Some(pending) = &self.patch_feedback.pending {
            if pending.id == p.id && pending.revision == p.revision && pending.crop == crop {
                ctx.request_repaint_after(Duration::from_millis(16));
                return;
            }
            self.patch_feedback.clear();
        }
        if self.patch_feedback.frame.as_ref().is_some_and(|f| {
            f.id == p.id && f.revision == p.revision && f.crop == crop && f.patch == patch
        }) {
            return;
        }
        let image = if crop.is_some() {
            p.photo.original.clone()
        } else {
            p.photo.preview.clone()
        };
        let mut edit = p.edit.clone();
        edit.patches.push(patch.clone());
        edit.sync_active_layer();
        self.patch_feedback.serial += 1;
        let request = self.patch_feedback.serial;
        let cancel = Arc::new(AtomicBool::new(false));
        if self
            .tx
            .send(Job::PatchPreview {
                id: p.id,
                revision: p.revision,
                request,
                image,
                edit,
                seg: p.seg.clone(),
                crop,
                cancel: cancel.clone(),
            })
            .is_ok()
        {
            self.patch_feedback.pending = Some(PatchRequest {
                id: p.id,
                revision: p.revision,
                request,
                crop,
                patch,
                cancel,
            });
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
    pub(super) fn accept_patch_preview(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        revision: u64,
        request: u64,
        crop: Option<engine::Crop>,
        result: anyhow::Result<RgbaImage>,
    ) {
        if !self.patch_feedback.pending.as_ref().is_some_and(|p| {
            p.id == id && p.revision == revision && p.request == request && p.crop == crop
        }) {
            return;
        }
        let pending = self.patch_feedback.pending.take().unwrap();
        if self.brush != Some(Target::Patch)
            || self.patch_draft.source_drag.is_none()
            || self.manual_blocked()
        {
            return;
        }
        let Some(p) = self
            .photos
            .get(self.current)
            .filter(|p| p.id == id && p.revision == revision)
        else {
            return;
        };
        match result {
            Ok(image) => {
                // Source pixels stay fixed while the donor moves. Preserve the
                // crop allocation and GPU upload across feedback frames.
                let previous = self
                    .patch_feedback
                    .frame
                    .as_ref()
                    .filter(|f| f.id == id && f.crop == crop);
                let original = if let Some(previous) = previous {
                    previous.original.clone()
                } else if let Some(crop) = crop {
                    Arc::new(
                        image::imageops::crop_imm(
                            &*p.photo.original,
                            crop.x,
                            crop.y,
                            crop.width,
                            crop.height,
                        )
                        .to_image(),
                    )
                } else {
                    p.photo.preview.clone()
                };
                let texture = if let Some(previous) = previous {
                    let mut texture = previous.texture.clone();
                    if !self.gpu {
                        texture.set(
                            egui::ColorImage::from_rgba_unmultiplied(
                                [image.width() as usize, image.height() as usize],
                                image.as_raw(),
                            ),
                            egui::TextureOptions::LINEAR,
                        );
                    }
                    texture
                } else {
                    canvas_texture(ctx, "patch-feedback", &image, self.gpu)
                };
                let original_texture = previous.map_or_else(
                    || canvas_texture(ctx, "patch-feedback-before", &original, self.gpu),
                    |frame| frame.original_texture.clone(),
                );
                self.patch_feedback.frame = Some(PatchFrame {
                    id,
                    revision,
                    request,
                    crop,
                    image: Arc::new(image),
                    original,
                    texture,
                    original_texture,
                    patch: pending.patch,
                    committed: false,
                });
            }
            Err(error) if !pending.cancel.load(Ordering::Relaxed) => {
                self.notify(format!("Patch preview failed: {error}"))
            }
            Err(_) => {}
        }
    }
    pub(super) fn commit_patch_feedback(&mut self, patch: &crate::cleanup::PatchStroke) {
        if let Some(pending) = self.patch_feedback.pending.take() {
            pending.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(frame) = &mut self.patch_feedback.frame
            && frame.patch == *patch
        {
            frame.revision = self.photos[self.current].revision;
            frame.committed = true;
        } else {
            self.patch_feedback.frame = None;
        }
    }
}
