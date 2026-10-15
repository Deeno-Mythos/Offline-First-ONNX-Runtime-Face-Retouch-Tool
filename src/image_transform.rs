//! Document geometry is separate from retouch layers and is always rebuilt from the source.
use crate::engine::{Edit, Photo, Segmentation};
use anyhow::{Result, ensure};
use image::imageops::{self, FilterType};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageTransform {
    pub quarter_turns: u8,
    pub size: Option<[u32; 2]>,
    /// Clockwise rotation after the lossless quarter turns and proportional resize.
    pub angle: f32,
    /// An aperture in the upright working canvas. Its angle never rotates photo pixels.
    pub crop: Option<CropFrame>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CropFrame {
    pub center: [f32; 2],
    pub extent: [f32; 2],
    pub angle: f32,
}
/// Frame bounds may extend past the photograph; those pixels are transparent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameBounds {
    pub x: i64,
    pub y: i64,
    pub width: u32,
    pub height: u32,
}
impl Default for CropFrame {
    fn default() -> Self {
        Self {
            center: [0.5; 2],
            extent: [1.; 2],
            angle: 0.,
        }
    }
}
impl CropFrame {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.center
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 32768.)
                && self
                    .extent
                    .iter()
                    .all(|v| v.is_finite() && *v > 0. && *v <= 32768.)
                && self.angle.is_finite()
                && self.angle.abs() <= 360.,
            "Invalid crop frame"
        );
        Ok(())
    }
    pub fn bounds(self, canvas: (u32, u32)) -> FrameBounds {
        let (w, h) = (canvas.0 as f64, canvas.1 as f64);
        let (s, c) = self.angle.to_radians().sin_cos();
        let half = [
            (self.extent[0] as f64 * w * c.abs() as f64
                + self.extent[1] as f64 * h * s.abs() as f64)
                * 0.5,
            (self.extent[1] as f64 * h * c.abs() as f64
                + self.extent[0] as f64 * w * s.abs() as f64)
                * 0.5,
        ];
        FrameBounds {
            x: (self.center[0] as f64 * w - half[0] + 0.0001).floor() as i64,
            y: (self.center[1] as f64 * h - half[1] + 0.0001).floor() as i64,
            // Output size depends only on the aperture, never its position.
            width: (half[0] * 2. - 0.0001).ceil().max(1.) as u32,
            height: (half[1] * 2. - 0.0001).ceil().max(1.) as u32,
        }
    }
    fn mask(self, canvas: (u32, u32)) -> FrameMask {
        let dims = [canvas.0 as f32, canvas.1 as f32];
        FrameMask {
            center: [self.center[0] * dims[0], self.center[1] * dims[1]],
            half: [
                self.extent[0] * dims[0] * 0.5,
                self.extent[1] * dims[1] * 0.5,
            ],
            trig: self.angle.to_radians().sin_cos(),
        }
    }
    pub fn is_full(self) -> bool {
        self == Self::default()
    }
}
struct FrameMask {
    center: [f32; 2],
    half: [f32; 2],
    trig: (f32, f32),
}
impl FrameMask {
    /// Coverage changes alpha only; the interior pixels remain byte-for-byte upright.
    fn coverage(&self, pixel: [f32; 2]) -> f32 {
        let dx = pixel[0] - self.center[0];
        let dy = pixel[1] - self.center[1];
        let (s, c) = self.trig;
        let x = dx * c + dy * s;
        let y = -dx * s + dy * c;
        ((self.half[0] - x.abs()).min(self.half[1] - y.abs()) + 0.5).clamp(0., 1.)
    }
}

