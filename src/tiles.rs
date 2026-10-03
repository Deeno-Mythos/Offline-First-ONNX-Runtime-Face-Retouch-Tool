//! Bounded native-resolution processing with global image coordinates and filter halos.
use super::*;
use std::borrow::Cow;
use std::cell::RefCell;

const EDGE: u32 = 512;

fn whole(image: &RgbaImage) -> Crop {
    Crop {
        x: 0,
        y: 0,
        width: image.width(),
        height: image.height(),
    }
}
fn pad(crop: Crop, dims: (u32, u32), radius: u32) -> Crop {
    let x = crop.x.saturating_sub(radius);
    let y = crop.y.saturating_sub(radius);
    Crop {
        x,
        y,
        width: (crop.x + crop.width + radius).min(dims.0) - x,
        height: (crop.y + crop.height + radius).min(dims.1) - y,
    }
}
fn linear_region(image: &RgbaImage, crop: Crop) -> Vec<[f32; 4]> {
    (0..crop.width * crop.height)
        .into_par_iter()
        .map(|i| {
            let p = image.get_pixel(crop.x + i % crop.width, crop.y + i / crop.width);
            [
                linear_byte(p[0]),
                linear_byte(p[1]),
                linear_byte(p[2]),
                p[3] as f32 / 255.0,
            ]
        })
        .collect()
}

/// Small LRU for globally registered blur samples, including remote healing donors.
type BlurTiles = RefCell<VecDeque<(usize, Crop, Vec<[f16; 4]>)>>;
struct BlurSampler<'a> {
    source: &'a RgbaImage,
    radius: usize,
    tiles: Cow<'a, BlurTiles>,
}
impl<'a> BlurSampler<'a> {
    fn new(source: &'a RgbaImage, radius: usize) -> Self {
        Self::new_cached(source, radius, None)
    }
    fn new_cached(source: &'a RgbaImage, radius: usize, tiles: Option<&'a BlurTiles>) -> Self {
        Self {
            source,
            radius,
            tiles: tiles.map_or_else(|| Cow::Owned(BlurTiles::default()), Cow::Borrowed),
        }
    }
    fn at(&self, x: u32, y: u32) -> [f32; 4] {
        let x = x.min(self.source.width() - 1);
        let y = y.min(self.source.height() - 1);
        let mut tiles = self.tiles.as_ref().borrow_mut();
        let hit = tiles.iter().position(|(radius, crop, _)| {
            *radius == self.radius
                && x >= crop.x
                && y >= crop.y
                && x < crop.x + crop.width
                && y < crop.y + crop.height
        });
        let (crop, data) = if let Some(i) = hit {
            let (_, crop, data) = tiles.remove(i).unwrap();
            (crop, data)
        } else {
            let crop = Crop {
                x: x / EDGE * EDGE,
                y: y / EDGE * EDGE,
                width: EDGE.min(self.source.width() - x / EDGE * EDGE),
                height: EDGE.min(self.source.height() - y / EDGE * EDGE),
            };
            let padded = pad(crop, self.source.dimensions(), self.radius as u32);
            let linear = linear_region(self.source, padded);
            let filtered = blur(
                &linear,
                padded.width as usize,
                padded.height as usize,
                self.radius,
            );
            let data = (0..crop.width * crop.height)
                .map(|i| {
                    filtered[((crop.y - padded.y + i / crop.width) * padded.width + crop.x
                        - padded.x
                        + i % crop.width) as usize]
                })
                .collect();
            (crop, data)
        };
        let value = data[((y - crop.y) * crop.width + x - crop.x) as usize].map(f16::to_f32);
        tiles.push_front((self.radius, crop, data));
        // At most 16 MiB of immutable donor pixels per native source. Reuse
        // across stamps and pans; radius is part of each entry's identity.
        tiles.truncate(8);
        value
    }
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let x = x.clamp(0.0, (self.source.width() - 1) as f32);
        let y = y.clamp(0.0, (self.source.height() - 1) as f32);
        let (ix, iy) = (x.floor() as u32, y.floor() as u32);
        let (tx, ty) = (x - ix as f32, y - iy as f32);
        let p = [
            self.at(ix, iy),
            self.at(ix + 1, iy),
            self.at(ix, iy + 1),
            self.at(ix + 1, iy + 1),
        ];
        std::array::from_fn(|c| {
            (p[0][c] * (1.0 - tx) + p[1][c] * tx) * (1.0 - ty)
                + (p[2][c] * (1.0 - tx) + p[3][c] * tx) * ty
        })
    }
}

