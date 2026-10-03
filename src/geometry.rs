//! Inverse sampling keeps every reshaping operation reversible and resolution independent.
use crate::engine::{Edit, Segmentation, Settings, brush_weight};
use image::RgbaImage;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceMesh {
    pub landmarks: Vec<[f32; 3]>,
    pub bounds: [f32; 4],
    pub confidence: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpStroke {
    /// Brush centre after the push, in displayed image coordinates.
    pub center: [f32; 2],
    pub delta: [f32; 2],
    pub radius: f32,
    pub softness: f32,
    pub strength: f32,
}

pub fn active(edit: &Edit, seg: Option<&Segmentation>) -> bool {
    if !edit.stack.layers.is_empty() {
        return edit
            .resolved_layers()
            .iter()
            .any(|layer| active(&layer.edit, seg));
    }
    let effective = edit.effective_layers();
    let edit = effective.as_ref();
    let s = edit.settings.effective();
    !edit.warps.is_empty()
        || (seg.is_some_and(|s| !s.faces.is_empty())
            && [s.eye_size, s.nose_width, s.lip_plumpness, s.jawline]
                .iter()
                .any(|v| *v != 0.0))
}

fn scale_feature(uv: &mut [f32; 2], center: [f32; 2], radius: [f32; 2], scale: [f32; 2]) {
    let d = [uv[0] - center[0], uv[1] - center[1]];
    let distance = (d[0] / radius[0].max(0.0001)).hypot(d[1] / radius[1].max(0.0001));
    let weight = brush_weight(distance, 0.65);
    for c in 0..2 {
        uv[c] = center[c] + d[c] / (1.0 + (scale[c] - 1.0) * weight);
    }
}

struct PreparedFace {
    aspect: [f32; 2],
    origin: [f32; 2],
    cos: f32,
    sin: f32,
    points: Vec<[f32; 2]>,
}
impl PreparedFace {
    fn new(face: &FaceMesh, dims: (u32, u32)) -> Option<Self> {
        if face.landmarks.len() < 468 {
            return None;
        }
        let short = dims.0.min(dims.1) as f32;
        let aspect = [dims.0 as f32 / short, dims.1 as f32 / short];
        let origin = [
            face.landmarks[1][0] * aspect[0],
            face.landmarks[1][1] * aspect[1],
        ];
        let a = face.landmarks[33];
        let b = face.landmarks[263];
        let angle = ((b[1] - a[1]) * aspect[1]).atan2((b[0] - a[0]) * aspect[0]);
        let (cos, sin) = (angle.cos(), angle.sin());
        let mut result = Self {
            aspect,
            origin,
            cos,
            sin,
            points: vec![],
        };
        result.points = face
            .landmarks
            .iter()
            .map(|p| result.local([p[0], p[1]]))
            .collect();
        Some(result)
    }
    fn local(&self, point: [f32; 2]) -> [f32; 2] {
        let d = [
            point[0] * self.aspect[0] - self.origin[0],
            point[1] * self.aspect[1] - self.origin[1],
        ];
        [
            d[0] * self.cos + d[1] * self.sin,
            -d[0] * self.sin + d[1] * self.cos,
        ]
    }
}
fn face_inverse(uv: &mut [f32; 2], face: &PreparedFace, s: &Settings) {
    let (aspect, origin, cos, sin) = (face.aspect, face.origin, face.cos, face.sin);
    let local = |point| face.local(point);
    let p = |i: usize| face.points[i];
    let mut mapped = local(*uv);
    let input = uv;
    let uv = &mut mapped;
    let fw = (p(454)[0] - p(234)[0]).abs().max(0.001);
    let fh = (p(152)[1] - p(10)[1]).abs().max(0.001);
    if s.jawline != 0.0 {
        let amount = s.jawline / 100.0;
        for (i, direction) in [(172, 1.0), (397, -1.0)] {
            let center = p(i);
            let shifted = [center[0] + direction * fw * 0.12 * amount, center[1]];
            let distance =
                ((uv[0] - shifted[0]) / (fw * 0.34)).hypot((uv[1] - shifted[1]) / (fh * 0.28));
            uv[0] -= direction * fw * 0.12 * amount * brush_weight(distance, 1.0);
        }
    }
    if s.lip_plumpness != 0.0 {
        let a = p(61);
        let b = p(291);
        let top = p(0);
        let bottom = p(17);
        scale_feature(
            uv,
            [(a[0] + b[0]) * 0.5, (top[1] + bottom[1]) * 0.5],
            [
                (a[0] - b[0]).abs() * 0.8,
                (top[1] - bottom[1]).abs() * 2.0 + fh * 0.025,
            ],
            [
                1.0 + s.lip_plumpness / 100.0 * 0.12,
                1.0 + s.lip_plumpness / 100.0 * 0.7,
            ],
        );
    }
    if s.nose_width != 0.0 {
        let a = p(98);
        let b = p(327);
        let center = p(2);
        scale_feature(
            uv,
            center,
            [(a[0] - b[0]).abs() * 1.4, fh * 0.18],
            [1.0 + s.nose_width / 100.0 * 0.45, 1.0],
        );
    }
    if s.eye_size != 0.0 {
        for (a, b, top, bottom) in [(33, 133, 159, 145), (362, 263, 386, 374)] {
            let a = p(a);
            let b = p(b);
            let top = p(top);
            let bottom = p(bottom);
            scale_feature(
                uv,
                [(a[0] + b[0]) * 0.5, (top[1] + bottom[1]) * 0.5],
                [
                    (a[0] - b[0]).abs() * 0.95,
                    (top[1] - bottom[1]).abs() * 2.6 + fh * 0.012,
                ],
                [1.0 + s.eye_size / 100.0 * 0.5; 2],
            );
        }
    }
    *input = [
        (origin[0] + mapped[0] * cos - mapped[1] * sin) / aspect[0],
        (origin[1] + mapped[0] * sin + mapped[1] * cos) / aspect[1],
    ];
}

/// Reuses face projections and visits only brushes whose footprint contains the query.
/// The descending cursor preserves stroke order even when a push crosses a spatial cell.
pub(crate) struct Mapping<'a> {
    edit: Cow<'a, Edit>,
    settings: Settings,
    faces: Vec<PreparedFace>,
    dims: (u32, u32),
    bins: std::collections::HashMap<(i32, i32), Vec<usize>>,
    global: Vec<usize>,
    stack: Vec<Mapping<'a>>,
}
impl<'a> Mapping<'a> {
    pub(crate) fn new(edit: &'a Edit, seg: Option<&Segmentation>, dims: (u32, u32)) -> Self {
        if !edit.stack.layers.is_empty() {
            let mut mapping = Self::prepare(Cow::Owned(Edit::default()), seg, dims);
            // Brush coordinates belong to the selected layer's input image. Lower layers
            // have already been composited into that input; invert only this layer and above.
            let mut selected = false;
            for layer in edit.stack.layers.iter() {
                selected |= layer.id == edit.stack.active;
                if selected && layer.factor() > 0.0 {
                    let flat = if layer.id == edit.stack.active {
                        edit.flat_edit()
                    } else {
                        layer.edit.flat_edit()
                    };
                    let mut flat = flat.effective_layers().into_owned();
                    for warp in &mut *flat.warps {
                        warp.strength *= layer.factor();
                    }
                    for value in [
                        &mut flat.settings.eye_size,
                        &mut flat.settings.nose_width,
                        &mut flat.settings.lip_plumpness,
                        &mut flat.settings.jawline,
                    ] {
                        *value *= layer.factor();
                    }
                    mapping
                        .stack
                        .push(Self::prepare(Cow::Owned(flat), seg, dims));
                }
            }
            return mapping;
        }
        // Resolve once for the whole stroke or deformation field, never per sampled pixel.
        let edit = edit.effective_layers();
        Self::prepare(edit, seg, dims)
    }
    fn prepare(edit: Cow<'a, Edit>, seg: Option<&Segmentation>, dims: (u32, u32)) -> Self {
        let mut bins = std::collections::HashMap::<_, Vec<usize>>::new();
        let mut global = vec![];
        let short = dims.0.min(dims.1) as f32;
        for (i, s) in edit.warps.iter().enumerate() {
            let rx = s.radius.max(0.0001) * short / dims.0 as f32 + 0.000001;
            let ry = s.radius.max(0.0001) * short / dims.1 as f32 + 0.000001;
            let (x0, x1) = (
                ((s.center[0] - rx) * 32.0).floor() as i32,
                ((s.center[0] + rx) * 32.0).floor() as i32,
            );
            let (y0, y1) = (
                ((s.center[1] - ry) * 32.0).floor() as i32,
                ((s.center[1] + ry) * 32.0).floor() as i32,
            );
            if ((i64::from(x1) - i64::from(x0) + 1).max(0) as u64)
                .saturating_mul((i64::from(y1) - i64::from(y0) + 1).max(0) as u64)
                > 4096
            {
                global.push(i);
                continue;
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    bins.entry((x, y)).or_default().push(i);
                }
            }
        }
        Self {
            settings: edit.settings.effective(),
            dims,
            bins,
            global,
            faces: seg
                .into_iter()
                .flat_map(|s| s.faces.iter())
                .filter_map(|f| PreparedFace::new(f, dims))
                .collect(),
            edit,
            stack: vec![],
        }
    }
    pub(crate) fn source(&self, mut result: [f32; 2]) -> [f32; 2] {
        if !self.stack.is_empty() {
            for layer in self.stack.iter().rev() {
                result = layer.source(result);
            }
            return result;
        }
        let short = self.dims.0.min(self.dims.1) as f32;
        let mut cursor = self.edit.warps.len();
        while cursor > 0 {
            let key = (
                (result[0] * 32.0).floor() as i32,
                (result[1] * 32.0).floor() as i32,
            );
            let candidate = |indices: &[usize]| {
                let pos = indices.partition_point(|i| *i < cursor);
                pos.checked_sub(1).map(|i| indices[i])
            };
            let local = self.bins.get(&key).and_then(|v| candidate(v));
            let Some(index) = local.max(candidate(&self.global)) else {
                break;
            };
            cursor = index;
            let stroke = &self.edit.warps[index];
            let d = [
                (result[0] - stroke.center[0]) * self.dims.0 as f32 / short,
                (result[1] - stroke.center[1]) * self.dims.1 as f32 / short,
            ];
            let weight = brush_weight(
                d[0].hypot(d[1]) / stroke.radius.max(0.0001),
                stroke.softness,
            ) * (stroke.strength / 100.0).clamp(0.0, 1.0);
            for (c, v) in result.iter_mut().enumerate() {
                *v -= stroke.delta[c] * weight;
            }
        }
        for face in self.faces.iter().rev() {
            face_inverse(&mut result, face, &self.settings);
        }
        result
    }
}

