//! Incremental native viewport pixels. Only manual edit footprints are
//! redrawn; structural, mask, lighting and layer changes use the full renderer.
use super::*;
use std::sync::Weak;

const FRAME_BYTES: usize = 32 * 1024 * 1024;
pub(super) struct FrameCache {
    source: Weak<RgbaImage>,
    seg: Option<Weak<Segmentation>>,
    edit: Edit,
    crop: Crop,
    image: RgbaImage,
    warp: crate::geometry::WarpCache,
    damage: Option<Crop>,
}
fn same_seg(a: &Option<Weak<Segmentation>>, b: Option<&Arc<Segmentation>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.ptr_eq(&Arc::downgrade(b)),
        _ => false,
    }
}
fn intersect(a: Crop, b: Crop) -> Crop {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Crop {
        x,
        y,
        width: (a.x + a.width).min(b.x + b.width).saturating_sub(x),
        height: (a.y + a.height).min(b.y + b.height).saturating_sub(y),
    }
}
fn union(a: Crop, b: Crop) -> Crop {
    if a.width == 0 || a.height == 0 {
        return b;
    }
    if b.width == 0 || b.height == 0 {
        return a;
    }
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Crop {
        x,
        y,
        width: (a.x + a.width).max(b.x + b.width) - x,
        height: (a.y + a.height).max(b.y + b.height) - y,
    }
}
#[derive(Default)]
struct Bounds(Option<[f32; 4]>);
impl Bounds {
    fn point(&mut self, p: [f32; 2], radius: f32, (w, h): (u32, u32)) {
        let r = radius * w.min(h) as f32;
        let p = [p[0] * w as f32, p[1] * h as f32];
        let add = [p[0] - r, p[1] - r, p[0] + r, p[1] + r];
        self.0 = Some(self.0.map_or(add, |b| {
            [
                b[0].min(add[0]),
                b[1].min(add[1]),
                b[2].max(add[2]),
                b[3].max(add[3]),
            ]
        }));
    }
    fn crop(&self, (w, h): (u32, u32), pad: [f32; 2]) -> Crop {
        let Some(b) = self.0 else {
            return Crop {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            };
        };
        let x = (b[0] - pad[0]).floor().clamp(0., w as f32) as u32;
        let y = (b[1] - pad[1]).floor().clamp(0., h as f32) as u32;
        let right = (b[2] + pad[0]).ceil().clamp(0., w as f32) as u32;
        let bottom = (b[3] + pad[1]).ceil().clamp(0., h as f32) as u32;
        Crop {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }
}

fn manual_damage(
    before: &Edit,
    after: &Edit,
    seg: Option<&Segmentation>,
    dims: (u32, u32),
    crop: Crop,
    warp: &mut crate::geometry::WarpCache,
) -> Option<Crop> {
    // A lower layer can feed filters/deformations in the layers above it. Do
    // not guess those dependencies: optimize the changing top layer only.
    let old = before.resolved_layers();
    let new = after.resolved_layers();
    let (a, b) = match (old.split_last(), new.split_last()) {
        (Some((a, below)), Some((b, next))) if below == next => {
            let mut meta = b.clone();
            meta.edit = a.edit.clone();
            if meta != *a {
                return None;
            }
            (a.edit.effective_layers(), b.edit.effective_layers())
        }
        (None, None) if before.stack.layers.is_empty() && after.stack.layers.is_empty() => {
            (before.effective_layers(), after.effective_layers())
        }
        _ => return None,
    };
    let a = a.as_ref();
    let b = b.as_ref();
    let mut fixed = b.clone();
    fixed.strokes = a.strokes.clone();
    fixed.clones = a.clones.clone();
    fixed.warps = a.warps.clone();
    fixed.patches = a.patches.clone();
    if fixed != *a
        || !b.strokes.starts_with(&a.strokes)
        || !b.clones.starts_with(&a.clones)
        || !b.warps.starts_with(&a.warps)
    {
        return None;
    }
    let patch_start = if b.patches.starts_with(&a.patches) {
        a.patches.len()
    } else if !a.patches.is_empty()
        && b.patches.len() == a.patches.len()
        && b.patches[..b.patches.len() - 1] == a.patches[..a.patches.len() - 1]
    {
        a.patches.len() - 1
    } else {
        return None;
    };
    if crate::geometry::active(a, seg)
        && crate::geometry::field_dimensions(a, dims) != crate::geometry::field_dimensions(b, dims)
    {
        return None;
    }
    let mut source = Bounds::default();
    let mut destination = Bounds::default();
    for stroke in &b.strokes[a.strokes.len()..] {
        if stroke.target != Target::Heal {
            return None;
        }
        source.point(
            stroke.center,
            stroke.radius.max(1. / dims.0.min(dims.1) as f32),
            dims,
        );
    }
    for clone in &b.clones[a.clones.len()..] {
        source.point(
            clone.center,
            clone.radius.max(1. / dims.0.min(dims.1) as f32),
            dims,
        );
    }
    for patch in b.patches[patch_start..]
        .iter()
        .chain(a.patches.get(patch_start..).unwrap_or(&[]))
    {
        for &point in &patch.boundary {
            source.point(point, 0., dims);
        }
    }
    for stroke in &b.warps[a.warps.len()..] {
        destination.point(stroke.center, stroke.radius.max(0.0001), dims);
    }
    let (gw, gh) = crate::geometry::field_dimensions(b, dims);
    let mut damage = destination.crop(
        dims,
        [
            dims.0 as f32 / gw as f32 + 3.,
            dims.1 as f32 / gh as f32 + 3.,
        ],
    );
    let source = source.crop(dims, [3., 3.]);
    if source.width != 0 && source.height != 0 {
        let mapped = if crate::geometry::active(b, seg) {
            warp.displacement(b, seg, dims)
                .output_damage(source, crop, dims)
        } else {
            source
        };
        damage = union(damage, mapped);
    }
    Some(intersect(damage, crop))
}

impl Renderer {
    /// Native brush/patch worker path. Cache one bounded frame and redraw only
    /// exact affected pixels. The returned frame remains a complete sharp crop.
    pub fn render_viewport(
        &mut self,
        image: &Arc<RgbaImage>,
        edit: &Edit,
        seg: Option<&Arc<Segmentation>>,
        crop: Crop,
        cancel: Option<&AtomicBool>,
    ) -> Result<RgbaImage> {
        check_cancel(cancel)?;
        let mut previous = self.viewport.take();
        let damage = previous
            .as_mut()
            .filter(|frame| {
                frame.source.ptr_eq(&Arc::downgrade(image))
                    && same_seg(&frame.seg, seg)
                    && frame.crop == crop
            })
            .and_then(|frame| {
                manual_damage(
                    &frame.edit,
                    edit,
                    seg.map(Arc::as_ref),
                    image.dimensions(),
                    crop,
                    &mut frame.warp,
                )
            });
        if let Some(damage) = damage {
            let mut frame = previous.unwrap();
            if damage.width != 0 && damage.height != 0 {
                let patch = match self.render_region(image, edit, seg, damage, cancel) {
                    Ok(patch) => patch,
                    Err(error) => {
                        self.viewport = Some(frame);
                        return Err(error);
                    }
                };
                image::imageops::replace(
                    &mut frame.image,
                    &patch,
                    (damage.x - crop.x) as i64,
                    (damage.y - crop.y) as i64,
                );
            }
            frame.edit = edit.clone();
            frame.damage = (damage.width != 0 && damage.height != 0).then_some(damage);
            let result = frame.image.clone();
            self.viewport = Some(frame);
            return Ok(result);
        }
        let result = self.render_region(image, edit, seg, crop, cancel)?;
        if result.as_raw().len() <= FRAME_BYTES {
            self.viewport = Some(FrameCache {
                source: Arc::downgrade(image),
                seg: seg.map(Arc::downgrade),
                edit: edit.clone(),
                crop,
                image: result.clone(),
                warp: Default::default(),
                damage: Some(crop),
            });
        }
        Ok(result)
    }
    /// Source pixels processed in the last viewport update (None: unchanged).
    pub fn viewport_damage(&self) -> Option<Crop> {
        self.viewport.as_ref().and_then(|f| f.damage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cleanup::{CloneStamp, PatchStroke},
        geometry::WarpStroke,
        layer_stack::LayerType,
    };

    fn fixture() -> (Arc<RgbaImage>, Crop) {
        let image = RgbaImage::from_fn(1234, 913, |x, y| {
            image::Rgba([
                (55 + (x * 7 + y * 3) % 150) as u8,
                (65 + (x * 3 + y * 5) % 140) as u8,
                (45 + (x * 5 + y * 11) % 160) as u8,
                (120 + (x + y) % 136) as u8,
            ])
        });
        (
            Arc::new(image),
            Crop {
                x: 137,
                y: 117,
                width: 950,
                height: 700,
            },
        )
    }
    fn append(edit: &mut Edit, tool: Target, center: [f32; 2]) {
        match tool {
            Target::Liquify => edit.warps.push(WarpStroke {
                center,
                delta: [0.008, -0.004],
                radius: 0.047,
                softness: 0.7,
                strength: 85.,
            }),
            Target::Heal => edit.strokes.push(Stroke {
                center,
                target: tool,
                radius: 0.036,
                softness: 0.7,
                erase: false,
                strength: 100.,
            }),
            Target::Clone => edit.clones.push(CloneStamp {
                center,
                source: [center[0] + 0.1, center[1] - 0.08],
                radius: 0.04,
                softness: 0.7,
                strength: 90.,
            }),
            Target::Patch => edit.patches.push(PatchStroke {
                boundary: vec![
                    [center[0] - 0.02, center[1] - 0.02],
                    [center[0] + 0.02, center[1] - 0.02],
                    [center[0] + 0.02, center[1] + 0.02],
                    [center[0] - 0.02, center[1] + 0.02],
                ],
                offset: [0.1, -0.08],
                softness: 0.7,
                strength: 95.,
            }),
            _ => unreachable!(),
        }
    }
    fn verify(
        renderer: &mut Renderer,
        image: &Arc<RgbaImage>,
        edit: &Edit,
        seg: Option<&Arc<Segmentation>>,
        crop: Crop,
    ) {
        let incremental = renderer
            .render_viewport(image, edit, seg, crop, None)
            .unwrap();
        let full = Renderer::default()
            .render_region(image, edit, seg, crop, None)
            .unwrap();
        // Crop-local filter accumulation and identity warp interpolation can
        // round one byte differently. This is the native renderer's existing
        // region tolerance; stale damage or a lossy preview is much larger.
        let max_difference = incremental
            .as_raw()
            .iter()
            .zip(full.as_raw())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(
            max_difference <= 1,
            "viewport must match full native pixels; damage {:?}",
            renderer.viewport_damage()
        );
    }
    #[test]
    fn growing_manual_strokes_redraw_only_their_footprints_without_changing_other_pixels() {
        let (image, crop) = fixture();
        for tool in [Target::Liquify, Target::Heal, Target::Clone, Target::Patch] {
            let mut renderer = Renderer::default();
            let mut edit = Edit::default();
            verify(&mut renderer, &image, &edit, None, crop);
            append(&mut edit, tool, [0.43, 0.45]);
            verify(&mut renderer, &image, &edit, None, crop);
            assert_ne!(
                renderer.viewport_damage(),
                Some(crop),
                "first brush contact must also be local"
            );
            for center in [[0.45, 0.46], [0.47, 0.46], [0.50, 0.48]] {
                append(&mut edit, tool, center);
                verify(&mut renderer, &image, &edit, None, crop);
                let dirty = renderer.viewport_damage().unwrap();
                assert!(
                    dirty.width * dirty.height < crop.width * crop.height / 8,
                    "{tool:?}: {dirty:?}"
                );
            }
        }
    }
    #[test]
    fn warped_heal_clone_and_moving_patch_donors_match_full_pixels_with_filters() {
        let (image, crop) = fixture();
        let seg = Arc::new(Segmentation {
            width: 16,
            height: 16,
            skin: vec![0.85; 256].into(),
            ..Default::default()
        });
        let mut renderer = Renderer::default();
        let mut edit = Edit::default();
        edit.settings.smoothing = 45.;
        edit.settings.sharpening = 24.;
        edit.settings.exposure = 0.1;
        append(&mut edit, Target::Liquify, [0.44, 0.46]);
        // A deliberately strong deformation checks projected source damage.
        edit.warps[0].delta = [0.13, -0.05];
        edit.warps[0].radius = 0.15;
        verify(&mut renderer, &image, &edit, Some(&seg), crop);
        for tool in [Target::Heal, Target::Clone, Target::Patch, Target::Heal] {
            append(&mut edit, tool, [0.43, 0.44]);
            verify(&mut renderer, &image, &edit, Some(&seg), crop);
            assert_ne!(renderer.viewport_damage(), Some(crop));
        }
        for offset in [[0.08, 0.07], [-0.08, 0.05], [0.12, -0.09]] {
            edit.patches.last_mut().unwrap().offset = offset;
            verify(&mut renderer, &image, &edit, Some(&seg), crop);
            assert_ne!(renderer.viewport_damage(), Some(crop));
        }
        // Replacing the draft boundary must also restore its old footprint.
        for p in &mut edit.patches.last_mut().unwrap().boundary {
            p[0] += 0.08;
        }
        verify(&mut renderer, &image, &edit, Some(&seg), crop);
    }
    #[test]
    fn structural_changes_undo_pan_source_and_segmentation_rebuild_the_viewport() {
        let (image, crop) = fixture();
        let mut renderer = Renderer::default();
        let mut edit = Edit::default();
        verify(&mut renderer, &image, &edit, None, crop);
        append(&mut edit, Target::Liquify, [0.43, 0.46]);
        verify(&mut renderer, &image, &edit, None, crop);
        assert_ne!(renderer.viewport_damage(), Some(crop));
        append(&mut edit, Target::Liquify, [0.43, 0.46]);
        edit.warps.last_mut().unwrap().radius = 0.009;
        verify(&mut renderer, &image, &edit, None, crop);
        assert_eq!(renderer.viewport_damage(), Some(crop));
        edit.warps.pop();
        verify(&mut renderer, &image, &edit, None, crop);
        assert_eq!(renderer.viewport_damage(), Some(crop));
        edit.settings.exposure = 0.4;
        verify(&mut renderer, &image, &edit, None, crop);
        assert_eq!(renderer.viewport_damage(), Some(crop));
        let panned = Crop {
            x: crop.x + 17,
            ..crop
        };
        verify(&mut renderer, &image, &edit, None, panned);
        assert_eq!(renderer.viewport_damage(), Some(panned));
        let other = Arc::new(image.as_ref().clone());
        verify(&mut renderer, &other, &edit, None, panned);
        assert_eq!(renderer.viewport_damage(), Some(panned));
        let seg = Arc::new(Segmentation::default());
        verify(&mut renderer, &other, &edit, Some(&seg), panned);
        assert_eq!(renderer.viewport_damage(), Some(panned));
        append(&mut edit, Target::Clone, [0., 0.]);
        verify(&mut renderer, &other, &edit, Some(&seg), panned);
        assert_eq!(
            renderer.viewport_damage(),
            None,
            "offscreen edits need no redraw"
        );
        let cancel = AtomicBool::new(true);
        assert!(
            renderer
                .render_viewport(&other, &edit, Some(&seg), panned, Some(&cancel))
                .is_err()
        );
        verify(&mut renderer, &other, &edit, Some(&seg), panned);
    }
    #[test]
    fn top_layer_brushes_are_incremental_and_lower_layer_edits_use_full_dependencies() {
        let (image, crop) = fixture();
        let mut renderer = Renderer::default();
        let mut edit = Edit::default();
        edit.settings.exposure = 0.1;
        edit.ensure_stack();
        edit.add_layer(LayerType::Retouch);
        edit.stack.layers.last_mut().unwrap().opacity = 67.;
        verify(&mut renderer, &image, &edit, None, crop);
        append(&mut edit, Target::Clone, [0.43, 0.46]);
        verify(&mut renderer, &image, &edit, None, crop);
        assert_ne!(renderer.viewport_damage(), Some(crop));
        edit.select_layer(1);
        append(&mut edit, Target::Heal, [0.44, 0.48]);
        verify(&mut renderer, &image, &edit, None, crop);
        assert_eq!(renderer.viewport_damage(), Some(crop));
        edit.stack.layers.last_mut().unwrap().opacity = 32.;
        verify(&mut renderer, &image, &edit, None, crop);
        assert_eq!(renderer.viewport_damage(), Some(crop));
    }
}