#[cfg(feature = "onnx")]
fn references(
    image: &RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    radius: usize,
) -> Vec<UnderEyeReference> {
    let Some(seg) = seg.filter(|s| !s.faces.is_empty()) else {
        return vec![];
    };
    let (w, h) = image.dimensions();
    let sampler = BlurSampler::new(image, radius);
    seg.faces
        .iter()
        .flat_map(|face| crate::model::portrait::eye_regions(face, w, h))
        .map(|eye| {
            let mut sum = 0.0;
            let mut count = 0;
            for below in [0.78, 0.98, 1.18] {
                for across in [-0.2, 0.0, 0.2] {
                    let x = eye.lower[0]
                        + eye.down[0] * eye.width * below
                        + eye.axis[0] * eye.width * across;
                    let y = eye.lower[1]
                        + eye.down[1] * eye.width * below
                        + eye.axis[1] * eye.width * across;
                    if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
                        continue;
                    }
                    let px = (x.round() as u32).min(w - 1);
                    let py = (y.round() as u32).min(h - 1);
                    if !seg.skin.is_empty() && sample_mask(&seg.skin, seg, px, py, w, h) < 0.25 {
                        continue;
                    }
                    let masks = local_layers_region(
                        image,
                        edit,
                        Some(seg),
                        [false, false, false, true, false],
                        Crop {
                            x: px,
                            y: py,
                            width: 1,
                            height: 1,
                        },
                    );
                    if masks.value(0, 3) > 0.18 {
                        continue;
                    }
                    sum += lum(&sampler.at(px, py));
                    count += 1;
                }
            }
            let luma = if count == 0 {
                let x = (eye.lower[0] + eye.down[0] * eye.width).round() as u32;
                let y = (eye.lower[1] + eye.down[1] * eye.width).round() as u32;
                lum(&sampler.at(x.min(w - 1), y.min(h - 1)))
            } else {
                sum / count as f32
            };
            UnderEyeReference { eye, luma }
        })
        .collect()
}

#[cfg(feature = "onnx")]
fn shadow_lift(
    low: &[[f16; 4]],
    masks: &LocalMasks,
    crop: Crop,
    references: &[UnderEyeReference],
) -> Vec<f32> {
    if references.is_empty() {
        return vec![0.0; masks.pixels()];
    }
    (0..masks.pixels())
        .into_par_iter()
        .map(|i| {
            let region = masks.value(i, 3);
            if region <= 0.0 {
                return 0.0;
            }
            let p = [
                (crop.x + i as u32 % crop.width) as f32 + 0.5,
                (crop.y + i as u32 / crop.width) as f32 + 0.5,
            ];
            let reference = references
                .iter()
                .min_by(|a, b| {
                    let d = |r: &UnderEyeReference| {
                        (p[0] - r.eye.lower[0]).powi(2) + (p[1] - r.eye.lower[1]).powi(2)
                    };
                    d(a).total_cmp(&d(b))
                })
                .unwrap();
            (reference.luma - lum_half(&low[i]) - 0.012).clamp(0.0, 0.12) * region * 0.9
        })
        .collect()
}

struct Prepared<'a> {
    image: &'a RgbaImage,
    edit: &'a Edit,
    seg: Option<&'a Segmentation>,
    s: Settings,
    color: crate::color::PreparedColor,
    flyaway: Option<Arc<crate::flyaway::FlyawayMap>>,
    cleanup: Option<Arc<crate::cleanup::PreparedCleanup>>,
    #[cfg(feature = "onnx")]
    eyes: Arc<Vec<UnderEyeReference>>,
}