/// Maps a brush position on a reshaped portrait back to the immutable source.
pub fn source_uv(
    uv: [f32; 2],
    edit: &Edit,
    seg: Option<&Segmentation>,
    dims: (u32, u32),
) -> [f32; 2] {
    Mapping::new(edit, seg, dims).source(uv)
}

pub fn display_uv(
    source: [f32; 2],
    edit: &Edit,
    seg: Option<&Segmentation>,
    dims: (u32, u32),
) -> [f32; 2] {
    let mapping = Mapping::new(edit, seg, dims);
    let mut uv = source;
    for _ in 0..12 {
        let mapped = mapping.source(uv);
        for c in 0..2 {
            uv[c] += source[c] - mapped[c];
        }
    }
    uv
}

type WarpKey = (
    (u32, u32),
    [f32; 4],
    crate::shared::SharedVec<WarpStroke>,
    Vec<FaceMesh>,
);
#[derive(Default)]
pub(crate) struct WarpCache {
    key: Option<WarpKey>,
    field: std::sync::Arc<Vec<[f32; 2]>>,
    dims: (u32, u32),
}
impl WarpCache {
    pub(crate) fn bytes(&self) -> usize {
        self.field.len() * 8
    }

    pub(crate) fn displacement(
        &mut self,
        edit: &Edit,
        seg: Option<&Segmentation>,
        image_dims: (u32, u32),
    ) -> DisplacementField {
        let effective = edit.effective_layers();
        let edit = effective.as_ref();
        let s = edit.settings.effective();
        let shape = [s.eye_size, s.nose_width, s.lip_plumpness, s.jawline];
        let faces = seg.map_or(&[][..], |s| s.faces.as_slice());
        let grid_dims = field_dimensions(edit, image_dims);
        let same_base = self
            .key
            .as_ref()
            .is_some_and(|key| key.0 == image_dims && key.1 == shape && key.3 == faces);
        let unchanged = same_base
            && self
                .key
                .as_ref()
                .is_some_and(|key| key.2.same_storage(&edit.warps) || key.2 == edit.warps);
        if !unchanged {
            let prefix = self
                .key
                .as_ref()
                .filter(|key| {
                    same_base
                        && self.dims == grid_dims
                        && key.2.len() < edit.warps.len()
                        && edit.warps.starts_with(&key.2)
                })
                .map(|key| key.2.len());
            if let Some(prefix) = prefix {
                // New inverse brushes leave every point outside their combined footprints unchanged.
                // Re-evaluate affected vertices using the complete ordered mapping, not interpolation
                // of a previous result, so growing strokes stay byte-identical to a fresh field.
                let (gw, gh) = grid_dims;
                let (w, h) = image_dims;
                let short = w.min(h) as f32;
                let mut bounds = [gw, gh, 0, 0];
                for stroke in &edit.warps[prefix..] {
                    let rx = stroke.radius.max(0.0001) * short / w as f32;
                    let ry = stroke.radius.max(0.0001) * short / h as f32;
                    bounds[0] = bounds[0].min(
                        ((stroke.center[0] - rx) * gw as f32)
                            .floor()
                            .clamp(0.0, gw as f32) as u32,
                    );
                    bounds[1] = bounds[1].min(
                        ((stroke.center[1] - ry) * gh as f32)
                            .floor()
                            .clamp(0.0, gh as f32) as u32,
                    );
                    bounds[2] = bounds[2].max(
                        ((stroke.center[0] + rx) * gw as f32)
                            .ceil()
                            .clamp(0.0, gw as f32) as u32,
                    );
                    bounds[3] = bounds[3].max(
                        ((stroke.center[1] + ry) * gh as f32)
                            .ceil()
                            .clamp(0.0, gh as f32) as u32,
                    );
                }
                let mapping = Mapping::new(edit, seg, image_dims);
                let stride = (gw + 1) as usize;
                let field = std::sync::Arc::make_mut(&mut self.field);
                let start = bounds[1] as usize * stride;
                let end = (bounds[3] + 1) as usize * stride;
                field[start..end]
                    .par_chunks_mut(stride)
                    .enumerate()
                    .for_each(|(row, values)| {
                        let y = bounds[1] + row as u32;
                        for x in bounds[0]..=bounds[2] {
                            values[x as usize] =
                                mapping.source([x as f32 / gw as f32, y as f32 / gh as f32]);
                        }
                    });
            } else {
                let field = DisplacementField::new(edit, seg, image_dims);
                self.dims = field.dims;
                self.field = field.field;
            }
            self.key = Some((image_dims, shape, edit.warps.clone(), faces.to_vec()));
        }
        DisplacementField {
            dims: self.dims,
            field: self.field.clone(),
        }
    }
}
pub fn warp(image: RgbaImage, edit: &Edit, seg: Option<&Segmentation>) -> RgbaImage {
    warp_inner(image, edit, seg, None)
}
pub(crate) fn warp_cached(
    image: RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    cache: &mut WarpCache,
) -> RgbaImage {
    warp_inner(image, edit, seg, Some(cache))
}
pub(crate) struct DisplacementField {
    dims: (u32, u32),
    field: std::sync::Arc<Vec<[f32; 2]>>,
}
impl DisplacementField {
    /// Conservative destination cells whose bilinear source samples can touch
    /// a changed source region. Folded/overlapping warps are included as well.
    pub(crate) fn output_damage(
        &self,
        source: crate::engine::Crop,
        visible: crate::engine::Crop,
        (w, h): (u32, u32),
    ) -> crate::engine::Crop {
        let (gw, gh) = self.dims;
        let gx0 = (visible.x as f64 * gw as f64 / w as f64).floor() as u32;
        let gy0 = (visible.y as f64 * gh as f64 / h as f64).floor() as u32;
        let gx1 = ((visible.x + visible.width) as f64 * gw as f64 / w as f64).ceil() as u32;
        let gy1 = ((visible.y + visible.height) as f64 * gh as f64 / h as f64).ceil() as u32;
        let mut bounds = [gw, gh, 0, 0];
        for y in gy0..gy1.min(gh) {
            for x in gx0..gx1.min(gw) {
                let i = (y * (gw + 1) + x) as usize;
                let q = [
                    self.field[i],
                    self.field[i + 1],
                    self.field[i + (gw + 1) as usize],
                    self.field[i + (gw + 1) as usize + 1],
                ];
                let mut min = [f32::INFINITY; 2];
                let mut max = [f32::NEG_INFINITY; 2];
                for p in q {
                    for c in 0..2 {
                        let side = if c == 0 { w } else { h };
                        let value = (p[c] * side as f32).clamp(0., side.saturating_sub(1) as f32);
                        min[c] = min[c].min(value);
                        max[c] = max[c].max(value);
                    }
                }
                if max[0] >= source.x as f32
                    && min[0] <= (source.x + source.width) as f32
                    && max[1] >= source.y as f32
                    && min[1] <= (source.y + source.height) as f32
                {
                    bounds[0] = bounds[0].min(x);
                    bounds[1] = bounds[1].min(y);
                    bounds[2] = bounds[2].max(x + 1);
                    bounds[3] = bounds[3].max(y + 1);
                }
            }
        }
        if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
            return crate::engine::Crop {
                width: 0,
                height: 0,
                ..visible
            };
        }
        let x = ((bounds[0] as f64 * w as f64 / gw as f64).floor() as u32)
            .saturating_sub(2)
            .max(visible.x);
        let y = ((bounds[1] as f64 * h as f64 / gh as f64).floor() as u32)
            .saturating_sub(2)
            .max(visible.y);
        let right = ((bounds[2] as f64 * w as f64 / gw as f64).ceil() as u32 + 2)
            .min(visible.x + visible.width);
        let bottom = ((bounds[3] as f64 * h as f64 / gh as f64).ceil() as u32 + 2)
            .min(visible.y + visible.height);
        crate::engine::Crop {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }
    pub(crate) fn new(edit: &Edit, seg: Option<&Segmentation>, (w, h): (u32, u32)) -> Self {
        let effective = edit.effective_layers();
        let edit = effective.as_ref();
        let mapping = Mapping::new(edit, seg, (w, h));
        let (gw, gh) = field_dimensions(edit, (w, h));
        let mut field = vec![[0.0; 2]; ((gw + 1) * (gh + 1)) as usize];
        field.par_iter_mut().enumerate().for_each(|(i, p)| {
            *p = mapping.source([
                (i as u32 % (gw + 1)) as f32 / gw as f32,
                (i as u32 / (gw + 1)) as f32 / gh as f32,
            ]);
        });
        Self {
            dims: (gw, gh),
            field: std::sync::Arc::new(field),
        }
    }
    pub(crate) fn source(&self, uv: [f32; 2]) -> [f32; 2] {
        let (gw, gh) = self.dims;
        let gx = uv[0] * gw as f32;
        let gy = uv[1] * gh as f32;
        let (x, y) = (gx.floor() as u32, gy.floor() as u32);
        let (tx, ty) = (gx - x as f32, gy - y as f32);
        let q = [
            self.field[(y * (gw + 1) + x) as usize],
            self.field[(y * (gw + 1) + x + 1) as usize],
            self.field[((y + 1) * (gw + 1) + x) as usize],
            self.field[((y + 1) * (gw + 1) + x + 1) as usize],
        ];
        std::array::from_fn(|c| {
            (q[0][c] * (1.0 - tx) + q[1][c] * tx) * (1.0 - ty)
                + (q[2][c] * (1.0 - tx) + q[3][c] * tx) * ty
        })
    }
}