impl ImageTransform {
    pub fn validate(self) -> Result<()> {
        ensure!(self.quarter_turns < 4, "Invalid image rotation");
        ensure!(
            self.angle.is_finite() && self.angle.abs() <= 360.,
            "Invalid image angle"
        );
        if let Some(crop) = self.crop {
            crop.validate()?;
        }
        if let Some([w, h]) = self.size {
            ensure!(
                w > 0
                    && h > 0
                    && w <= 32768
                    && h <= 32768
                    && u64::from(w) * u64::from(h) <= 80_000_000,
                "Image size must be positive, at most 32,768 pixels per side and 80 megapixels"
            );
            let (ow, oh) = self.uncropped_dimensions((w, h));
            ensure!(
                ow <= 32768 && oh <= 32768 && u64::from(ow) * u64::from(oh) <= 80_000_000,
                "Rotated image exceeds the 80 megapixel limit"
            );
        }
        Ok(())
    }
    pub fn dimensions(self, source: (u32, u32)) -> (u32, u32) {
        let canvas = self.uncropped_dimensions(source);
        self.crop.map_or(canvas, |frame| {
            let b = frame.bounds(canvas);
            (b.width, b.height)
        })
    }
    pub fn uncropped_dimensions(self, source: (u32, u32)) -> (u32, u32) {
        let (w, h) = self.base_dimensions(source);
        let (sin, cos) = self.angle.to_radians().sin_cos();
        (
            (w as f32 * cos.abs() + h as f32 * sin.abs() - 0.0001)
                .ceil()
                .max(1.) as u32,
            (h as f32 * cos.abs() + w as f32 * sin.abs() - 0.0001)
                .ceil()
                .max(1.) as u32,
        )
    }
    pub fn base_dimensions(self, source: (u32, u32)) -> (u32, u32) {
        self.size.map(|s| (s[0], s[1])).unwrap_or_else(|| {
            if self.quarter_turns % 2 == 1 {
                (source.1, source.0)
            } else {
                source
            }
        })
    }
    /// Exact right angles use index permutations rather than an interpolation filter.
    pub fn normalized(mut self) -> Self {
        let turns = (self.angle / 90.).round();
        if (self.angle - turns * 90.).abs() < 0.001 {
            self.angle = 0.;
            self = self.rotated((turns as i32).rem_euclid(4) as u8);
        }
        if self.crop.is_some_and(CropFrame::is_full) {
            self.crop = None;
        }
        self
    }
    pub fn rotated(mut self, turns: u8) -> Self {
        self.quarter_turns = (self.quarter_turns + turns % 4) % 4;
        if turns % 2 == 1
            && let Some(size) = &mut self.size
        {
            size.swap(0, 1);
        }
        self
    }
    pub fn map_point(self, p: [f32; 2], source: (u32, u32)) -> [f32; 2] {
        self.canvas_point(
            self.oriented_point(point(p, self.quarter_turns), source, false),
            source,
            false,
        )
    }
    pub fn source_point(self, p: [f32; 2], source: (u32, u32)) -> [f32; 2] {
        point(
            self.oriented_point(self.canvas_point(p, source, true), source, true),
            (4 - self.quarter_turns) % 4,
        )
    }
    /// Convert between uncropped and cropped canvas coordinates, without rotating content.
    pub fn canvas_point(self, p: [f32; 2], source: (u32, u32), inverse: bool) -> [f32; 2] {
        let Some(frame) = self.crop else {
            return p;
        };
        let (w, h) = self.uncropped_dimensions(source);
        let b = frame.bounds((w, h));
        if inverse {
            [
                (p[0] * b.width as f32 + b.x as f32) / w as f32,
                (p[1] * b.height as f32 + b.y as f32) / h as f32,
            ]
        } else {
            [
                (p[0] * w as f32 - b.x as f32) / b.width as f32,
                (p[1] * h as f32 - b.y as f32) / b.height as f32,
            ]
        }
    }
    /// Input/output are normalized; pixel aspect is accounted for before rotating.
    pub fn oriented_point(self, p: [f32; 2], source: (u32, u32), inverse: bool) -> [f32; 2] {
        let (bw, bh) = self.base_dimensions(source);
        let (ow, oh) = self.uncropped_dimensions(source);
        let (from, to, angle) = if inverse {
            ((ow, oh), (bw, bh), -self.angle)
        } else {
            ((bw, bh), (ow, oh), self.angle)
        };
        let (sin, cos) = angle.to_radians().sin_cos();
        let (x, y) = ((p[0] - 0.5) * from.0 as f32, (p[1] - 0.5) * from.1 as f32);
        [
            (x * cos - y * sin) / to.0 as f32 + 0.5,
            (x * sin + y * cos) / to.1 as f32 + 0.5,
        ]
    }
}

pub fn remap_point(
    p: [f32; 2],
    before: ImageTransform,
    after: ImageTransform,
    source: (u32, u32),
) -> [f32; 2] {
    after.map_point(before.source_point(p, source), source)
}

pub fn point(mut p: [f32; 2], turns: u8) -> [f32; 2] {
    for _ in 0..turns % 4 {
        p = [1.0 - p[1], p[0]];
    }
    p
}
fn vector(mut p: [f32; 2], turns: u8) -> [f32; 2] {
    for _ in 0..turns % 4 {
        p = [-p[1], p[0]];
    }
    p
}

/// Re-open hidden crop margins once on a worker, with all existing retouch edits aligned.
/// Pointer movement only changes the frame mesh; it never triggers this preparation.
pub(crate) fn frame_preview(
    source: &Photo,
    mut edit: Edit,
    seg: Option<&std::sync::Arc<Segmentation>>,
    cancel: &AtomicBool,
) -> Result<image::RgbaImage> {
    let before = edit.transform;
    let mut full = before;
    full.crop = None;
    edit.transform_actions(before, full, source.dimensions());
    edit.transform = full;
    let canvas = full.uncropped_dimensions(source.dimensions());
    let base = full.base_dimensions(source.dimensions());
    let scale = (1440. / canvas.0.max(canvas.1) as f32).min(1.);
    full.size = Some([
        (base.0 as f32 * scale).round().max(1.) as u32,
        (base.1 as f32 * scale).round().max(1.) as u32,
    ]);
    let preview_source =
        crate::engine::photo_from_image(source.name.clone(), None, (*source.preview).clone());
    let photo = preview_source.transformed(full, Some(cancel))?;
    let seg = seg.map(|s| std::sync::Arc::new(s.transformed(full, preview_source.dimensions())));
    crate::engine::Renderer::default().render(&photo.preview, &edit, seg.as_ref(), Some(cancel))
}