/// Prepared corrections are reused when panning; image pixels remain in bounded tiles.
#[derive(Default)]
pub(super) struct NativeCache {
    warp: crate::geometry::WarpCache,
    segmentation: Option<Arc<Segmentation>>,
    flyaway: Option<Arc<crate::flyaway::FlyawayMap>>,
    cleanup_key: Option<Edit>,
    cleanup: Option<Arc<crate::cleanup::PreparedCleanup>>,
    donor_blur: BlurTiles,
    #[cfg(feature = "onnx")]
    eyes: Option<(SharedVec<Stroke>, Arc<Vec<UnderEyeReference>>)>,
}
impl NativeCache {
    pub(super) fn bytes(&self) -> usize {
        self.flyaway.as_ref().map_or(0, |m| m.bytes())
            + self.warp.bytes()
            + self.cleanup.as_ref().map_or(0, |m| m.bytes())
            + self.segmentation.as_ref().map_or(0, |s| s.map_bytes())
            + self
                .donor_blur
                .borrow()
                .iter()
                .map(|(_, _, pixels)| pixels.len() * std::mem::size_of::<[f16; 4]>())
                .sum::<usize>()
    }
}
impl<'a> Prepared<'a> {
    fn new(
        image: &'a RgbaImage,
        edit: &'a Edit,
        seg: Option<&'a Segmentation>,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self> {
        Self::new_cached(image, edit, seg, cancel, None, None)
    }
    fn new_cached(
        image: &'a RgbaImage,
        edit: &'a Edit,
        seg: Option<&'a Segmentation>,
        cancel: Option<&AtomicBool>,
        mut cache: Option<&mut NativeCache>,
        shared_seg: Option<&Arc<Segmentation>>,
    ) -> Result<Self> {
        check_cancel(cancel)?;
        let s = edit.settings.effective();
        let radius = (image.width().min(image.height()) as f32 * 0.003)
            .round()
            .max(1.0) as usize;
        if let Some(c) = cache.as_mut() {
            let same = match (&c.segmentation, shared_seg) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            };
            if !same {
                c.segmentation = shared_seg.cloned();
                c.flyaway = None;
                #[cfg(feature = "onnx")]
                {
                    c.eyes = None;
                }
            }
        }
        let make_cleanup = |tiles: Option<&BlurTiles>| {
            let sampler = BlurSampler::new_cached(image, radius, tiles);
            crate::cleanup::PreparedCleanup::prepare_cancellable(
                image,
                edit,
                |x, y| sampler.sample(x, y),
                cancel,
            )
            .map(|p| p.map(Arc::new))
        };
        let cleanup = if let Some(c) = cache.as_mut() {
            let same = c.cleanup_key.as_ref().is_some_and(|k| {
                (k.strokes.same_storage(&edit.strokes) || k.strokes == edit.strokes)
                    && (k.clones.same_storage(&edit.clones) || k.clones == edit.clones)
                    && (k.patches.same_storage(&edit.patches) || k.patches == edit.patches)
                    && k.settings.effective().healing == s.healing
            });
            if !same {
                let suffix = c.cleanup_key.as_ref().and_then(|key| {
                    crate::cleanup::appended_edit(
                        &key.strokes,
                        &key.clones,
                        &key.patches,
                        key.settings.effective().healing,
                        edit,
                    )
                });
                if let (Some(suffix), Some(previous)) = (suffix, c.cleanup.as_mut()) {
                    let sampler = BlurSampler::new_cached(image, radius, Some(&c.donor_blur));
                    if let Some(newer) = crate::cleanup::PreparedCleanup::prepare_cancellable(
                        image,
                        &suffix,
                        |x, y| sampler.sample(x, y),
                        cancel,
                    )? {
                        Arc::make_mut(previous).append(newer);
                    }
                } else {
                    c.cleanup = make_cleanup(Some(&c.donor_blur))?;
                }
                c.cleanup_key = Some(edit.clone());
            }
            c.cleanup.clone()
        } else {
            make_cleanup(None)?
        };
        check_cancel(cancel)?;
        let color = crate::color::PreparedColor::new(&s.color, image);
        let flyaway = if s.flyaway_hairs != 0.0 {
            if let Some(c) = cache.as_mut() {
                if c.flyaway.is_none() {
                    c.flyaway = seg
                        .map(|seg| crate::flyaway::detect(image, seg, cancel).map(Arc::new))
                        .transpose()?;
                }
                c.flyaway.clone()
            } else {
                seg.map(|seg| crate::flyaway::detect(image, seg, cancel).map(Arc::new))
                    .transpose()?
            }
        } else {
            None
        };
        #[cfg(feature = "onnx")]
        let eyes = if s.under_eyes != 0.0 {
            if let Some(c) = cache.as_mut() {
                let same = c.eyes.as_ref().is_some_and(|(strokes, _)| {
                    strokes.same_storage(&edit.strokes) || *strokes == edit.strokes
                });
                if !same {
                    c.eyes = Some((
                        edit.strokes.clone(),
                        Arc::new(references(image, edit, seg, radius * 4)),
                    ));
                }
                c.eyes.as_ref().unwrap().1.clone()
            } else {
                Arc::new(references(image, edit, seg, radius * 4))
            }
        } else {
            Arc::new(vec![])
        };
        Ok(Self {
            image,
            edit,
            seg,
            s,
            color,
            flyaway,
            cleanup,
            #[cfg(feature = "onnx")]
            eyes,
        })
    }
    fn tile(&self, core: Crop, cancel: Option<&AtomicBool>) -> Result<RgbaImage> {
        check_cancel(cancel)?;
        let (w, h) = self.image.dimensions();
        let s = &self.s;
        let radius = (w.min(h) as f32 * 0.003).round().max(1.0) as usize;
        let texture = s.smoothing != 0.0
            || s.tone_evenness != 0.0
            || s.under_eyes != 0.0
            || s.forehead != 0.0
            || s.laugh_lines != 0.0;
        let fine_needed = texture || s.eyes != 0.0 || s.sharpening != 0.0;
        let background_radius = if s.background == Background::Blur {
            (w.min(h) as f32 * s.background_blur / 3000.0)
                .round()
                .max(1.0) as usize
        } else {
            0
        };
        let halo = background_radius.max(if texture {
            radius * 4
        } else if fine_needed {
            radius
        } else {
            0
        });
        let region = pad(core, (w, h), halo as u32);
        let linear = linear_region(self.image, region);
        let fine = fine_needed.then(|| {
            blur(
                &linear,
                region.width as usize,
                region.height as usize,
                radius,
            )
        });
        check_cancel(cancel)?;
        let low = texture.then(|| {
            blur(
                &linear,
                region.width as usize,
                region.height as usize,
                radius * 4,
            )
        });
        check_cancel(cancel)?;
        let background = (background_radius > 0).then(|| {
            blur(
                &linear,
                region.width as usize,
                region.height as usize,
                background_radius,
            )
        });
        check_cancel(cancel)?;
        let targets = [
            s.smoothing != 0.0 || s.tone_evenness != 0.0 || s.redness != 0.0,
            s.teeth != 0.0,
            s.eyes != 0.0,
            s.under_eyes != 0.0,
            s.background != Background::Original,
        ];
        let masks = local_layers_region(self.image, self.edit, self.seg, targets, region);
        let lift = if s.under_eyes != 0.0 {
            #[cfg(feature = "onnx")]
            {
                Some(shadow_lift(
                    low.as_deref().unwrap(),
                    &masks,
                    region,
                    &self.eyes,
                ))
            }
            #[cfg(not(feature = "onnx"))]
            {
                Some(vec![0.0; masks.pixels()])
            }
        } else {
            None
        };
        let healed = self
            .cleanup
            .as_ref()
            .and_then(|p| p.apply_region(self.image, &linear, region));
        check_cancel(cancel)?;
        let output = render_pixels(PixelPass {
            w,
            h,
            region,
            s,
            color: &self.color,
            texture,
            linear: &linear,
            fine: fine.as_deref(),
            low: low.as_deref(),
            background: background.as_deref(),
            masks: &masks,
            under_eye_lift: lift.as_deref(),
            healed: healed.as_deref(),
            flyaway: self.flyaway.as_deref(),
            seg: self.seg,
            cancel,
        });
        check_cancel(cancel)?;
        let padded = RgbaImage::from_raw(region.width, region.height, output).unwrap();
        Ok(image::imageops::crop_imm(
            &padded,
            core.x - region.x,
            core.y - region.y,
            core.width,
            core.height,
        )
        .to_image())
    }
    fn unwarped(&self, crop: Crop, cancel: Option<&AtomicBool>) -> Result<RgbaImage> {
        let mut output = RgbaImage::new(crop.width, crop.height);
        for y in (crop.y..crop.y + crop.height).step_by(EDGE as usize) {
            for x in (crop.x..crop.x + crop.width).step_by(EDGE as usize) {
                check_cancel(cancel)?;
                let tile = Crop {
                    x,
                    y,
                    width: EDGE.min(crop.x + crop.width - x),
                    height: EDGE.min(crop.y + crop.height - y),
                };
                let rendered = self.tile(tile, cancel)?;
                // Copy rows without allocating a second full-size float or byte image.
                for row in 0..tile.height {
                    let start = (((y - crop.y + row) * crop.width + x - crop.x) * 4) as usize;
                    let from = (row * tile.width * 4) as usize;
                    output.as_mut()[start..start + tile.width as usize * 4]
                        .copy_from_slice(&rendered.as_raw()[from..from + tile.width as usize * 4]);
                }
            }
        }
        Ok(output)
    }
}