pub(crate) fn field_dimensions(edit: &Edit, (w, h): (u32, u32)) -> (u32, u32) {
    let grid = edit
        .warps
        .iter()
        .map(|s| (8.0 / s.radius.max(0.001)).ceil() as u32)
        .max()
        .unwrap_or(384)
        .clamp(384, 2048);
    if w >= h {
        (grid, (grid as f32 * h as f32 / w as f32).ceil() as u32)
    } else {
        ((grid as f32 * w as f32 / h as f32).ceil() as u32, grid)
    }
}
fn warp_inner(
    image: RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    cache: Option<&mut WarpCache>,
) -> RgbaImage {
    let effective = edit.effective_layers();
    let edit = effective.as_ref();
    if !active(edit, seg) {
        return image;
    }
    let (w, h) = image.dimensions();
    let build = || {
        let prepared = DisplacementField::new(edit, seg, (w, h));
        (prepared.dims, prepared.field)
    };
    let ((gw, gh), field) = if let Some(cache) = cache {
        let field = cache.displacement(edit, seg, (w, h));
        (field.dims, field.field)
    } else {
        build()
    };
    let mut output = vec![0; (w * h * 4) as usize];
    output.par_chunks_mut(4).enumerate().for_each(|(i, out)| {
        let gx = (i as u32 % w) as f32 / w as f32 * gw as f32;
        let gy = (i as u32 / w) as f32 / h as f32 * gh as f32;
        let (x, y) = (gx.floor() as u32, gy.floor() as u32);
        let (tx, ty) = (gx - x as f32, gy - y as f32);
        let q = [
            field[(y * (gw + 1) + x) as usize],
            field[(y * (gw + 1) + x + 1) as usize],
            field[((y + 1) * (gw + 1) + x) as usize],
            field[((y + 1) * (gw + 1) + x + 1) as usize],
        ];
        let uv = std::array::from_fn::<_, 2, _>(|c| {
            (q[0][c] * (1.0 - tx) + q[1][c] * tx) * (1.0 - ty)
                + (q[2][c] * (1.0 - tx) + q[3][c] * tx) * ty
        });
        let color = sample_rgba(&image, uv[0] * w as f32, uv[1] * h as f32);
        for c in 0..4 {
            out[c] = color[c].round().clamp(0.0, 255.0) as u8;
        }
    });
    RgbaImage::from_raw(w, h, output).unwrap()
}

