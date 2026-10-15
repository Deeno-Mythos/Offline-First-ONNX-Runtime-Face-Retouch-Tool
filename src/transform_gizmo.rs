//! Frame handles update GPU geometry immediately; pixels are rebuilt once on Apply.
use super::*;
use crate::image_transform::{CropFrame, ImageTransform};

const HANDLES: [[f32; 2]; 8] = [
    [-1., -1.],
    [0., -1.],
    [1., -1.],
    [1., 0.],
    [1., 1.],
    [0., 1.],
    [-1., 1.],
    [-1., 0.],
];
#[derive(Clone, Copy, PartialEq)]
enum Aspect {
    Free,
    Original,
    Square,
    Portrait,
    Photo,
    Wide,
}

impl Aspect {
    fn label(self) -> &'static str {
        match self {
            Self::Free => "Free",
            Self::Original => "Original",
            Self::Square => "1:1",
            Self::Portrait => "4:5",
            Self::Photo => "3:2",
            Self::Wide => "16:9",
        }
    }
    fn ratio(self, canvas: (u32, u32)) -> Option<f32> {
        match self {
            Self::Free => None,
            Self::Original => Some(canvas.0 as f32 / canvas.1 as f32),
            Self::Square => Some(1.),
            Self::Portrait => Some(0.8),
            Self::Photo => Some(1.5),
            Self::Wide => Some(16. / 9.),
        }
    }
}
pub(super) struct TransformDraft {
    id: u64,
    revision: u64,
    original: ImageTransform,
    pub(super) target: ImageTransform,
    source: (u32, u32),
    aspect: Aspect,
    pub(super) center: Option<Pos2>,
    pub(super) scale: f32,
    fit_requested: bool,
    drag: Option<Drag>,
    texture: Option<egui::TextureHandle>,
    checker: Option<egui::TextureHandle>,
    preview_cancel: Arc<AtomicBool>,
    pub(super) preview_pending: bool,
    error: Option<String>,
}
impl Drop for TransformDraft {
    fn drop(&mut self) {
        self.preview_cancel.store(true, Ordering::Relaxed);
    }
}
#[derive(Clone, Copy)]
enum Drag {
    Scale {
        handle: usize,
        anchor: Pos2,
        size: Vec2,
        angle: f32,
    },
    Rotate {
        center: Pos2,
        start: f32,
        frame: CropFrame,
    },
    Move {
        start: Pos2,
        frame: CropFrame,
        locked_axis: Option<usize>,
    },
    New {
        start: Pos2,
    },
    Pan {
        previous: Pos2,
    },
}
fn rotate(v: Vec2, angle: f32) -> Vec2 {
    let (s, c) = angle.to_radians().sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}