impl Edit {
    pub fn transform_actions(
        &mut self,
        before: ImageTransform,
        after: ImageTransform,
        source: (u32, u32),
    ) {
        if before.angle == 0. && after.angle == 0. && before.crop.is_none() && after.crop.is_none()
        {
            self.rotate_actions((after.quarter_turns + 4 - before.quarter_turns) % 4);
            return;
        }
        let map = |p| remap_point(p, before, after, source);
        let zero = map([0., 0.]);
        let vector = |v: [f32; 2]| {
            let p = map(v);
            [p[0] - zero[0], p[1] - zero[1]]
        };
        let old = before.dimensions(source);
        let new = after.dimensions(source);
        let dx = vector([old.0.min(old.1) as f32 / old.0 as f32, 0.]);
        let dy = vector([0., old.0.min(old.1) as f32 / old.1 as f32]);
        let radius = ((dx[0] * dy[1] - dx[1] * dy[0]) * new.0 as f32 * new.1 as f32)
            .abs()
            .sqrt()
            / new.0.min(new.1) as f32;
        for s in self.strokes.iter_mut() {
            s.center = map(s.center);
            s.radius *= radius;
        }
        for s in self.warps.iter_mut() {
            s.center = map(s.center);
            s.delta = vector(s.delta);
            s.radius *= radius;
        }
        for s in self.clones.iter_mut() {
            s.center = map(s.center);
            s.source = map(s.source);
            s.radius *= radius;
        }
        for s in self.patches.iter_mut() {
            for p in &mut s.boundary {
                *p = map(*p);
            }
            s.offset = vector(s.offset);
            if let Some(feather) = &mut s.feather {
                *feather *= radius;
            }
        }
        for layer in self.stack.layers.iter_mut() {
            layer.edit.transform_actions(before, after, source);
            if let Some(mask) = &mut layer.mask {
                for s in mask.strokes.iter_mut() {
                    s.center = map(s.center);
                    s.radius *= radius;
                }
            }
        }
    }
    /// Move existing tools and layer masks with the document; history retains the previous geometry.
    pub fn rotate_actions(&mut self, turns: u8) {
        for s in self.strokes.iter_mut() {
            s.center = point(s.center, turns);
        }
        for s in self.warps.iter_mut() {
            s.center = point(s.center, turns);
            s.delta = vector(s.delta, turns);
        }
        for s in self.clones.iter_mut() {
            s.center = point(s.center, turns);
            s.source = point(s.source, turns);
        }
        for s in self.patches.iter_mut() {
            for p in &mut s.boundary {
                *p = point(*p, turns);
            }
            s.offset = vector(s.offset, turns);
        }
        for layer in self.stack.layers.iter_mut() {
            layer.edit.rotate_actions(turns);
            if let Some(mask) = &mut layer.mask {
                for s in mask.strokes.iter_mut() {
                    s.center = point(s.center, turns);
                }
            }
        }
    }
}

impl Photo {
    pub fn transformed(
        &self,
        transform: ImageTransform,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self> {
        transform.validate()?;
        ensure!(
            self.applied_transform == ImageTransform::default(),
            "A transform needs the original source image"
        );
        let check = || -> Result<()> {
            ensure!(
                !cancel.is_some_and(|c| c.load(Ordering::Relaxed)),
                "Cancelled"
            );
            Ok(())
        };
        check()?;
        if transform == ImageTransform::default() {
            return Ok(self.clone());
        }
        let (ow, oh) = transform.dimensions(self.dimensions());
        ensure!(
            ow <= 32768 && oh <= 32768 && u64::from(ow) * u64::from(oh) <= 80_000_000,
            "Crop frame exceeds the 80 megapixel limit"
        );
        let native = self.open_native()?;
        use std::borrow::Cow;
        let mut pixels = match transform.quarter_turns {
            1 => Cow::Owned(imageops::rotate90(&*native.original)),
            2 => Cow::Owned(imageops::rotate180(&*native.original)),
            3 => Cow::Owned(imageops::rotate270(&*native.original)),
            _ => Cow::Borrowed(&*native.original),
        };
        check()?;
        let (w, h) = transform.base_dimensions(native.dimensions());
        if pixels.dimensions() != (w, h) {
            pixels = Cow::Owned(imageops::resize(&*pixels, w, h, FilterType::Lanczos3));
        }
        if transform.angle != 0. {
            let (ow, oh) = transform.uncropped_dimensions(native.dimensions());
            ensure!(
                ow <= 32768 && oh <= 32768 && u64::from(ow) * u64::from(oh) <= 80_000_000,
                "Rotated image exceeds the 80 megapixel limit"
            );
            let mut output = image::RgbaImage::new(ow, oh);
            output
                .as_mut()
                .par_chunks_mut(ow as usize * 4)
                .enumerate()
                .for_each(|(y, row)| {
                    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                        return;
                    }
                    for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        let uv = transform.oriented_point(
                            [(x as f32 + 0.5) / ow as f32, (y as f32 + 0.5) / oh as f32],
                            native.dimensions(),
                            true,
                        );
                        if uv.iter().any(|v| *v < 0. || *v > 1.) {
                            continue;
                        }
                        pixel.copy_from_slice(&cubic_pixel(
                            &pixels,
                            uv[0] * w as f32 - 0.5,
                            uv[1] * h as f32 - 0.5,
                        ));
                    }
                });
            pixels = Cow::Owned(output);
        }
        if let Some(frame) = transform.crop {
            let canvas = pixels.dimensions();
            let bounds = frame.bounds(canvas);
            let mask = frame.mask(canvas);
            let mut output = image::RgbaImage::new(bounds.width, bounds.height);
            output
                .as_mut()
                .par_chunks_mut(bounds.width as usize * 4)
                .enumerate()
                .for_each(|(y, row)| {
                    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                        return;
                    }
                    for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        let sx = bounds.x + x as i64;
                        let sy = bounds.y + y as i64;
                        if sx < 0 || sy < 0 || sx >= canvas.0 as i64 || sy >= canvas.1 as i64 {
                            continue;
                        }
                        pixel.copy_from_slice(&pixels.get_pixel(sx as u32, sy as u32).0);
                        let alpha = mask.coverage([sx as f32 + 0.5, sy as f32 + 0.5]);
                        pixel[3] = (pixel[3] as f32 * alpha).round() as u8;
                        if pixel[3] == 0 {
                            pixel.fill(0);
                        }
                    }
                });
            pixels = Cow::Owned(output);
        }
        check()?;
        let mut photo = if matches!(&pixels, Cow::Borrowed(_)) {
            native.clone()
        } else {
            crate::engine::photo_from_image(native.name, native.path, pixels.into_owned())
        };
        photo.applied_transform = transform;
        Ok(photo)
    }
}