pub fn sample_rgba(image: &RgbaImage, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = image.dimensions();
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (ix, iy) = (x.floor() as u32, y.floor() as u32);
    let (tx, ty) = (x - ix as f32, y - iy as f32);
    let p = [
        image.get_pixel(ix, iy),
        image.get_pixel((ix + 1).min(w - 1), iy),
        image.get_pixel(ix, (iy + 1).min(h - 1)),
        image.get_pixel((ix + 1).min(w - 1), (iy + 1).min(h - 1)),
    ];
    std::array::from_fn(|c| {
        (p[0][c] as f32 * (1.0 - tx) + p[1][c] as f32 * tx) * (1.0 - ty)
            + (p[2][c] as f32 * (1.0 - tx) + p[3][c] as f32 * tx) * ty
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incremental_warp_vertices_match_fresh_fields_after_overlap_resize_undo_and_opacity() {
        let mut edit = Edit::default();
        let mut cache = WarpCache::default();
        let mut seg = Segmentation::default();
        for step in 0..15 {
            match step {
                8 => edit.warps[0].radius = 0.015,
                9 => edit.warps[0].strength = 30.0,
                10 => {
                    edit.warps.pop();
                }
                11 => {
                    edit.layers
                        .get_mut(crate::layers::LayerKind::Liquify)
                        .opacity = 50.0
                }
                12 => {
                    edit.settings.jawline = 50.0;
                    seg.faces.push(FaceMesh {
                        landmarks: vec![],
                        bounds: [0.2; 4],
                        confidence: 0.8,
                    });
                }
                _ => edit.warps.push(WarpStroke {
                    center: [0.49 + step as f32 * 0.005, 0.5],
                    delta: [0.04, -0.025],
                    radius: 0.08,
                    softness: 0.6,
                    strength: 90.0,
                }),
            }
            let dims = if step < 14 { (160, 240) } else { (240, 160) };
            let cached = cache.displacement(&edit, Some(&seg), dims);
            let effective = edit.effective_layers();
            let fresh = DisplacementField::new(&effective, Some(&seg), dims);
            assert_eq!(cached.dims, fresh.dims);
            assert_eq!(cached.field, fresh.field, "Incorrect field at step {step}");
        }
    }
    #[test]
    fn layer_opacity_maps_brushes_to_visible_geometry_and_reuses_warp_fields() {
        use crate::layers::LayerKind;
        let image = RgbaImage::from_fn(100, 100, |x, y| {
            image::Rgba([x as u8 * 2, y as u8 * 2, 0, 255])
        });
        let mut edit = Edit::default();
        edit.warps.push(WarpStroke {
            center: [0.55, 0.5],
            delta: [0.05, 0.0],
            radius: 0.2,
            softness: 0.7,
            strength: 100.0,
        });
        edit.layers.get_mut(LayerKind::Liquify).opacity = 50.0;
        let uv = source_uv([0.55, 0.5], &edit, None, (100, 100));
        assert!((uv[0] - 0.525).abs() < 0.00001);
        let visible = display_uv(uv, &edit, None, (100, 100));
        assert!((visible[0] - 0.55).abs() < 0.00001);
        let mut cache = WarpCache::default();
        let first = warp_cached(image.clone(), &edit, None, &mut cache);
        let field = cache.field.clone();
        assert_eq!(warp_cached(image.clone(), &edit, None, &mut cache), first);
        assert!(std::sync::Arc::ptr_eq(&field, &cache.field));
        edit.layers.get_mut(LayerKind::Liquify).opacity = 25.0;
        warp_cached(image.clone(), &edit, None, &mut cache);
        assert!(!std::sync::Arc::ptr_eq(&field, &cache.field));
        edit.layers.get_mut(LayerKind::Liquify).visible = false;
        assert!(!active(&edit, None));
        assert_eq!(source_uv([0.55, 0.5], &edit, None, (100, 100)), [0.55, 0.5]);
        assert_eq!(
            display_uv([0.55, 0.5], &edit, None, (100, 100)),
            [0.55, 0.5]
        );
        assert_eq!(warp(image.clone(), &edit, None), image);
        assert_eq!(edit.warps[0].strength, 100.0);
    }
    #[test]
    fn manual_push_moves_texture_and_keeps_distant_pixels() {
        let original = RgbaImage::from_fn(100, 100, |x, y| {
            image::Rgba([x as u8 * 2, y as u8 * 2, 0, 255])
        });
        let mut edit = Edit::default();
        edit.warps.push(WarpStroke {
            center: [0.55, 0.5],
            delta: [0.05, 0.0],
            radius: 0.2,
            softness: 0.7,
            strength: 100.0,
        });
        let result = warp(original.clone(), &edit, None);
        assert_eq!(result.get_pixel(5, 5), original.get_pixel(5, 5));
        assert!(result.get_pixel(55, 50)[0] < original.get_pixel(55, 50)[0] - 8);
        let uv = source_uv([0.55, 0.5], &edit, None, (100, 100));
        assert!((uv[0] - 0.5).abs() < 0.00001);
        assert_eq!(warp(original.clone(), &Edit::default(), None), original);
    }
    #[test]
    fn face_reshaping_follows_eye_rotation_and_stays_zero_at_neutral() {
        let mut landmarks = vec![[0.5, 0.5, 0.0]; 478];
        for (i, p) in [
            (33, [0.3, 0.35]),
            (133, [0.43, 0.35]),
            (263, [0.7, 0.35]),
            (362, [0.57, 0.35]),
            (159, [0.365, 0.32]),
            (145, [0.365, 0.38]),
            (386, [0.635, 0.32]),
            (374, [0.635, 0.38]),
            (234, [0.2, 0.5]),
            (454, [0.8, 0.5]),
            (10, [0.5, 0.15]),
            (152, [0.5, 0.85]),
        ] {
            landmarks[i] = [p[0], p[1], 0.0];
        }
        let face = FaceMesh {
            landmarks,
            bounds: [0.2, 0.15, 0.6, 0.7],
            confidence: 0.95,
        };
        let seg = Segmentation {
            faces: vec![face.clone()],
            ..Default::default()
        };
        let mut edit = Edit::default();
        let input = [0.4, 0.35];
        let neutral = source_uv(input, &edit, Some(&seg), (100, 100));
        assert!((neutral[0] - input[0]).abs() < 0.00001);
        edit.settings.eye_size = 100.0;
        let normal = source_uv(input, &edit, Some(&seg), (100, 100));
        let mut rotated = face;
        for p in &mut rotated.landmarks {
            let x = p[0];
            p[0] = 1.0 - p[1];
            p[1] = x;
        }
        let rotated = Segmentation {
            faces: vec![rotated],
            ..Default::default()
        };
        let mapped = source_uv(
            [1.0 - input[1], input[0]],
            &edit,
            Some(&rotated),
            (100, 100),
        );
        assert!((mapped[0] - (1.0 - normal[1])).abs() < 0.00001);
        assert!((mapped[1] - normal[0]).abs() < 0.00001);
        let roundtrip = display_uv(normal, &edit, Some(&seg), (100, 100));
        assert!((roundtrip[0] - input[0]).abs() < 0.0001);
    }

    #[test]
    fn spatial_brush_lookup_matches_ordered_inverse_when_pushes_cross_cells() {
        let mut edit = Edit::default();
        let mut seed = 73491u32;
        let mut random = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 16777216.0
        };
        for i in 0..200 {
            edit.warps.push(WarpStroke {
                center: [random(), random()],
                delta: [(random() - 0.5) * 0.14, (random() - 0.5) * 0.14],
                radius: if i == 0 { 10.0 } else { 0.01 + random() * 0.25 },
                softness: random(),
                strength: 30. + random() * 70.,
            });
        }
        let mapping = Mapping::new(&edit, None, (700, 1100));
        for y in -3..43 {
            for x in -3..43 {
                let point = [x as f32 / 40., y as f32 / 40.];
                let mut expected = point;
                for stroke in edit.warps.iter().rev() {
                    let d = [
                        (expected[0] - stroke.center[0]) * 700. / 700.,
                        (expected[1] - stroke.center[1]) * 1100. / 700.,
                    ];
                    let weight = brush_weight(
                        d[0].hypot(d[1]) / stroke.radius.max(0.0001),
                        stroke.softness,
                    ) * (stroke.strength / 100.).clamp(0., 1.);
                    for (c, v) in expected.iter_mut().enumerate() {
                        *v -= stroke.delta[c] * weight;
                    }
                }
                assert_eq!(mapping.source(point), expected, "{point:?}");
            }
        }
    }
}