pub(super) fn render(
    image: &RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    cancel: Option<&AtomicBool>,
) -> Result<RgbaImage> {
    let effective = edit.effective_layers();
    let edit = effective.as_ref();
    if image.width() == 0 || image.height() == 0 {
        return Ok(image.clone());
    }
    let prepared = Prepared::new(image, edit, seg, cancel)?;
    let output = prepared.unwarped(whole(image), cancel)?;
    check_cancel(cancel)?;
    let output = crate::geometry::warp(output, edit, seg);
    check_cancel(cancel)?;
    Ok(output)
}

pub(super) fn render_region(
    image: &RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    crop: Crop,
    cancel: Option<&AtomicBool>,
) -> Result<RgbaImage> {
    render_region_cached(image, edit, seg, crop, cancel, None, None)
}
pub(super) fn render_region_cached(
    image: &RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    crop: Crop,
    cancel: Option<&AtomicBool>,
    mut cache: Option<&mut NativeCache>,
    shared_seg: Option<&Arc<Segmentation>>,
) -> Result<RgbaImage> {
    anyhow::ensure!(
        crop.width > 0
            && crop.height > 0
            && crop
                .x
                .checked_add(crop.width)
                .is_some_and(|r| r <= image.width())
            && crop
                .y
                .checked_add(crop.height)
                .is_some_and(|b| b <= image.height()),
        "Invalid native render region"
    );
    let effective = edit.effective_layers();
    let edit = effective.as_ref();
    let prepared =
        Prepared::new_cached(image, edit, seg, cancel, cache.as_deref_mut(), shared_seg)?;
    if !crate::geometry::active(edit, seg) {
        return prepared.unwarped(crop, cancel);
    }
    // The same deformation grid is used for viewport renders and complete exports.
    let field = if let Some(cache) = cache {
        cache.warp.displacement(edit, seg, image.dimensions())
    } else {
        crate::geometry::DisplacementField::new(edit, seg, image.dimensions())
    };
    let mut min = [image.width() - 1, image.height() - 1];
    let mut max = [0; 2];
    let mut coordinates = Vec::with_capacity((crop.width * crop.height) as usize);
    for y in crop.y..crop.y + crop.height {
        check_cancel(cancel)?;
        for x in crop.x..crop.x + crop.width {
            let uv = field.source([
                x as f32 / image.width() as f32,
                y as f32 / image.height() as f32,
            ]);
            let p = [
                (uv[0] * image.width() as f32).clamp(0.0, (image.width() - 1) as f32),
                (uv[1] * image.height() as f32).clamp(0.0, (image.height() - 1) as f32),
            ];
            for c in 0..2 {
                min[c] = min[c].min(p[c].floor() as u32);
                max[c] = max[c].max(p[c].ceil() as u32);
            }
            coordinates.push(p);
        }
    }
    let needed = Crop {
        x: min[0],
        y: min[1],
        width: max[0] - min[0] + 1,
        height: max[1] - min[1] + 1,
    };
    let source = prepared.unwarped(needed, cancel)?;
    let mut output = RgbaImage::new(crop.width, crop.height);
    output
        .as_mut()
        .par_chunks_mut(4)
        .zip(coordinates.par_iter())
        .for_each(|(out, p)| {
            let color = crate::geometry::sample_rgba(
                &source,
                p[0] - needed.x as f32,
                p[1] - needed.y as f32,
            );
            for c in 0..4 {
                out[c] = color[c].round().clamp(0.0, 255.0) as u8;
            }
        });
    check_cancel(cancel)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (RgbaImage, Segmentation, Edit) {
        let (w, h) = (1051, 777);
        let image = RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([
                ((x * 3 + y) % 180 + 20) as u8,
                ((x + y * 2) % 150 + 30) as u8,
                ((x * 2 + y * 3) % 140 + 40) as u8,
                if (x + y) % 47 == 0 { 170 } else { 255 },
            ])
        });
        let (sw, sh) = (107, 83);
        let n = (sw * sh) as usize;
        let mut seg = Segmentation {
            width: sw,
            height: sh,
            skin: vec![0.8; n].into(),
            teeth: vec![0.2; n].into(),
            eyes: vec![0.4; n].into(),
            under_eyes: vec![0.3; n].into(),
            forehead: vec![0.5; n].into(),
            laugh_lines: vec![0.4; n].into(),
            contour: vec![0.2; n].into(),
            highlight: vec![0.4; n].into(),
            background: (0..n)
                .map(|i| (i % sw as usize) as f32 / sw as f32)
                .collect(),
            neural_blend: vec![[0.52, 0.51, 0.53]; n].into(),
            repair_delta: vec![[0.01, -0.02, 0.015]; n].into(),
            ..Default::default()
        };
        let mut landmarks = vec![[0.5, 0.5, 0.0]; 478];
        for (i, p) in [
            (33, [0.34, 0.4]),
            (133, [0.44, 0.4]),
            (145, [0.39, 0.44]),
            (362, [0.56, 0.4]),
            (263, [0.66, 0.4]),
            (374, [0.61, 0.44]),
            (152, [0.5, 0.82]),
        ] {
            landmarks[i] = [p[0], p[1], 0.0];
        }
        seg.faces.push(crate::geometry::FaceMesh {
            landmarks,
            bounds: [0.2, 0.2, 0.6, 0.62],
            confidence: 0.9,
        });
        let mut edit = Edit::default();
        let s = &mut edit.settings;
        s.smoothing = 60.0;
        s.blemishes = 45.0;
        s.tone_evenness = 25.0;
        s.redness = 20.0;
        s.teeth = 30.0;
        s.eyes = 40.0;
        s.under_eyes = 70.0;
        s.forehead = 30.0;
        s.laugh_lines = 15.0;
        s.contour = 35.0;
        s.face_highlight = 25.0;
        s.exposure = 0.15;
        s.shadows = 20.0;
        s.highlights = -20.0;
        s.contrast = 10.0;
        s.warmth = 15.0;
        s.tint = -10.0;
        s.saturation = 12.0;
        s.sharpening = 18.0;
        s.vignette = 20.0;
        s.color.hsl[1].hue = 15.0;
        s.color.hsl[0].saturation = -20.0;
        s.color.curves[0][2] = 0.18;
        s.color.reference = Some(
            crate::color::ReferenceProfile::from_image(
                "ref".into(),
                &RgbaImage::from_fn(99, 101, |x, y| {
                    image::Rgba([(x + 40) as u8, (y + 50) as u8, 90, 255])
                }),
            )
            .unwrap(),
        );
        s.color.reference_strength = 20.0;
        edit.strokes.push(Stroke {
            target: Target::Skin,
            center: [0.48, 0.65],
            radius: 0.1,
            erase: true,
            strength: 100.0,
            softness: 0.75,
        });
        edit.strokes.push(Stroke {
            target: Target::Heal,
            center: [0.5, 0.6],
            radius: 0.01,
            erase: false,
            strength: 100.0,
            softness: 0.8,
        });
        edit.clones.push(crate::cleanup::CloneStamp {
            center: [0.48, 0.66],
            source: [0.85, 0.1],
            radius: 0.08,
            softness: 0.65,
            strength: 60.0,
        });
        edit.patches.push(crate::cleanup::PatchStroke {
            boundary: vec![[0.46, 0.61], [0.56, 0.61], [0.56, 0.75], [0.46, 0.75]],
            offset: [-0.25, -0.5],
            softness: 0.7,
            strength: 70.0,
        });
        (image, seg, edit)
    }
    fn close(a: &RgbaImage, b: &RgbaImage) {
        assert_eq!(a.dimensions(), b.dimensions());
        let diffs: Vec<_> = a
            .as_raw()
            .iter()
            .zip(b.as_raw())
            .map(|(a, b)| a.abs_diff(*b))
            .collect();
        let max = *diffs.iter().max().unwrap();
        let mean = diffs.iter().map(|d| *d as f64).sum::<f64>() / diffs.len() as f64;
        assert!(max <= 1 && mean < 0.05, "max difference {max}, mean {mean}");
    }
    #[test]
    fn layered_full_and_native_renders_match_and_native_opacity_reuses_cleanup() {
        use crate::layers::LayerKind;
        let (image, seg, mut edit) = fixture();
        edit.settings.background = Background::Solid;
        edit.warps.push(crate::geometry::WarpStroke {
            center: [0.52, 0.64],
            delta: [0.012, -0.015],
            radius: 0.12,
            softness: 0.7,
            strength: 80.0,
        });
        for (kind, opacity) in [
            (LayerKind::Portrait, 75.0),
            (LayerKind::Color, 60.0),
            (LayerKind::Background, 40.0),
            (LayerKind::Liquify, 35.0),
            (LayerKind::SpotHeal, 50.0),
            (LayerKind::CloneStamp, 25.0),
            (LayerKind::Patch, 65.0),
        ] {
            edit.layers.get_mut(kind).opacity = opacity;
        }
        let effective = edit.effective_layers();
        let reference = render_inner(&image, &effective, Some(&seg), None, None, None).unwrap();
        close(
            &render(&image, &edit, Some(&seg), None).unwrap(),
            &reference,
        );
        let crop = Crop {
            x: 489,
            y: 472,
            width: 67,
            height: 103,
        };
        let expected =
            image::imageops::crop_imm(&reference, crop.x, crop.y, crop.width, crop.height)
                .to_image();
        close(
            &render_region(&image, &edit, Some(&seg), crop, None).unwrap(),
            &expected,
        );
        let seg = Arc::new(seg);
        let mut cache = NativeCache::default();
        render_region_cached(
            &image,
            &edit,
            Some(&seg),
            crop,
            None,
            Some(&mut cache),
            Some(&seg),
        )
        .unwrap();
        let cleanup = cache.cleanup.as_ref().unwrap().clone();
        render_region_cached(
            &image,
            &edit,
            Some(&seg),
            crop,
            None,
            Some(&mut cache),
            Some(&seg),
        )
        .unwrap();
        assert!(Arc::ptr_eq(&cleanup, cache.cleanup.as_ref().unwrap()));
        edit.layers.get_mut(LayerKind::CloneStamp).visible = false;
        render_region_cached(
            &image,
            &edit,
            Some(&seg),
            crop,
            None,
            Some(&mut cache),
            Some(&seg),
        )
        .unwrap();
        assert!(!Arc::ptr_eq(&cleanup, cache.cleanup.as_ref().unwrap()));
        for kind in LayerKind::ALL {
            edit.layers.get_mut(kind).visible = false;
        }
        let hidden = render_region_cached(
            &image,
            &edit,
            Some(&seg),
            crop,
            None,
            Some(&mut cache),
            Some(&seg),
        )
        .unwrap();
        assert_eq!(
            hidden,
            image::imageops::crop_imm(&image, crop.x, crop.y, crop.width, crop.height).to_image()
        );
        assert_eq!(edit.warps[0].strength, 80.0);
    }
    #[test]
    fn tiled_all_adjustments_masks_remote_donors_and_background_have_no_seams() {
        let (image, seg, mut edit) = fixture();
        for background in [
            Background::Original,
            Background::Blur,
            Background::Solid,
            Background::Transparent,
        ] {
            edit.settings.background = background;
            edit.settings.background_blur = 70.0;
            let reference = render_inner(&image, &edit, Some(&seg), None, None, None).unwrap();
            let tiled = render(&image, &edit, Some(&seg), None).unwrap();
            close(&reference, &tiled);
            let crop = Crop {
                x: 497,
                y: 504,
                width: 119,
                height: 188,
            };
            let native = render_region(&image, &edit, Some(&seg), crop, None).unwrap();
            close(
                &image::imageops::crop_imm(&reference, crop.x, crop.y, crop.width, crop.height)
                    .to_image(),
                &native,
            );
        }
    }
    #[test]
    fn warped_native_region_matches_complete_export_and_validates_bounds() {
        let (image, seg, mut edit) = fixture();
        edit.warps.push(crate::geometry::WarpStroke {
            center: [0.5, 0.63],
            delta: [0.18, -0.06],
            radius: 0.24,
            softness: 0.8,
            strength: 100.0,
        });
        let crop = Crop {
            x: 481,
            y: 448,
            width: 156,
            height: 153,
        };
        let reference = render(&image, &edit, Some(&seg), None).unwrap();
        let native = render_region(&image, &edit, Some(&seg), crop, None).unwrap();
        close(
            &image::imageops::crop_imm(&reference, crop.x, crop.y, crop.width, crop.height)
                .to_image(),
            &native,
        );
        assert!(
            render_region(
                &image,
                &edit,
                None,
                Crop {
                    x: u32::MAX,
                    ..crop
                },
                None
            )
            .is_err()
        );
        assert!(
            render_region(&image, &edit, None, crop, Some(&AtomicBool::new(true)))
                .unwrap_err()
                .is::<Cancelled>()
        );
    }
    #[test]
    fn neutral_large_render_is_byte_identical_and_under_eye_does_not_darken_unmasked_hair() {
        let image = RgbaImage::from_fn(1511, 1503, |x, y| {
            image::Rgba([
                (x % 256) as u8,
                (y % 256) as u8,
                ((x + y) % 256) as u8,
                ((x * 3 + y) % 256) as u8,
            ])
        });
        assert_eq!(
            render_cancellable(&image, &Edit::default(), None, None).unwrap(),
            image
        );
        let small = RgbaImage::from_fn(16, 16, |x, y| {
            image::Rgba([
                if x < 4 { 1 } else { 120 },
                if x < 4 { 2 } else { 90 },
                if x < 4 { 3 } else { 80 },
                if y < 3 { 150 } else { 255 },
            ])
        });
        let seg = Segmentation {
            width: 16,
            height: 16,
            under_eyes: (0..256)
                .map(|i| if i % 16 > 9 { 1.0 } else { 0.0 })
                .collect(),
            ..Default::default()
        };
        let mut edit = Edit::default();
        edit.settings.under_eyes = 100.0;
        for output in [
            render_inner(&small, &edit, Some(&seg), None, None, None).unwrap(),
            render(&small, &edit, Some(&seg), None).unwrap(),
        ] {
            for y in 0..16 {
                for x in 0..4 {
                    assert_eq!(output.get_pixel(x, y), small.get_pixel(x, y));
                }
            }
        }
    }
    #[test]
    fn native_preparation_reuses_corrections_and_invalidates_changed_sources_masks_and_tools() {
        let (image, seg, mut edit) = fixture();
        let image = Arc::new(image);
        let seg = Arc::new(seg);
        let crop = Crop {
            x: 489,
            y: 472,
            width: 67,
            height: 103,
        };
        let mut renderer = Renderer::default();
        renderer
            .render_region(&image, &edit, Some(&seg), crop, None)
            .unwrap();
        let cleanup = renderer.native_sources[0]
            .1
            .cleanup
            .as_ref()
            .unwrap()
            .clone();
        #[cfg(feature = "onnx")]
        let eyes = renderer.native_sources[0]
            .1
            .eyes
            .as_ref()
            .unwrap()
            .1
            .clone();
        edit.settings.exposure += 0.2;
        let actual = renderer
            .render_region(&image, &edit, Some(&seg), crop, None)
            .unwrap();
        assert!(Arc::ptr_eq(
            &cleanup,
            renderer.native_sources[0].1.cleanup.as_ref().unwrap()
        ));
        close(
            &actual,
            &render_region(&image, &edit, Some(&seg), crop, None).unwrap(),
        );
        edit.clones.push(crate::cleanup::CloneStamp {
            center: [0.49, 0.66],
            source: [0.1, 0.1],
            radius: 0.05,
            softness: 0.5,
            strength: 100.0,
        });
        renderer
            .render_region(&image, &edit, Some(&seg), crop, None)
            .unwrap();
        assert!(!Arc::ptr_eq(
            &cleanup,
            renderer.native_sources[0].1.cleanup.as_ref().unwrap()
        ));
        let mut changed = (*seg).clone();
        changed.under_eyes = vec![0.0; (changed.width * changed.height) as usize].into();
        let changed = Arc::new(changed);
        let actual = renderer
            .render_region(&image, &edit, Some(&changed), crop, None)
            .unwrap();
        #[cfg(feature = "onnx")]
        assert!(!Arc::ptr_eq(
            &eyes,
            &renderer.native_sources[0].1.eyes.as_ref().unwrap().1
        ));
        close(
            &actual,
            &render_region(&image, &edit, Some(&changed), crop, None).unwrap(),
        );
        let other = Arc::new(RgbaImage::from_pixel(
            1051,
            777,
            image::Rgba([10, 20, 30, 255]),
        ));
        let actual = renderer
            .render_region(&other, &edit, Some(&seg), crop, None)
            .unwrap();
        close(
            &actual,
            &render_region(&other, &edit, Some(&seg), crop, None).unwrap(),
        );
        assert_eq!(renderer.native_sources.len(), 2);
    }

    #[test]
    fn donor_filter_cache_reuses_pixels_separates_radii_and_stays_bounded() {
        let image = RgbaImage::from_fn(1600, 1600, |x, y| {
            image::Rgba([(x % 251) as u8, (y % 251) as u8, 90, 255])
        });
        let cache = BlurTiles::default();
        let first = BlurSampler::new_cached(&image, 3, Some(&cache)).sample(65.3, 72.8);
        let allocation = cache.borrow()[0].2.as_ptr();
        assert_eq!(
            first,
            BlurSampler::new_cached(&image, 3, Some(&cache)).sample(65.3, 72.8)
        );
        assert_eq!(allocation, cache.borrow()[0].2.as_ptr());
        let larger = BlurSampler::new_cached(&image, 9, Some(&cache)).sample(65.3, 72.8);
        assert_eq!(larger, BlurSampler::new(&image, 9).sample(65.3, 72.8));
        assert_eq!(cache.borrow().len(), 2);
        for y in [70., 600., 1100.] {
            for x in [70., 600., 1100., 1590.] {
                let actual = BlurSampler::new_cached(&image, 3, Some(&cache)).sample(x, y);
                assert_eq!(actual, BlurSampler::new(&image, 3).sample(x, y));
            }
        }
        let retained = cache.borrow();
        assert_eq!(retained.len(), 8);
        assert!(retained.iter().map(|(_, _, p)| p.len() * 8).sum::<usize>() <= 16 * 1024 * 1024);
    }
}