impl Segmentation {
    pub fn transformed(&self, transform: ImageTransform, source: (u32, u32)) -> Self {
        let mut s = self.rotated(transform.quarter_turns);
        if (transform.angle == 0. && transform.crop.is_none()) || s.width == 0 || s.height == 0 {
            return s;
        }
        // Expand compact maps through the existing sampler, including its crop origin.
        let old = s.resample(s.width + 1, s.height + 1);
        let (ow, oh) = transform.dimensions(source);
        let scale = s.width.max(s.height) as f32 / ow.max(oh) as f32;
        let (w, h) = (
            (ow as f32 * scale).round().max(1.) as u32,
            (oh as f32 * scale).round().max(1.) as u32,
        );
        let canvas = transform.uncropped_dimensions(source);
        let mask = transform.crop.map(|frame| frame.mask(canvas));
        let coordinates: Vec<_> = (0..w * h)
            .into_par_iter()
            .map(|i| {
                let uv = transform.canvas_point(
                    [
                        (i % w) as f32 / w as f32 + 0.5 / w as f32,
                        (i / w) as f32 / h as f32 + 0.5 / h as f32,
                    ],
                    source,
                    true,
                );
                if mask.as_ref().is_some_and(|mask| {
                    mask.coverage([uv[0] * canvas.0 as f32, uv[1] * canvas.1 as f32]) == 0.
                }) {
                    [-1.; 2]
                } else {
                    transform.oriented_point(uv, source, true)
                }
            })
            .collect();
        let scalar = |data: &[f32]| -> crate::shared::SharedVec<f32> {
            if data.is_empty() {
                return Default::default();
            }
            coordinates
                .par_iter()
                .map(|p| sample_map(data, old.width, old.height, *p, 0.))
                .collect()
        };
        let rgb = |data: &[[f32; 3]], outside| -> crate::shared::SharedVec<[f32; 3]> {
            if data.is_empty() {
                return Default::default();
            }
            coordinates
                .par_iter()
                .map(|p| sample_map(data, old.width, old.height, *p, outside))
                .collect()
        };
        s.skin = scalar(&old.skin);
        s.teeth = scalar(&old.teeth);
        s.eyes = scalar(&old.eyes);
        s.background = scalar(&old.background);
        s.under_eyes = scalar(&old.under_eyes);
        s.forehead = scalar(&old.forehead);
        s.laugh_lines = scalar(&old.laugh_lines);
        s.contour = scalar(&old.contour);
        s.highlight = scalar(&old.highlight);
        s.blemish = scalar(&old.blemish);
        s.neural_blend = rgb(&old.neural_blend, [0.5; 3]);
        s.repair_delta = rgb(&old.repair_delta, [0.; 3]);
        for face in &mut s.faces {
            for p in &mut face.landmarks {
                let xy = transform.canvas_point(
                    transform.oriented_point([p[0], p[1]], source, false),
                    source,
                    false,
                );
                p[0] = xy[0];
                p[1] = xy[1];
            }
            let [x0, y0, x1, y1] = face.bounds;
            let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|p| {
                transform.canvas_point(transform.oriented_point(p, source, false), source, false)
            });
            face.bounds = [
                corners.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min),
                corners.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min),
                corners
                    .iter()
                    .map(|p| p[0])
                    .fold(f32::NEG_INFINITY, f32::max),
                corners
                    .iter()
                    .map(|p| p[1])
                    .fold(f32::NEG_INFINITY, f32::max),
            ];
        }
        s.width = w;
        s.height = h;
        s.map_crop = None;
        s.compact_generated_maps();
        s
    }
    /// Exact index permutation also preserves compact generated maps and explicit masks.
    pub fn rotated(&self, turns: u8) -> Self {
        let mut s = self.clone();
        for _ in 0..turns % 4 {
            let (w, h) = (s.width, s.height);
            let crop = s.map_crop;
            let (mw, mh) = crop.map_or((w, h), |c| (c.width, c.height));
            fn rotate<T: Clone>(v: &mut crate::shared::SharedVec<T>, w: u32, h: u32) {
                if v.is_empty() || v.len() != w as usize * h as usize {
                    return;
                }
                *v = (0..w * h)
                    .map(|i| {
                        let (x, y) = (i % h, i / h);
                        v[((h - 1 - x) * w + y) as usize].clone()
                    })
                    .collect();
            }
            for map in [
                &mut s.skin,
                &mut s.teeth,
                &mut s.eyes,
                &mut s.under_eyes,
                &mut s.forehead,
                &mut s.laugh_lines,
                &mut s.contour,
                &mut s.highlight,
                &mut s.blemish,
            ] {
                rotate(map, mw, mh);
            }
            rotate(&mut s.background, w, h);
            rotate(&mut s.neural_blend, mw, mh);
            rotate(&mut s.repair_delta, mw, mh);
            s.map_crop = crop.map(|c| crate::engine::Crop {
                x: h - c.y - c.height,
                y: c.x,
                width: c.height,
                height: c.width,
            });
            for face in &mut s.faces {
                for p in &mut face.landmarks {
                    let xy = point([p[0], p[1]], 1);
                    p[0] = xy[0];
                    p[1] = xy[1];
                }
                let [x0, y0, x1, y1] = face.bounds;
                face.bounds = [1.0 - y1, x0, 1.0 - y0, x1];
            }
            s.width = h;
            s.height = w;
        }
        s
    }
}