fn normalized_angle(angle: f32) -> f32 {
    (angle + 180.).rem_euclid(360.) - 180.
}
fn edge_pan_delta(pointer: Pos2, viewport: Rect, dt: f32) -> Vec2 {
    let edge = 28.;
    let velocity = |position: f32, min: f32, max: f32| {
        if position < min + edge {
            ((min + edge - position) / edge).clamp(0., 1.)
        } else if position > max - edge {
            -((position - (max - edge)) / edge).clamp(0., 1.)
        } else {
            0.
        }
    };
    vec2(
        velocity(pointer.x, viewport.min.x, viewport.max.x),
        velocity(pointer.y, viewport.min.y, viewport.max.y),
    ) * (420. * dt.clamp(0.001, 0.05))
}
/// Clip image UVs to the photograph instead of letting the sampler stretch its edges.
fn clip_polygon(mut polygon: Vec<Pos2>, rect: Rect) -> Vec<Pos2> {
    for (axis, edge, minimum) in [
        (0, rect.min.x, true),
        (0, rect.max.x, false),
        (1, rect.min.y, true),
        (1, rect.max.y, false),
    ] {
        let mut clipped = Vec::with_capacity(8);
        if let Some(&last) = polygon.last() {
            let mut previous = last;
            let inside = |p: Pos2| {
                if minimum {
                    p[axis] >= edge
                } else {
                    p[axis] <= edge
                }
            };
            for &current in &polygon {
                if inside(previous) != inside(current) {
                    let fraction = (edge - previous[axis]) / (current[axis] - previous[axis]);
                    clipped.push(previous + (current - previous) * fraction);
                }
                if inside(current) {
                    clipped.push(current);
                }
                previous = current;
            }
        }
        polygon = clipped;
    }
    polygon
}
fn polygon_image(
    painter: &egui::Painter,
    texture: egui::TextureId,
    polygon: &[Pos2],
    uv: impl Fn(Pos2) -> Pos2,
) {
    if polygon.len() < 3 {
        return;
    }
    let mut mesh = egui::Mesh::with_texture(texture);
    for &pos in polygon {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: uv(pos),
            color: Color32::WHITE,
        });
    }
    for i in 1..polygon.len() as u32 - 1 {
        mesh.indices.extend_from_slice(&[0, i, i + 1]);
    }
    painter.add(mesh);
}
impl TransformDraft {
    pub(super) fn request_fit(&mut self) {
        self.fit_requested = true;
        self.center = None;
        self.drag = None;
    }
    fn fit_to_viewport(&mut self, viewport: Rect) {
        let (w, h) = self.canvas();
        let photo = Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32));
        let f = self.frame();
        let center = photo.min + photo.size() * vec2(f.center[0], f.center[1]);
        let size = photo.size() * vec2(f.extent[0], f.extent[1]);
        let bounds = [HANDLES[0], HANDLES[2], HANDLES[4], HANDLES[6]]
            .into_iter()
            .fold(photo, |bounds, corner| {
                bounds.union(Rect::from_min_max(
                    center + rotate(size * vec2(corner[0], corner[1]) * 0.5, f.angle),
                    center + rotate(size * vec2(corner[0], corner[1]) * 0.5, f.angle),
                ))
            });
        let padding = (viewport.size() * 0.12).max(vec2(52., 52.));
        let room = (viewport.size() - padding * 2.).max(vec2(40., 40.));
        self.scale = (room.x / bounds.width()).min(room.y / bounds.height());
        self.center = Some(viewport.center() + (photo.center() - bounds.center()) * self.scale);
        self.fit_requested = false;
    }
    fn frame(&self) -> CropFrame {
        self.target.crop.unwrap_or_default()
    }
    fn canvas(&self) -> (u32, u32) {
        self.target.uncropped_dimensions(self.source)
    }
    fn canvas_size(&self) -> Vec2 {
        let (w, h) = self.canvas();
        vec2(w as f32, h as f32) * self.scale
    }
    fn image_rect(&self) -> Rect {
        Rect::from_center_size(self.center.unwrap(), self.canvas_size())
    }
    fn frame_geometry(&self) -> (Pos2, Vec2, f32) {
        let f = self.frame();
        let rect = self.image_rect();
        (
            rect.min + rect.size() * vec2(f.center[0], f.center[1]),
            rect.size() * vec2(f.extent[0], f.extent[1]),
            f.angle,
        )
    }
    fn handles_geometry(&self) -> (Pos2, Vec2, f32) {
        self.frame_geometry()
    }
    fn valid(&self) -> bool {
        let (w, h) = self.target.dimensions(self.source);
        self.target.validate().is_ok()
            && w <= 32768
            && h <= 32768
            && u64::from(w) * u64::from(h) <= 80_000_000
    }
    /// The aperture can move beyond any photo edge without changing its dimensions.
    fn set_frame(&mut self, mut f: CropFrame) {
        let (w, h) = self.canvas();
        let dims = vec2(w as f32, h as f32);
        f.extent = [
            f.extent[0].clamp(1. / dims.x, 32768.),
            f.extent[1].clamp(1. / dims.y, 32768.),
        ];
        self.target.crop = if f.is_full() { None } else { Some(f) };
    }
    fn move_frame(&mut self, mut frame: CropFrame, mut pixels: Vec2, constrain: bool) {
        if constrain {
            if pixels.x.abs() >= pixels.y.abs() {
                pixels.y = 0.;
            } else {
                pixels.x = 0.;
            }
        }
        let (w, h) = self.canvas();
        // Native-pixel increments avoid resampling and accumulating pointer rounding.
        pixels = pixels.round();
        frame.center[0] += pixels.x / w as f32;
        frame.center[1] += pixels.y / h as f32;
        self.set_frame(frame);
    }
    fn set_aspect(&mut self, aspect: Aspect) {
        self.aspect = aspect;
        if let Some(ratio) = aspect.ratio(self.canvas()) {
            let mut f = self.frame();
            let (w, h) = self.canvas();
            let width = (f.extent[0] * w as f32).min(f.extent[1] * h as f32 * ratio);
            f.extent = [width / w as f32, width / ratio / h as f32];
            self.set_frame(f);
        }
    }
}
impl HasturApp {
    pub(super) fn open_crop(&mut self) {
        self.end_canvas_gesture();
        let Some(p) = self
            .photos
            .get(self.current)
            .filter(|p| p.photo.resident && p.document_ready())
        else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        // Uncropped edits already have a sharp completed overview. Re-open hidden margins
        // asynchronously only when necessary; no native allocation on pointer events.
        let needs_preview = p.edit.transform.crop.is_some() || p.rendered_revision != p.revision;
        if needs_preview {
            let _ = self.tx.send(Job::FramePreview {
                id: p.id,
                revision: p.revision,
                source: p.canonical_photo().clone(),
                edit: p.edit.clone(),
                seg: p.canonical_seg().cloned(),
                cancel: cancel.clone(),
            });
        }
        self.transform_draft = Some(TransformDraft {
            id: p.id,
            revision: p.revision,
            original: p.edit.transform,
            target: p.edit.transform,
            source: p.canonical_photo().dimensions(),
            aspect: Aspect::Free,
            center: None,
            scale: 1.,
            fit_requested: p.view.fit,
            drag: None,
            texture: None,
            checker: None,
            preview_cancel: cancel,
            preview_pending: needs_preview,
            error: None,
        });
        self.brush = None;
        self.compare = false;
        self.library_open = false;
        self.patch_draft = PatchDraft::default();
    }
    pub(super) fn accept_frame_preview(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        revision: u64,
        result: anyhow::Result<RgbaImage>,
    ) {
        let Some(d) = self.transform_draft.as_mut().filter(|d| {
            d.id == id && d.revision == revision && !d.preview_cancel.load(Ordering::Relaxed)
        }) else {
            return;
        };
        d.preview_pending = false;
        match result {
            Ok(image) => d.texture = Some(texture(ctx, "frame-overview", &image)),
            Err(e) => d.error = Some(format!("Could not prepare crop preview: {e:#}")),
        }
    }
    fn apply_transform(&mut self) {
        if let Some(draft) = self.transform_draft.take() {
            if !draft.valid() {
                self.transform_draft = Some(draft);
                return;
            }
            self.set_document_transform(draft.target);
        }
    }
    pub(super) fn rotate_frame(&mut self, turns: u8) {
        if self.transform_draft.is_none() {
            self.open_crop();
        }
        if let Some(d) = &mut self.transform_draft {
            let mut f = d.frame();
            f.angle = normalized_angle(f.angle + turns as f32 * 90.);
            d.set_frame(f);
        }
    }
    pub(super) fn transform_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.transform_draft.as_ref().is_some_and(|d| {
            self.photos
                .get(self.current)
                .is_none_or(|p| p.id != d.id || p.edit.transform != d.original)
        }) {
            self.transform_draft = None;
        }
        if self.transform_draft.is_some() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.transform_draft = None;
            } else if !ctx.wants_keyboard_input()
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            {
                self.apply_transform();
            } else if !ctx.wants_keyboard_input() {
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Num0)) {
                    self.transform_draft.as_mut().unwrap().request_fit();
                    return true;
                }
                for (key, delta) in [
                    (egui::Key::ArrowLeft, vec2(-1., 0.)),
                    (egui::Key::ArrowRight, vec2(1., 0.)),
                    (egui::Key::ArrowUp, vec2(0., -1.)),
                    (egui::Key::ArrowDown, vec2(0., 1.)),
                ] {
                    let step = ctx.input_mut(|i| {
                        if i.consume_key(egui::Modifiers::SHIFT, key) {
                            10.
                        } else if i.consume_key(egui::Modifiers::NONE, key) {
                            1.
                        } else {
                            0.
                        }
                    });
                    if step != 0. {
                        let d = self.transform_draft.as_mut().unwrap();
                        d.move_frame(d.frame(), delta * step, false);
                        ctx.request_repaint();
                    }
                }
            }
            return true;
        }
        if !ctx.wants_keyboard_input() && !egui::Popup::is_any_open(ctx) {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::T)) {
                self.open_crop();
                return true;
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::C)) {
                self.open_crop();
                return true;
            }
        }
        false
    }
    pub(super) fn transform_bar(&mut self, ui: &mut egui::Ui) {
        let Some(d) = self.transform_draft.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Crop").strong());
            {
                let previous = d.aspect;
                let r = egui::ComboBox::from_id_salt("crop-aspect")
                    .selected_text(d.aspect.label())
                    .width(75.)
                    .show_ui(ui, |ui| {
                        for aspect in [
                            Aspect::Free,
                            Aspect::Original,
                            Aspect::Square,
                            Aspect::Portrait,
                            Aspect::Photo,
                            Aspect::Wide,
                        ] {
                            let r = ui.selectable_value(&mut d.aspect, aspect, aspect.label());
                            hit(ui, &format!("crop-aspect-{}", aspect.label()), r.rect);
                        }
                    });
                hit(ui, "crop-aspect", r.response.rect);
                if previous != d.aspect {
                    d.set_aspect(d.aspect);
                }
            }
            ui.label(RichText::new("Frame").color(MUTED));
            let mut angle = d.frame().angle;
            let r = ui.add(
                egui::DragValue::new(&mut angle)
                    .range(-180.0..=180.0)
                    .speed(0.2)
                    .suffix("°"),
            );
            hit(ui, "transform-angle", r.rect);
            if r.changed() {
                let mut f = d.frame();
                f.angle = angle;
                d.set_frame(f);
            }
            r.on_hover_text("Rotate the crop frame. The photo stays upright.");
            let r = small_chip(ui, "Reset frame", false);
            hit(ui, "crop-reset", r.rect);
            if r.clicked() {
                d.target.crop = None;
                d.aspect = Aspect::Free;
            }
            let (w, h) = d.target.dimensions(d.source);
            ui.label(
                RichText::new(format!("{w} × {h} px"))
                    .size(11.)
                    .color(MUTED),
            );
            let r = ui.add_enabled(d.valid(), primary("Apply").min_size(vec2(58., 32.)));
            hit(ui, "transform-apply", r.rect);
            apply = r.clicked();
            let r = ui.add(secondary("Cancel").min_size(vec2(58., 32.)));
            hit(ui, "transform-cancel", r.rect);
            cancel = r.clicked();
        });
        if apply {
            self.apply_transform();
        }
        if cancel {
            self.transform_draft = None;
        }
    }
    pub(super) fn transform_canvas(
        &mut self,
        ui: &mut egui::Ui,
        viewport: Rect,
        image_rect: Rect,
    ) -> bool {
        let Some(mut d) = self.transform_draft.take() else {
            return false;
        };
        let Some(p) = self.photos.get(self.current).filter(|p| p.id == d.id) else {
            return false;
        };
        if d.texture.is_none() && !d.preview_pending && d.error.is_none() {
            d.texture = Some(texture(ui.ctx(), "frame-overview", &p.edited));
        }
        if d.center.is_none() {
            let dims = d.canvas();
            if d.fit_requested {
                d.fit_to_viewport(viewport);
            } else {
                d.scale = image_rect.width() / p.photo.dimensions().0 as f32;
                // Preserve the existing zoom/pan when re-opening cropped margins.
                let crop = d.original.crop.map(|f| f.bounds(d.canvas()));
                let offset = crop.map_or(Vec2::ZERO, |b| {
                    vec2(
                        b.x as f32 + b.width as f32 * 0.5 - dims.0 as f32 * 0.5,
                        b.y as f32 + b.height as f32 * 0.5 - dims.1 as f32 * 0.5,
                    ) * d.scale
                });
                d.center = Some(image_rect.center() - offset);
            }
        }
        let response = ui.interact(
            viewport,
            ui.id().with("transform-canvas"),
            Sense::click_and_drag(),
        );
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let space = ui.input(|i| i.key_down(egui::Key::Space));
        let shift = ui.input(|i| i.modifiers.shift);
        if response.hovered() && d.drag.is_none() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.01 {
                let multiplier = (scroll
                    * 0.003
                    * self.preferences.zoom_speed
                    * if self.preferences.invert_scroll {
                        -1.
                    } else {
                        1.
                    })
                .exp();
                let next = (d.scale * multiplier).clamp(0.001, 8.);
                let anchor = pointer.unwrap_or(viewport.center());
                d.center = Some(anchor + (d.center.unwrap() - anchor) * (next / d.scale));
                d.scale = next;
            }
        }
        if ui.input(|i| i.pointer.primary_down())
            && let Some(pos) = pointer
            && let Some(drag) = d.drag
            && !matches!(drag, Drag::Pan { .. })
        {
            let mut delta = edge_pan_delta(pos, viewport, ui.input(|i| i.stable_dt));
            if shift
                && let Drag::Move {
                    start, locked_axis, ..
                } = drag
            {
                let movement = pos - start;
                let axis =
                    locked_axis.unwrap_or_else(|| usize::from(movement.y.abs() > movement.x.abs()));
                delta[1 - axis] = 0.;
            }
            if delta != Vec2::ZERO {
                d.center = Some(d.center.unwrap() + delta);
                // Gesture anchors follow the camera, so the active point stays under
                // the cursor while the document scrolls beyond the viewport edge.
                d.drag = Some(match drag {
                    Drag::Move {
                        start,
                        frame,
                        locked_axis,
                    } => Drag::Move {
                        start: start + delta,
                        frame,
                        locked_axis,
                    },
                    Drag::Scale {
                        handle,
                        anchor,
                        size,
                        angle,
                    } => Drag::Scale {
                        handle,
                        anchor: anchor + delta,
                        size,
                        angle,
                    },
                    Drag::Rotate {
                        center,
                        start,
                        frame,
                    } => Drag::Rotate {
                        center: center + delta,
                        start,
                        frame,
                    },
                    Drag::New { start } => Drag::New {
                        start: start + delta,
                    },
                    Drag::Pan { .. } => unreachable!(),
                });
                ui.ctx().request_repaint_after(Duration::from_millis(16));
            }
        }
        let (center, size, angle) = d.handles_geometry();
        let positions = HANDLES.map(|v| center + rotate(size * vec2(v[0], v[1]) * 0.5, angle));
        let (fc, fs, fa) = d.frame_geometry();
        let rotation = fc + rotate(vec2(0., -fs.y * 0.5 - 30.), fa);
        let hovered = pointer.and_then(|pos| {
            positions
                .iter()
                .enumerate()
                .filter(|(_, p)| p.distance(pos) <= 12.)
                .min_by(|(_, a), (_, b)| a.distance(pos).total_cmp(&b.distance(pos)))
                .map(|(i, _)| i)
        });
        let move_hovered = pointer.is_some_and(|pos| pos.distance(fc) <= 7.);
        let rotate_hovered = pointer.is_some_and(|pos| pos.distance(rotation) <= 13.);
        if ui.input(|i| i.pointer.primary_pressed())
            && response.contains_pointer()
            && let Some(pos) = pointer
        {
            let local = rotate(pos - fc, -fa);
            let inside_frame = local.x.abs() <= fs.x * 0.5 && local.y.abs() <= fs.y * 0.5;
            d.drag = if space {
                Some(Drag::Pan { previous: pos })
            } else if rotate_hovered {
                Some(Drag::Rotate {
                    center: fc,
                    start: (pos - fc).angle(),
                    frame: d.frame(),
                })
            } else if move_hovered {
                Some(Drag::Move {
                    start: pos,
                    frame: d.frame(),
                    locked_axis: None,
                })
            } else if let Some(handle) = hovered {
                Some(Drag::Scale {
                    handle,
                    anchor: center
                        - rotate(
                            size * vec2(HANDLES[handle][0], HANDLES[handle][1]) * 0.5,
                            angle,
                        ),
                    size,
                    angle,
                })
            } else if inside_frame {
                Some(Drag::Move {
                    start: pos,
                    frame: d.frame(),
                    locked_axis: None,
                })
            } else if d.image_rect().contains(pos) {
                Some(Drag::New { start: pos })
            } else {
                None
            };
        }
        if ui.input(|i| i.pointer.primary_down() || i.pointer.primary_released())
            && let (Some(drag), Some(pos)) = (d.drag, pointer)
        {
            match drag {
                Drag::Rotate {
                    center,
                    start,
                    mut frame,
                } => {
                    let next = normalized_angle(
                        frame.angle + ((pos - center).angle() - start).to_degrees(),
                    );
                    frame.angle = if shift {
                        (next / 15.).round() * 15.
                    } else {
                        next
                    };
                    d.set_frame(frame);
                }
                Drag::Scale {
                    handle,
                    anchor,
                    size,
                    angle,
                } => {
                    let v = vec2(HANDLES[handle][0], HANDLES[handle][1]);
                    let delta = rotate(pos - anchor, -angle);
                    let mut next = vec2(
                        if v.x == 0. {
                            size.x
                        } else {
                            (delta.x * v.x).max(d.scale)
                        },
                        if v.y == 0. {
                            size.y
                        } else {
                            (delta.y * v.y).max(d.scale)
                        },
                    );
                    let ratio = d
                        .aspect
                        .ratio(d.canvas())
                        .or_else(|| shift.then_some(size.x / size.y));
                    if let Some(ratio) = ratio {
                        if v.x == 0. {
                            next.x = next.y * ratio;
                        } else if v.y == 0. {
                            next.y = next.x / ratio;
                        } else {
                            let fraction = next.dot(size) / size.length_sq();
                            next = size * fraction.max(d.scale / size.min_elem());
                        }
                    }
                    {
                        let mut f = d.frame();
                        let rect = d.image_rect();
                        let new_center = anchor + rotate(next * v * 0.5, angle);
                        let normalized = (new_center - rect.min) / rect.size();
                        f.center = [normalized.x, normalized.y];
                        f.extent = [next.x / rect.width(), next.y / rect.height()];
                        d.set_frame(f);
                    }
                }
                Drag::Move {
                    start,
                    frame,
                    mut locked_axis,
                } => {
                    let mut pixels = (pos - start) / d.scale;
                    if shift && pixels != Vec2::ZERO {
                        let axis = *locked_axis
                            .get_or_insert_with(|| usize::from(pixels.y.abs() > pixels.x.abs()));
                        pixels[1 - axis] = 0.;
                    } else if !shift {
                        locked_axis = None;
                    }
                    d.move_frame(frame, pixels, shift);
                    d.drag = Some(Drag::Move {
                        start,
                        frame,
                        locked_axis,
                    });
                }
                Drag::New { start } => {
                    let rect = d.image_rect();
                    let start = start.clamp(rect.min, rect.max);
                    let end = pos.clamp(rect.min, rect.max);
                    let mut size = (end - start).abs().max(vec2(d.scale, d.scale));
                    if let Some(ratio) = d.aspect.ratio(d.canvas()).or_else(|| shift.then_some(1.))
                    {
                        size.x = size.x.min(size.y * ratio);
                        size.y = size.x / ratio;
                    }
                    let end = start
                        + size
                            * vec2(
                                if end.x >= start.x { 1. } else { -1. },
                                if end.y >= start.y { 1. } else { -1. },
                            );
                    let center = (start + (end - start) * 0.5 - rect.min) / rect.size();
                    d.set_frame(CropFrame {
                        center: [center.x, center.y],
                        extent: [size.x / rect.width(), size.y / rect.height()],
                        angle: 0.,
                    });
                }
                Drag::Pan { previous } => {
                    d.center = Some(d.center.unwrap() + (pos - previous));
                    d.drag = Some(Drag::Pan { previous: pos });
                }
            }
            ui.ctx().request_repaint();
        }
        if ui.input(|i| i.pointer.primary_released() || !i.pointer.primary_down()) {
            d.drag = None;
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(if space {
                if d.drag.is_some() {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Grab
                }
            } else if rotate_hovered || matches!(d.drag, Some(Drag::Rotate { .. })) {
                egui::CursorIcon::Crosshair
            } else if move_hovered || matches!(d.drag, Some(Drag::Move { .. })) {
                egui::CursorIcon::Move
            } else if hovered.is_some() || matches!(d.drag, Some(Drag::Scale { .. })) {
                egui::CursorIcon::ResizeNwSe
            } else {
                let local = rotate(pointer.unwrap() - fc, -fa);
                if local.x.abs() <= fs.x * 0.5 && local.y.abs() <= fs.y * 0.5 {
                    egui::CursorIcon::Move
                } else {
                    egui::CursorIcon::Crosshair
                }
            });
        }
        let painter = ui.painter().with_clip_rect(viewport);
        let image = d.image_rect();
        hit(ui, "image", image);
        let (fc, fs, fa) = d.frame_geometry();
        let corners = [HANDLES[0], HANDLES[2], HANDLES[4], HANDLES[6]];
        let positions = corners.map(|v| fc + rotate(fs * vec2(v[0], v[1]) * 0.5, fa));
        let checker = d.checker.get_or_insert_with(|| {
            ui.ctx().load_texture(
                "crop-transparency",
                egui::ColorImage::new(
                    [2, 2],
                    vec![
                        Color32::from_gray(40),
                        Color32::from_gray(55),
                        Color32::from_gray(55),
                        Color32::from_gray(40),
                    ],
                ),
                egui::TextureOptions::NEAREST_REPEAT,
            )
        });
        polygon_image(&painter, checker.id(), &positions, |pos| {
            let uv = (pos - fc) / 24.;
            pos2(uv.x, uv.y)
        });
        if let Some(texture) = &d.texture {
            painter.image(
                texture.id(),
                image,
                Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                Color32::from_gray(95),
            );
            let visible = clip_polygon(positions.to_vec(), image);
            polygon_image(&painter, texture.id(), &visible, |pos| {
                let uv = (pos - image.min) / image.size();
                pos2(uv.x.clamp(0., 1.), uv.y.clamp(0., 1.))
            });
        } else {
            painter.text(
                viewport.center(),
                Align2::CENTER_CENTER,
                d.error.as_deref().unwrap_or("Preparing full crop preview…"),
                FontId::proportional(14.),
                YELLOW,
            );
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        for i in 0..4 {
            painter.line_segment(
                [positions[i], positions[(i + 1) % 4]],
                Line::new(1_f32, YELLOW),
            );
        }
        {
            for fraction in [1. / 3., 2. / 3.] {
                for (a, b) in [
                    (vec2(fraction - 0.5, -0.5), vec2(fraction - 0.5, 0.5)),
                    (vec2(-0.5, fraction - 0.5), vec2(0.5, fraction - 0.5)),
                ] {
                    painter.line_segment(
                        [fc + rotate(fs * a, fa), fc + rotate(fs * b, fa)],
                        Line::new(0.75_f32, Color32::from_white_alpha(100)),
                    );
                }
            }
        }
        let (center, size, angle) = d.handles_geometry();
        let handles = HANDLES.map(|v| center + rotate(size * vec2(v[0], v[1]) * 0.5, angle));
        for (i, pos) in handles.into_iter().enumerate() {
            let r = Rect::from_center_size(pos, vec2(9., 9.));
            painter.rect_filled(r.expand(1.), 1., BG);
            painter.rect_filled(r, 1., TEXT);
            hit(
                ui,
                &format!("transform-handle-{i}"),
                Rect::from_center_size(pos, vec2(24., 24.)),
            );
        }
        let top = fc + rotate(vec2(0., -fs.y * 0.5), fa);
        let rotation = top + rotate(vec2(0., -30.), fa);
        painter.line_segment([top, rotation], Line::new(1_f32, YELLOW));
        painter.circle_filled(rotation, 7., YELLOW);
        painter.circle_stroke(rotation, 8., Line::new(1_f32, BG));
        hit(
            ui,
            "transform-rotate",
            Rect::from_center_size(rotation, vec2(26., 26.)),
        );
        hit(
            ui,
            "crop-frame",
            positions
                .iter()
                .fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(*p, *p))),
        );
        painter.circle_filled(fc, 8., BG);
        for axis in [vec2(5., 0.), vec2(0., 5.)] {
            painter.line_segment([fc - axis, fc + axis], Line::new(1.5_f32, YELLOW));
        }
        hit(ui, "crop-move", Rect::from_center_size(fc, vec2(14., 14.)));
        let instructions =
            "Drag inside or + to move · hold at canvas edge to keep moving · Shift locks X/Y";
        painter.text(viewport.center_bottom() - vec2(0., 12.), Align2::CENTER_BOTTOM, format!("{instructions}\nSpace pans freely · 0 / Fit shows frame + photo · Arrows nudge · Enter applies · Esc cancels"), FontId::proportional(11.), MUTED);
        self.transform_draft = Some(d);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_frame_preview_clips_image_mesh_to_photo_and_keeps_outside_empty() {
        let image = Rect::from_min_max(pos2(0., 0.), pos2(100., 80.));
        for center in [pos2(5., 10.), pos2(95., 70.), pos2(-300., 70.)] {
            let corners = [HANDLES[0], HANDLES[2], HANDLES[4], HANDLES[6]];
            let frame = corners.map(|v| center + rotate(vec2(v[0] * 50., v[1] * 40.), 31.));
            let clipped = clip_polygon(frame.to_vec(), image);
            if center.x == -300. {
                assert!(clipped.is_empty());
            } else {
                assert!(clipped.len() >= 3);
                for vertex in clipped {
                    assert!(image.expand(0.0001).contains(vertex));
                    let uv = (vertex - image.min) / image.size();
                    assert!((-0.00001..=1.00001).contains(&uv.x));
                    assert!((-0.00001..=1.00001).contains(&uv.y));
                }
            }
        }
    }
}