trait MapValue: Copy + Send + Sync {
    fn mix(a: Self, b: Self, amount: f32) -> Self;
}
impl MapValue for f32 {
    fn mix(a: Self, b: Self, t: f32) -> Self {
        a + (b - a) * t
    }
}
impl MapValue for [f32; 3] {
    fn mix(a: Self, b: Self, t: f32) -> Self {
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
    }
}
fn sample_map<T: MapValue>(data: &[T], w: u32, h: u32, uv: [f32; 2], outside: T) -> T {
    if uv.iter().any(|v| *v < 0. || *v > 1.) {
        return outside;
    }
    let (x, y) = (
        (uv[0] * w as f32 - 0.5).clamp(0., (w - 1) as f32),
        (uv[1] * h as f32 - 0.5).clamp(0., (h - 1) as f32),
    );
    let (x0, y0) = (x as u32, y as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    T::mix(
        T::mix(
            data[(y0 * w + x0) as usize],
            data[(y0 * w + x1) as usize],
            x.fract(),
        ),
        T::mix(
            data[(y1 * w + x0) as usize],
            data[(y1 * w + x1) as usize],
            x.fract(),
        ),
        y.fract(),
    )
}
fn cubic_pixel(image: &image::RgbaImage, x: f32, y: f32) -> [u8; 4] {
    let kernel = |d: f32| {
        let d = d.abs();
        if d < 1. {
            1.5 * d.powi(3) - 2.5 * d.powi(2) + 1.
        } else if d < 2. {
            -0.5 * d.powi(3) + 2.5 * d.powi(2) - 4. * d + 2.
        } else {
            0.
        }
    };
    let mut rgba = [0.; 4];
    for iy in y.floor() as i32 - 1..=y.floor() as i32 + 2 {
        for ix in x.floor() as i32 - 1..=x.floor() as i32 + 2 {
            let weight = kernel(x - ix as f32) * kernel(y - iy as f32);
            let p = image.get_pixel(
                ix.clamp(0, image.width() as i32 - 1) as u32,
                iy.clamp(0, image.height() as i32 - 1) as u32,
            );
            let alpha = p[3] as f32 / 255.;
            for i in 0..3 {
                rgba[i] += p[i] as f32 * alpha * weight;
            }
            rgba[3] += alpha * weight;
        }
    }
    if rgba[3] <= 0. {
        return [0; 4];
    }
    [
        (rgba[0] / rgba[3]).round().clamp(0., 255.) as u8,
        (rgba[1] / rgba[3]).round().clamp(0., 255.) as u8,
        (rgba[2] / rgba[3]).round().clamp(0., 255.) as u8,
        (rgba[3] * 255.).round().clamp(0., 255.) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{self, Crop, ExportOptions, ExportSize, Stroke, Target};
    use image::{Rgba, RgbaImage};

    fn source() -> Photo {
        engine::photo_from_image(
            "geometry.png".into(),
            None,
            RgbaImage::from_fn(36, 54, |x, y| {
                Rgba([(x * 7) as u8, (y * 4) as u8, ((x + y) * 2) as u8, 255])
            }),
        )
    }
    #[test]
    fn frame_translation_preserves_size_and_native_pixels_with_transparent_margins() {
        let photo = source();
        let size = photo.dimensions();
        for (dx, dy) in [(-8, -11), (7, 9), (50, 70), (-50, -70)] {
            let transform = ImageTransform {
                crop: Some(CropFrame {
                    center: [
                        0.5 + dx as f32 / size.0 as f32,
                        0.5 + dy as f32 / size.1 as f32,
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            };
            let bounds = transform.crop.unwrap().bounds(size);
            assert_eq!((bounds.x, bounds.y), (dx, dy));
            let working = photo.transformed(transform, None).unwrap();
            assert_eq!(working.dimensions(), size);
            for (x, y, pixel) in working.original.enumerate_pixels() {
                let (sx, sy) = (x as i64 + dx, y as i64 + dy);
                if sx >= 0 && sy >= 0 && sx < size.0 as i64 && sy < size.1 as i64 {
                    assert_eq!(pixel, photo.original.get_pixel(sx as u32, sy as u32));
                } else {
                    assert_eq!(pixel.0, [0; 4]);
                }
            }
            let directory = tempfile::tempdir().unwrap();
            let exported = engine::export(
                &photo,
                &Edit {
                    transform,
                    ..Default::default()
                },
                None,
                directory.path(),
                ExportSize::Original,
                true,
                100,
            )
            .unwrap();
            assert_eq!(image::open(exported).unwrap().to_rgba8(), *working.original);
            let saved: ImageTransform =
                ron::from_str(&ron::to_string(&transform).unwrap()).unwrap();
            assert_eq!(saved, transform);
        }
    }
    #[test]
    fn moving_a_rotated_frame_never_changes_dimensions_and_maps_remain_aligned() {
        let photo = source();
        let source_size = photo.dimensions();
        let frame = CropFrame {
            extent: [0.93, 0.87],
            angle: 33.,
            ..Default::default()
        };
        let size = ImageTransform {
            crop: Some(frame),
            ..Default::default()
        }
        .dimensions(source_size);
        for center in [[0.5, 0.5], [-0.2, 0.8], [1.2, -0.8], [20., 30.]] {
            let transform = ImageTransform {
                crop: Some(CropFrame { center, ..frame }),
                ..Default::default()
            };
            assert_eq!(transform.dimensions(source_size), size);
            let original = [0.4, 0.55];
            let mapped = transform.map_point(original, source_size);
            let restored = transform.source_point(mapped, source_size);
            assert!((restored[0] - original[0]).abs() < 0.00001);
            assert!((restored[1] - original[1]).abs() < 0.00001);
        }
        let translated = ImageTransform {
            crop: Some(CropFrame {
                center: [0.5 - 9. / 36., 0.5 - 12. / 54.],
                ..Default::default()
            }),
            ..Default::default()
        };
        let dense = Segmentation {
            width: 36,
            height: 54,
            skin: vec![1.; 36 * 54].into(),
            ..Default::default()
        };
        let aligned = dense.transformed(translated, source_size).resample(72, 108);
        assert_eq!(
            aligned.skin[0], 0.,
            "AI must not affect transparent margins"
        );
        assert!(aligned.skin[40 * 72 + 40] > 0.99);
        let mut edit = Edit::default();
        edit.strokes.push(Stroke {
            target: Target::Heal,
            center: [0.4, 0.55],
            radius: 0.05,
            softness: 0.7,
            strength: 100.,
            erase: false,
        });
        edit.transform_actions(Default::default(), translated, source_size);
        assert_eq!(
            edit.strokes[0].center,
            translated.map_point([0.4, 0.55], source_size)
        );
        edit.transform_actions(translated, Default::default(), source_size);
        assert!((edit.strokes[0].center[0] - 0.4).abs() < 0.00001);
        assert!((edit.strokes[0].center[1] - 0.55).abs() < 0.00001);
    }
    #[test]
    fn crop_copies_native_pixels_and_frame_rotation_never_rotates_the_photo() {
        let photo = source();
        let axis = ImageTransform {
            crop: Some(CropFrame {
                center: [0.5; 2],
                extent: [0.5, 1. / 3.],
                angle: 0.,
            }),
            ..Default::default()
        };
        let cropped = photo.transformed(axis, None).unwrap();
        assert_eq!(cropped.dimensions(), (18, 18));
        assert_eq!(
            *cropped.original,
            imageops::crop_imm(&*photo.original, 9, 18, 18, 18).to_image()
        );
        let rotated = ImageTransform {
            crop: Some(CropFrame {
                angle: 33.,
                ..axis.crop.unwrap()
            }),
            ..axis
        };
        let clipped = photo.transformed(rotated, None).unwrap();
        let bounds = rotated.crop.unwrap().bounds(photo.dimensions());
        let mut opaque = 0;
        let mut transparent = 0;
        for (x, y, p) in clipped.original.enumerate_pixels() {
            if p[3] == 255 {
                opaque += 1;
                assert_eq!(
                    p,
                    photo
                        .original
                        .get_pixel((bounds.x + x as i64) as u32, (bounds.y + y as i64) as u32),
                    "Frame rotation resampled or rotated an interior pixel"
                );
            } else if p[3] == 0 {
                transparent += 1;
            }
        }
        assert!(opaque > 100 && transparent > 50);
        assert_eq!(rotated.quarter_turns, 0);
        assert_eq!(rotated.angle, 0.);
        let directory = tempfile::tempdir().unwrap();
        let exported = engine::export(
            &photo,
            &Edit {
                transform: rotated,
                ..Default::default()
            },
            None,
            directory.path(),
            ExportSize::Original,
            true,
            100,
        )
        .unwrap();
        assert_eq!(image::open(exported).unwrap().to_rgba8(), *clipped.original);
        assert_eq!(
            *photo
                .transformed(Default::default(), None)
                .unwrap()
                .original,
            *photo.original
        );
        assert!(
            photo
                .transformed(rotated, Some(&AtomicBool::new(true)))
                .is_err()
        );
        let legacy: ImageTransform =
            ron::from_str("(quarter_turns:1,size:None,angle:17.0)").unwrap();
        assert_eq!(legacy.crop, None);
    }
    #[test]
    fn crop_reprojects_tools_and_compact_maps_and_reopens_hidden_margins() {
        let photo = source();
        let target = ImageTransform {
            crop: Some(CropFrame {
                center: [0.5; 2],
                extent: [0.5, 1. / 3.],
                angle: 14.,
            }),
            ..Default::default()
        };
        let mut edit = Edit::default();
        edit.strokes.push(Stroke {
            target: Target::Heal,
            center: [0.4, 0.5],
            radius: 0.05,
            softness: 0.7,
            strength: 100.,
            erase: false,
        });
        edit.ensure_stack();
        edit.active_layer_mut().unwrap().mask = Some(crate::layer_mask::LayerMask {
            base: 0,
            strokes: vec![crate::layer_mask::MaskStroke {
                center: [0.4, 0.5],
                radius: 0.05,
                softness: 0.7,
                strength: 100.,
                reveal: true,
            }]
            .into(),
        });
        let original = edit.clone();
        edit.transform_actions(Default::default(), target, photo.dimensions());
        assert_eq!(
            edit.strokes[0].center,
            target.map_point([0.4, 0.5], photo.dimensions())
        );
        assert_eq!(
            edit.active_layer().unwrap().mask.as_ref().unwrap().strokes[0].center,
            edit.strokes[0].center
        );
        edit.transform_actions(target, Default::default(), photo.dimensions());
        assert!((edit.strokes[0].center[0] - original.strokes[0].center[0]).abs() < 0.00001);
        assert!((edit.strokes[0].radius - original.strokes[0].radius).abs() < 0.00001);
        let dense = Segmentation {
            width: 36,
            height: 54,
            skin: (0..36 * 54)
                .map(|i| {
                    if (12..24).contains(&(i % 36)) && (19..34).contains(&(i / 36)) {
                        1.
                    } else {
                        0.
                    }
                })
                .collect(),
            ..Default::default()
        };
        let mut compact = dense.clone();
        compact.compact_generated_maps();
        let a = dense
            .transformed(target, photo.dimensions())
            .resample(80, 80);
        let b = compact
            .transformed(target, photo.dimensions())
            .resample(80, 80);
        assert!(a.skin.iter().any(|v| *v > 0.9) && a.skin.contains(&0.));
        assert!(
            a.skin
                .iter()
                .zip(b.skin.iter())
                .all(|(x, y)| (x - y).abs() < 0.00001)
        );
        let mut edit = Edit::default();
        edit.settings.exposure = 0.25;
        let expected = engine::render(&photo.preview, &edit, None);
        edit.transform = target;
        let preview = frame_preview(&photo, edit, None, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            preview, expected,
            "Re-opening a crop lost the margins or changed existing adjustments"
        );
        let bad = ImageTransform {
            crop: Some(CropFrame {
                extent: [f32::NAN, 1.],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(bad.validate().is_err());
    }
    #[test]
    fn rotations_are_lossless_and_resizing_always_uses_source_pixels() {
        let photo = source();
        let mut t = ImageTransform::default();
        for _ in 0..4 {
            t = t.rotated(1);
        }
        assert_eq!(
            *photo.transformed(t, None).unwrap().original,
            *photo.original
        );
        let rotated = photo.transformed(t.rotated(1), None).unwrap();
        assert_eq!(rotated.dimensions(), (54, 36));
        assert_eq!(
            *rotated.original.get_pixel(53, 0),
            *photo.original.get_pixel(0, 0)
        );
        let small = ImageTransform {
            size: Some([12, 18]),
            ..t
        };
        let reduced = photo.transformed(small, None).unwrap();
        let restored = photo
            .transformed(
                ImageTransform {
                    size: Some([36, 54]),
                    ..t
                },
                None,
            )
            .unwrap();
        assert_eq!(*restored.original, *photo.original);
        assert_eq!(reduced.dimensions(), (12, 18));
        assert!(reduced.transformed(t, None).is_err());
        assert!(
            photo
                .transformed(small, Some(&AtomicBool::new(true)))
                .is_err()
        );
        assert_eq!(photo.dimensions(), (36, 54));
    }
    #[test]
    fn dense_and_compact_ai_maps_rotate_identically() {
        let mut s = Segmentation {
            width: 8,
            height: 6,
            skin: (0..48)
                .map(|i| {
                    if i % 8 >= 2 && i % 8 < 5 && i / 8 >= 1 && i / 8 < 4 {
                        i as f32 / 48.
                    } else {
                        0.
                    }
                })
                .collect(),
            background: (0..48).map(|i| i as f32 / 48.).collect(),
            ..Default::default()
        };
        let dense = s.rotated(1);
        s.compact_generated_maps();
        let compact = s.rotated(1);
        let a = dense.resample(13, 17);
        let b = compact.resample(13, 17);
        assert_eq!(a.skin, b.skin);
        assert_eq!(a.background, b.background);
        assert_eq!(s.rotated(1).rotated(3).skin, s.skin);
        assert_eq!(s.rotated(1).rotated(3).map_crop, s.map_crop);
    }
    #[test]
    fn layers_and_manual_edits_follow_rotation_without_changing_document_geometry() {
        let mut edit = Edit::default();
        edit.strokes.push(Stroke {
            target: Target::Heal,
            center: [0.25, 0.5],
            radius: 0.1,
            erase: false,
            strength: 100.,
            softness: 0.6,
        });
        edit.ensure_stack();
        edit.active_layer_mut().unwrap().mask = Some(crate::layer_mask::LayerMask {
            base: 0,
            strokes: vec![crate::layer_mask::MaskStroke {
                center: [0.25, 0.5],
                radius: 0.1,
                softness: 0.6,
                strength: 100.,
                reveal: true,
            }]
            .into(),
        });
        edit.rotate_actions(1);
        edit.transform = ImageTransform {
            quarter_turns: 1,
            size: Some([54, 36]),
            ..Default::default()
        };
        edit.sync_active_layer();
        let mask = edit.active_layer().unwrap().mask.as_ref().unwrap();
        assert_eq!(mask.sample_uv([0.5, 0.25], (54, 36)), 255);
        assert_eq!(edit.strokes[0].center, [0.5, 0.25]);
        let transform = edit.transform;
        edit.add_layer(crate::layer_stack::LayerType::Retouch);
        edit.select_layer(1);
        assert_eq!(edit.transform, transform);
        assert!(
            edit.stack
                .layers
                .iter()
                .all(|l| l.edit.transform == Default::default())
        );
    }
    #[test]
    fn native_regions_and_lossless_exports_use_the_document_size_and_rotation() {
        let photo = source();
        let edit = Edit {
            transform: ImageTransform {
                quarter_turns: 1,
                size: Some([27, 18]),
                ..Default::default()
            },
            ..Default::default()
        };
        let working = photo.transformed(edit.transform, None).unwrap();
        let full = engine::render(&working.original, &edit, None);
        let crop = Crop {
            x: 3,
            y: 2,
            width: 10,
            height: 8,
        };
        let region = engine::render_region(&working.original, &edit, None, crop, None).unwrap();
        assert_eq!(
            region,
            imageops::crop_imm(&full, crop.x, crop.y, crop.width, crop.height).to_image()
        );
        let directory = tempfile::tempdir().unwrap();
        let path = engine::export_with_options(
            &photo,
            &edit,
            None,
            directory.path(),
            &ExportOptions {
                size: ExportSize::Original,
                png: true,
                quality: 100,
                center: [0.5; 2],
                cancel: None,
            },
        )
        .unwrap();
        assert_eq!(image::open(path).unwrap().to_rgba8(), full);
        assert_eq!(photo.dimensions(), (36, 54));
    }

    #[test]
    fn free_rotation_preserves_coordinates_masks_and_exports_from_the_original() {
        let photo = source();
        let before = ImageTransform {
            quarter_turns: 1,
            size: Some([42, 28]),
            angle: -12.,
            ..Default::default()
        };
        let after = ImageTransform {
            quarter_turns: 2,
            size: Some([27, 40]),
            angle: 31.,
            ..Default::default()
        };
        let uv = [0.23, 0.61];
        let displayed = before.map_point(uv, photo.dimensions());
        let next = remap_point(displayed, before, after, photo.dimensions());
        let restored = after.source_point(next, photo.dimensions());
        assert!((restored[0] - uv[0]).abs() < 0.000001 && (restored[1] - uv[1]).abs() < 0.000001);
        let mut edit = Edit::default();
        edit.strokes.push(Stroke {
            target: Target::Heal,
            center: displayed,
            radius: 0.05,
            erase: false,
            strength: 90.,
            softness: 0.7,
        });
        edit.transform_actions(before, after, photo.dimensions());
        assert_eq!(edit.strokes[0].center, next);
        edit.transform_actions(after, before, photo.dimensions());
        assert!((edit.strokes[0].radius - 0.05).abs() < 0.000001);
        let s = Segmentation {
            width: 36,
            height: 54,
            skin: (0..36 * 54)
                .map(|i| if i % 36 < 18 { 1. } else { 0. })
                .collect(),
            ..Default::default()
        };
        let aligned = s.transformed(after, photo.dimensions());
        let dense = aligned.resample(60, 60);
        assert!(dense.skin.contains(&0.) && dense.skin.iter().any(|v| *v > 0.9));
        let directory = tempfile::tempdir().unwrap();
        let edit = Edit {
            transform: after,
            ..Default::default()
        };
        let working = photo.transformed(after, None).unwrap();
        let path = engine::export_with_options(
            &photo,
            &edit,
            None,
            directory.path(),
            &ExportOptions {
                size: ExportSize::Original,
                png: true,
                quality: 100,
                center: [0.5; 2],
                cancel: None,
            },
        )
        .unwrap();
        assert_eq!(image::open(path).unwrap().to_rgba8(), *working.original);
        assert!(working.original.pixels().any(|p| p[3] == 0));
        assert_eq!(
            *photo
                .transformed(Default::default(), None)
                .unwrap()
                .original,
            *photo.original
        );
        let exact = ImageTransform {
            angle: 90.,
            ..Default::default()
        }
        .normalized();
        assert_eq!(
            *photo.transformed(exact, None).unwrap().original,
            imageops::rotate90(&*photo.original)
        );
    }
}
