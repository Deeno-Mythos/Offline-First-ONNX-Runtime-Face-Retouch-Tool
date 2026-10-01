//! Texture transfer for spot healing and immutable-source clone stamps.
use crate::engine::{Edit, Target, brush_weight};
use half::f16;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CloneStamp {
    pub center: [f32; 2],
    pub source: [f32; 2],
    pub radius: f32,
    pub softness: f32,
    pub strength: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PatchStroke {
    /// Lasso boundary around the destination area, in original-image UV coordinates.
    pub boundary: Vec<[f32; 2]>,
    /// Translation from each destination pixel to its source texture, in UV units.
    pub offset: [f32; 2],
    pub softness: f32,
    pub strength: f32,
}

/// Donor selection and boundary corrections are computed once per edit, independently of tiles.
pub struct PreparedCleanup {
    operations: Vec<PreparedOperation>,
}
enum PreparedOperation {
    Stamp(PreparedStamp),
    Patch(PreparedPatch),
}
struct PreparedStamp {
    center: [f32; 2],
    radius: f32,
    softness: f32,
    strength: f32,
    offset: [f32; 2],
    correction: [f32; 3],
    clip_source: bool,
}
struct PreparedPatch {
    boundary: Vec<[f32; 2]>,
    bounds: crate::engine::Crop,
    offset: [f32; 2],
    correction: [f32; 3],
    feather: f32,
    strength: f32,
}

fn sample_image(image: &RgbaImage, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = image.dimensions();
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (ix, iy) = (x.floor() as u32, y.floor() as u32);
    let (tx, ty) = (x - ix as f32, y - iy as f32);
    let at = |x: u32, y: u32| {
        let p = image.get_pixel(x.min(w - 1), y.min(h - 1));
        [
            crate::engine::linear_byte(p[0]),
            crate::engine::linear_byte(p[1]),
            crate::engine::linear_byte(p[2]),
            p[3] as f32 / 255.0,
        ]
    };
    let p = [
        at(ix, iy),
        at(ix + 1, iy),
        at(ix, iy + 1),
        at(ix + 1, iy + 1),
    ];
    std::array::from_fn(|c| {
        (p[0][c] * (1.0 - tx) + p[1][c] * tx) * (1.0 - ty)
            + (p[2][c] * (1.0 - tx) + p[3][c] * tx) * ty
    })
}

impl PreparedCleanup {
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.operations.capacity() * std::mem::size_of::<PreparedOperation>()
            + self
                .operations
                .iter()
                .map(|operation| match operation {
                    PreparedOperation::Patch(patch) => {
                        patch.boundary.capacity() * std::mem::size_of::<[f32; 2]>()
                    }
                    PreparedOperation::Stamp(_) => 0,
                })
                .sum::<usize>()
    }
    /// `fine_sample` returns the full-image fine blur at a global pixel coordinate. If a fine
    /// blur is unnecessary, callers can supply immutable linear samples of the original image.
    pub fn prepare(
        source: &RgbaImage,
        edit: &Edit,
        fine_sample: impl Fn(f32, f32) -> [f32; 4],
    ) -> Option<Self> {
        Self::prepare_cancellable(source, edit, fine_sample, None)
            .expect("Uncancelled cleanup preparation")
    }

    pub fn prepare_cancellable(
        source: &RgbaImage,
        edit: &Edit,
        fine_sample: impl Fn(f32, f32) -> [f32; 4],
        cancel: Option<&AtomicBool>,
    ) -> anyhow::Result<Option<Self>> {
        let (w, h) = source.dimensions();
        if w == 0 || h == 0 {
            return Ok(None);
        }
        let check = || -> anyhow::Result<()> {
            anyhow::ensure!(
                !cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)),
                "Cleanup preparation cancelled"
            );
            Ok(())
        };
        check()?;
        let mut operations = Vec::new();
        let healing = edit.settings.effective().healing / 100.0;
        for stroke in edit
            .strokes
            .iter()
            .filter(|s| healing != 0.0 && s.target == Target::Heal && !s.erase)
        {
            check()?;
            let (cx, cy) = (stroke.center[0] * w as f32, stroke.center[1] * h as f32);
            let radius = (stroke.radius * w.min(h) as f32).max(1.0);
            let mut best = (f32::MAX, [0.0; 2], [0.0; 3]);
            for j in 0..32 {
                check()?;
                let angle = j as f32 * std::f32::consts::TAU / 16.0;
                let offset = [
                    (angle.cos() * radius * if j < 16 { 2.7 } else { 4.2 }).round(),
                    (angle.sin() * radius * if j < 16 { 2.7 } else { 4.2 }).round(),
                ];
                if cx + offset[0] - radius < 0.0
                    || cy + offset[1] - radius < 0.0
                    || cx + offset[0] + radius >= w as f32
                    || cy + offset[1] + radius >= h as f32
                {
                    continue;
                }
                let mut correction = [0.0; 3];
                let mut score = 0.0;
                for k in 0..20 {
                    let a = k as f32 * std::f32::consts::TAU / 20.0;
                    let (x, y) = (cx + a.cos() * radius * 1.1, cy + a.sin() * radius * 1.1);
                    let target = fine_sample(x, y);
                    let clean = fine_sample(x + offset[0], y + offset[1]);
                    for c in 0..3 {
                        correction[c] += (target[c] - clean[c]) / 20.0;
                        score += (target[c] - clean[c]).powi(2);
                    }
                }
                let middle = sample_image(source, cx + offset[0], cy + offset[1]);
                let smooth = fine_sample(cx + offset[0], cy + offset[1]);
                for c in 0..3 {
                    score += (middle[c] - smooth[c]).powi(2) * 10.0;
                }
                if score < best.0 {
                    best = (score, offset, correction);
                }
            }
            if best.0 != f32::MAX {
                operations.push(PreparedOperation::Stamp(PreparedStamp {
                    center: [cx, cy],
                    radius,
                    softness: stroke.softness,
                    strength: healing,
                    offset: best.1,
                    correction: best.2,
                    clip_source: false,
                }));
            }
        }
        for clone in edit.clones.iter().filter(|clone| clone.strength > 0.0) {
            check()?;
            operations.push(PreparedOperation::Stamp(PreparedStamp {
                center: [clone.center[0] * w as f32, clone.center[1] * h as f32],
                radius: (clone.radius * w.min(h) as f32).max(0.5),
                softness: clone.softness,
                strength: clone.strength / 100.0,
                offset: [
                    (clone.source[0] - clone.center[0]) * w as f32,
                    (clone.source[1] - clone.center[1]) * h as f32,
                ],
                correction: [0.0; 3],
                clip_source: true,
            }));
        }
        for patch in &edit.patches {
            check()?;
            if let Some(patch) = prepare_patch(source, patch) {
                operations.push(PreparedOperation::Patch(patch));
            }
        }
        Ok((!operations.is_empty()).then_some(Self { operations }))
    }

    /// Retains original source coordinates even when donor pixels are far outside this region.
    pub fn apply_region(
        &self,
        source: &RgbaImage,
        linear: &[[f32; 4]],
        crop: crate::engine::Crop,
    ) -> Option<Vec<[f32; 4]>> {
        let (w, h) = source.dimensions();
        if crop.width == 0
            || crop.height == 0
            || crop.x.checked_add(crop.width)? > w
            || crop.y.checked_add(crop.height)? > h
            || linear.len() != (crop.width * crop.height) as usize
        {
            return None;
        }
        let intersects = |bounds: crate::engine::Crop| {
            bounds.x < crop.x + crop.width
                && bounds.y < crop.y + crop.height
                && crop.x < bounds.x + bounds.width
                && crop.y < bounds.y + bounds.height
        };
        let mut result = None;
        for operation in &self.operations {
            let bounds = match operation {
                PreparedOperation::Stamp(stamp) => {
                    let x = (stamp.center[0] - stamp.radius)
                        .floor()
                        .clamp(0.0, w as f32) as u32;
                    let y = (stamp.center[1] - stamp.radius)
                        .floor()
                        .clamp(0.0, h as f32) as u32;
                    let right = (stamp.center[0] + stamp.radius).ceil().clamp(0.0, w as f32) as u32;
                    let bottom =
                        (stamp.center[1] + stamp.radius).ceil().clamp(0.0, h as f32) as u32;
                    crate::engine::Crop {
                        x,
                        y,
                        width: right.saturating_sub(x),
                        height: bottom.saturating_sub(y),
                    }
                }
                PreparedOperation::Patch(patch) => patch.bounds,
            };
            if !intersects(bounds) {
                continue;
            }
            let result = result.get_or_insert_with(|| linear.to_vec());
            let min_x = bounds.x.max(crop.x);
            let min_y = bounds.y.max(crop.y);
            let max_x = (bounds.x + bounds.width).min(crop.x + crop.width);
            let max_y = (bounds.y + bounds.height).min(crop.y + crop.height);
            for y in min_y..max_y {
                for x in min_x..max_x {
                    let (clean, weight) = match operation {
                        PreparedOperation::Stamp(stamp) => {
                            let weight = brush_weight(
                                (x as f32 + 0.5 - stamp.center[0])
                                    .hypot(y as f32 + 0.5 - stamp.center[1])
                                    / stamp.radius,
                                stamp.softness,
                            ) * stamp.strength.clamp(0.0, 1.0);
                            if weight == 0.0 {
                                continue;
                            }
                            let sx = x as f32 + stamp.offset[0];
                            let sy = y as f32 + stamp.offset[1];
                            if stamp.clip_source
                                && (sx < 0.0
                                    || sy < 0.0
                                    || sx > (w - 1) as f32
                                    || sy > (h - 1) as f32)
                            {
                                continue;
                            }
                            let mut clean = sample_image(source, sx, sy);
                            for (c, value) in clean.iter_mut().enumerate().take(3) {
                                *value += stamp.correction[c];
                            }
                            (clean, weight)
                        }
                        PreparedOperation::Patch(patch) => {
                            let point = [x as f32 + 0.5, y as f32 + 0.5];
                            if !inside_polygon(point, &patch.boundary) {
                                continue;
                            }
                            let edge_distance = patch
                                .boundary
                                .iter()
                                .enumerate()
                                .map(|(i, a)| {
                                    distance_to_segment(
                                        point,
                                        *a,
                                        patch.boundary[(i + 1) % patch.boundary.len()],
                                    )
                                })
                                .fold(f32::INFINITY, f32::min);
                            let feather_weight = if patch.feather <= 0.0 {
                                1.0
                            } else {
                                let t = (edge_distance / patch.feather).clamp(0.0, 1.0);
                                t * t * (3.0 - 2.0 * t)
                            };
                            let sx = point[0] + patch.offset[0];
                            let sy = point[1] + patch.offset[1];
                            if sx < 0.0 || sy < 0.0 || sx > (w - 1) as f32 || sy > (h - 1) as f32 {
                                continue;
                            }
                            let mut clean = sample_image(source, sx, sy);
                            for (c, value) in clean.iter_mut().enumerate().take(3) {
                                *value = (*value + patch.correction[c]).clamp(0.0, 1.0);
                            }
                            (clean, feather_weight * patch.strength.clamp(0.0, 1.0))
                        }
                    };
                    let out = &mut result[((y - crop.y) * crop.width + x - crop.x) as usize];
                    for c in 0..4 {
                        out[c] += (clean[c] - out[c]) * weight;
                    }
                }
            }
        }
        result
    }
}

fn prepare_patch(source: &RgbaImage, patch: &PatchStroke) -> Option<PreparedPatch> {
    let (w, h) = source.dimensions();
    if patch.boundary.len() < 3 || patch.strength <= 0.0 {
        return None;
    }
    let boundary: Vec<_> = patch
        .boundary
        .iter()
        .map(|p| [p[0] * w as f32, p[1] * h as f32])
        .collect();
    if boundary.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    let left = boundary.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
    let right = boundary
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let top = boundary.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
    let bottom = boundary
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max);
    let (min_x, max_x, min_y, max_y) = (
        left.floor().clamp(0.0, w as f32) as u32,
        right.ceil().clamp(0.0, w as f32) as u32,
        top.floor().clamp(0.0, h as f32) as u32,
        bottom.ceil().clamp(0.0, h as f32) as u32,
    );
    if min_x >= max_x || min_y >= max_y {
        return None;
    }
    let offset = [patch.offset[0] * w as f32, patch.offset[1] * h as f32];
    let mut correction = [0.0; 3];
    let mut samples = 0.0;
    for i in 0..boundary.len() {
        let (a, b) = (boundary[i], boundary[(i + 1) % boundary.len()]);
        for sample_index in 0..8 {
            let t = (sample_index as f32 + 0.5) / 8.0;
            let (x, y) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
            let (sx, sy) = (x + offset[0], y + offset[1]);
            if x < 0.0
                || y < 0.0
                || x > (w - 1) as f32
                || y > (h - 1) as f32
                || sx < 0.0
                || sy < 0.0
                || sx > (w - 1) as f32
                || sy > (h - 1) as f32
            {
                continue;
            }
            let target = sample_image(source, x, y);
            let clean = sample_image(source, sx, sy);
            for c in 0..3 {
                correction[c] += target[c] - clean[c];
            }
            samples += 1.0;
        }
    }
    if samples > 0.0 {
        for c in &mut correction {
            *c = (*c / samples).clamp(-0.15, 0.15);
        }
    }
    Some(PreparedPatch {
        boundary,
        offset,
        correction,
        bounds: crate::engine::Crop {
            x: min_x,
            y: min_y,
            width: max_x - min_x,
            height: max_y - min_y,
        },
        feather: (right - left).min(bottom - top).max(1.0) * 0.12 * patch.softness.clamp(0.0, 1.0),
        strength: patch.strength / 100.0,
    })
}

fn sample(data: &[[f32; 4]], w: u32, h: u32, x: f32, y: f32) -> [f32; 4] {
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (ix, iy) = (x.floor() as u32, y.floor() as u32);
    let (tx, ty) = (x - ix as f32, y - iy as f32);
    let at = |x: u32, y: u32| data[(y.min(h - 1) * w + x.min(w - 1)) as usize];
    let p = [
        at(ix, iy),
        at(ix + 1, iy),
        at(ix, iy + 1),
        at(ix + 1, iy + 1),
    ];
    std::array::from_fn(|c| {
        (p[0][c] * (1.0 - tx) + p[1][c] * tx) * (1.0 - ty)
            + (p[2][c] * (1.0 - tx) + p[3][c] * tx) * ty
    })
}

fn sample_half(data: &[[f16; 4]], w: u32, h: u32, x: f32, y: f32) -> [f32; 4] {
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (ix, iy) = (x.floor() as u32, y.floor() as u32);
    let (tx, ty) = (x - ix as f32, y - iy as f32);
    let at = |x: u32, y: u32| data[(y.min(h - 1) * w + x.min(w - 1)) as usize].map(f16::to_f32);
    let p = [
        at(ix, iy),
        at(ix + 1, iy),
        at(ix, iy + 1),
        at(ix + 1, iy + 1),
    ];
    std::array::from_fn(|c| {
        (p[0][c] * (1.0 - tx) + p[1][c] * tx) * (1.0 - ty)
            + (p[2][c] * (1.0 - tx) + p[3][c] * tx) * ty
    })
}

pub fn apply(
    linear: &[[f32; 4]],
    fine: Option<&[[f16; 4]]>,
    w: u32,
    h: u32,
    edit: &Edit,
) -> Option<Vec<[f32; 4]>> {
    let healing = edit.settings.effective().healing / 100.0;
    if !edit.clones.iter().any(|s| s.strength != 0.0)
        && !edit.patches.iter().any(|s| s.strength != 0.0)
        && (healing == 0.0
            || !edit
                .strokes
                .iter()
                .any(|s| s.target == Target::Heal && !s.erase))
    {
        return None;
    }
    let mut result = linear.to_vec();
    // Each healed patch selects a nearby intact texture whose boundary best matches the destination.
    for stroke in edit
        .strokes
        .iter()
        .filter(|s| healing != 0.0 && s.target == Target::Heal && !s.erase)
    {
        let (cx, cy) = (stroke.center[0] * w as f32, stroke.center[1] * h as f32);
        let radius = (stroke.radius * w.min(h) as f32).max(1.0);
        let mut best = (f32::MAX, [0.0; 2], [0.0; 3]);
        for j in 0..32 {
            let angle = j as f32 * std::f32::consts::TAU / 16.0;
            let offset = [
                (angle.cos() * radius * (if j < 16 { 2.7 } else { 4.2 })).round(),
                (angle.sin() * radius * (if j < 16 { 2.7 } else { 4.2 })).round(),
            ];
            if cx + offset[0] - radius < 0.0
                || cy + offset[1] - radius < 0.0
                || cx + offset[0] + radius >= w as f32
                || cy + offset[1] + radius >= h as f32
            {
                continue;
            }
            let mut correction = [0.0; 3];
            let mut score = 0.0;
            for k in 0..20 {
                let a = k as f32 * std::f32::consts::TAU / 20.0;
                let (x, y) = (cx + a.cos() * radius * 1.1, cy + a.sin() * radius * 1.1);
                let target = fine.map_or_else(
                    || sample(linear, w, h, x, y),
                    |fine| sample_half(fine, w, h, x, y),
                );
                let source = fine.map_or_else(
                    || sample(linear, w, h, x + offset[0], y + offset[1]),
                    |fine| sample_half(fine, w, h, x + offset[0], y + offset[1]),
                );
                for c in 0..3 {
                    correction[c] += (target[c] - source[c]) / 20.0;
                    score += (target[c] - source[c]).powi(2);
                }
            }
            // Discourage copying another isolated dark or bright defect into the healed centre.
            let middle = sample(linear, w, h, cx + offset[0], cy + offset[1]);
            let smooth = fine.map_or_else(
                || sample(linear, w, h, cx + offset[0], cy + offset[1]),
                |fine| sample_half(fine, w, h, cx + offset[0], cy + offset[1]),
            );
            for c in 0..3 {
                score += (middle[c] - smooth[c]).powi(2) * 10.0;
            }
            if score < best.0 {
                best = (score, offset, correction);
            }
        }
        if best.0 == f32::MAX {
            continue;
        }
        stamp(
            &mut result,
            (w, h),
            [cx, cy],
            radius,
            stroke.softness,
            healing,
            |x, y| {
                let mut p = sample(linear, w, h, x + best.1[0], y + best.1[1]);
                for (c, value) in p.iter_mut().enumerate().take(3) {
                    *value += best.2[c];
                }
                Some(p)
            },
        );
    }
    for clone in &edit.clones {
        let center = [clone.center[0] * w as f32, clone.center[1] * h as f32];
        let offset = [
            (clone.source[0] - clone.center[0]) * w as f32,
            (clone.source[1] - clone.center[1]) * h as f32,
        ];
        stamp(
            &mut result,
            (w, h),
            center,
            (clone.radius * w.min(h) as f32).max(0.5),
            clone.softness,
            clone.strength / 100.0,
            |x, y| {
                let (source_x, source_y) = (x + offset[0], y + offset[1]);
                (source_x >= 0.0
                    && source_y >= 0.0
                    && source_x <= (w - 1) as f32
                    && source_y <= (h - 1) as f32)
                    .then(|| sample(linear, w, h, source_x, source_y))
            },
        );
    }
    for patch in &edit.patches {
        apply_patch(&mut result, linear, (w, h), patch);
    }
    Some(result)
}

fn apply_patch(
    result: &mut [[f32; 4]],
    source: &[[f32; 4]],
    dims: (u32, u32),
    patch: &PatchStroke,
) {
    let (w, h) = dims;
    if w == 0 || h == 0 || patch.boundary.len() < 3 || patch.strength <= 0.0 {
        return;
    }
    let boundary: Vec<_> = patch
        .boundary
        .iter()
        .map(|p| [p[0] * w as f32, p[1] * h as f32])
        .collect();
    if boundary.iter().flatten().any(|v| !v.is_finite()) {
        return;
    }
    let min_x = boundary
        .iter()
        .map(|p| p[0])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .clamp(0.0, w as f32) as u32;
    let max_x = boundary
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .clamp(0.0, w as f32) as u32;
    let min_y = boundary
        .iter()
        .map(|p| p[1])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .clamp(0.0, h as f32) as u32;
    let max_y = boundary
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .clamp(0.0, h as f32) as u32;
    if min_x >= max_x || min_y >= max_y {
        return;
    }
    let offset = [patch.offset[0] * w as f32, patch.offset[1] * h as f32];
    let mut correction = [0.0; 3];
    let mut correction_samples = 0.0;
    for i in 0..boundary.len() {
        let a = boundary[i];
        let b = boundary[(i + 1) % boundary.len()];
        for sample_index in 0..8 {
            let t = (sample_index as f32 + 0.5) / 8.0;
            let x = a[0] + (b[0] - a[0]) * t;
            let y = a[1] + (b[1] - a[1]) * t;
            let (source_x, source_y) = (x + offset[0], y + offset[1]);
            if x < 0.0
                || y < 0.0
                || x > (w - 1) as f32
                || y > (h - 1) as f32
                || source_x < 0.0
                || source_y < 0.0
                || source_x > (w - 1) as f32
                || source_y > (h - 1) as f32
            {
                continue;
            }
            let target = sample(source, w, h, x, y);
            let clean = sample(source, w, h, source_x, source_y);
            for c in 0..3 {
                correction[c] += target[c] - clean[c];
            }
            correction_samples += 1.0;
        }
    }
    if correction_samples > 0.0 {
        for value in &mut correction {
            *value = (*value / correction_samples).clamp(-0.15, 0.15);
        }
    }
    let bounds_width = boundary
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max)
        - boundary.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
    let bounds_height = boundary
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max)
        - boundary.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
    let feather = bounds_width.min(bounds_height).max(1.0) * 0.12 * patch.softness.clamp(0.0, 1.0);
    let strength = patch.strength / 100.0;
    for y in min_y..max_y {
        for x in min_x..max_x {
            let point = [x as f32 + 0.5, y as f32 + 0.5];
            if !inside_polygon(point, &boundary) {
                continue;
            }
            let edge_distance = boundary
                .iter()
                .enumerate()
                .map(|(i, a)| distance_to_segment(point, *a, boundary[(i + 1) % boundary.len()]))
                .fold(f32::INFINITY, f32::min);
            let feather_weight = if feather <= 0.0 {
                1.0
            } else {
                let t = (edge_distance / feather).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
            let (source_x, source_y) = (point[0] + offset[0], point[1] + offset[1]);
            if source_x < 0.0
                || source_y < 0.0
                || source_x > (w - 1) as f32
                || source_y > (h - 1) as f32
            {
                continue;
            }
            let mut clean = sample(source, w, h, source_x, source_y);
            for c in 0..3 {
                clean[c] = (clean[c] + correction[c]).clamp(0.0, 1.0);
            }
            let weight = feather_weight * strength.clamp(0.0, 1.0);
            let out = &mut result[(y * w + x) as usize];
            for c in 0..4 {
                out[c] += (clean[c] - out[c]) * weight;
            }
        }
    }
}

fn inside_polygon(point: [f32; 2], boundary: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let mut previous = boundary.len() - 1;
    for current in 0..boundary.len() {
        let a = boundary[current];
        let b = boundary[previous];
        if (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn distance_to_segment(point: [f32; 2], start: [f32; 2], end: [f32; 2]) -> f32 {
    let line = [end[0] - start[0], end[1] - start[1]];
    let length_squared = line[0] * line[0] + line[1] * line[1];
    if length_squared <= f32::EPSILON {
        return (point[0] - start[0]).hypot(point[1] - start[1]);
    }
    let t = (((point[0] - start[0]) * line[0] + (point[1] - start[1]) * line[1]) / length_squared)
        .clamp(0.0, 1.0);
    (point[0] - (start[0] + line[0] * t)).hypot(point[1] - (start[1] + line[1] * t))
}

fn stamp(
    result: &mut [[f32; 4]],
    dims: (u32, u32),
    center: [f32; 2],
    radius: f32,
    softness: f32,
    strength: f32,
    source: impl Fn(f32, f32) -> Option<[f32; 4]>,
) {
    let (w, h) = dims;
    for y in (center[1] - radius).floor().max(0.0) as u32
        ..(center[1] + radius).ceil().min(h as f32) as u32
    {
        for x in (center[0] - radius).floor().max(0.0) as u32
            ..(center[0] + radius).ceil().min(w as f32) as u32
        {
            let weight = brush_weight(
                (x as f32 + 0.5 - center[0]).hypot(y as f32 + 0.5 - center[1]) / radius,
                softness,
            ) * strength.clamp(0.0, 1.0);
            if weight == 0.0 {
                continue;
            }
            let Some(p) = source(x as f32, y as f32) else {
                continue;
            };
            let out = &mut result[(y * w + x) as usize];
            for c in 0..4 {
                out[c] += (p[c] - out[c]) * weight;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn linear_pixels(source: &RgbaImage) -> Vec<[f32; 4]> {
        source
            .pixels()
            .map(|p| {
                [
                    crate::engine::linear_byte(p[0]),
                    crate::engine::linear_byte(p[1]),
                    crate::engine::linear_byte(p[2]),
                    p[3] as f32 / 255.0,
                ]
            })
            .collect()
    }
    #[test]
    fn tiled_cleanup_matches_whole_image_for_overlapping_tools_and_remote_donors() {
        let (w, h) = (97, 83);
        let source = RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([
                ((x * 3 + y * 7) % 230) as u8,
                ((x * 5 + y * 11) % 215) as u8,
                ((x * 13 + y * 2) % 240) as u8,
                (160 + (x + y) % 95) as u8,
            ])
        });
        let linear = linear_pixels(&source);
        let fine: Vec<_> = linear.iter().map(|p| p.map(f16::from_f32)).collect();
        let mut edit = Edit::default();
        for center in [[0.25, 0.27], [0.45, 0.5], [0.02, 0.4]] {
            edit.strokes.push(crate::engine::Stroke {
                target: Target::Heal,
                center,
                radius: 0.045,
                erase: false,
                softness: 0.73,
            });
        }
        edit.clones.push(CloneStamp {
            center: [0.26, 0.29],
            source: [0.9, 0.8],
            radius: 0.11,
            softness: 0.62,
            strength: 74.0,
        });
        edit.clones.push(CloneStamp {
            center: [0.98, 0.8],
            source: [0.01, 0.5],
            radius: 0.11,
            softness: 0.0,
            strength: 100.0,
        });
        edit.patches.push(PatchStroke {
            boundary: vec![[0.15, 0.18], [0.43, 0.2], [0.34, 0.47], [0.17, 0.41]],
            offset: [0.47, 0.42],
            softness: 0.81,
            strength: 88.0,
        });
        let expected = apply(&linear, Some(&fine), w, h, &edit).unwrap();
        let prepared =
            PreparedCleanup::prepare(&source, &edit, |x, y| sample_half(&fine, w, h, x, y))
                .unwrap();
        for y in (0..h).step_by(13) {
            for x in (0..w).step_by(17) {
                let crop = crate::engine::Crop {
                    x,
                    y,
                    width: 17.min(w - x),
                    height: 13.min(h - y),
                };
                let mut region = Vec::new();
                let mut comparison = Vec::new();
                for row in y..y + crop.height {
                    region.extend_from_slice(
                        &linear[(row * w + x) as usize..(row * w + x + crop.width) as usize],
                    );
                    comparison.extend_from_slice(
                        &expected[(row * w + x) as usize..(row * w + x + crop.width) as usize],
                    );
                }
                let result = prepared
                    .apply_region(&source, &region, crop)
                    .unwrap_or(region);
                assert_eq!(
                    result, comparison,
                    "Global cleanup registration changed in tile {crop:?}"
                );
            }
        }
    }
    #[test]
    fn prepared_cleanup_matches_no_blur_fallback_and_leaves_unrelated_tiles_unallocated() {
        let source = RgbaImage::from_fn(80, 60, |x, y| {
            image::Rgba([(x * 2) as u8, (y * 3) as u8, ((x + y) * 2) as u8, 255])
        });
        let linear = linear_pixels(&source);
        let mut edit = Edit::default();
        edit.strokes.push(crate::engine::Stroke {
            target: Target::Heal,
            center: [0.55, 0.55],
            radius: 0.035,
            erase: false,
            softness: 0.5,
        });
        let prepared =
            PreparedCleanup::prepare(&source, &edit, |x, y| sample_image(&source, x, y)).unwrap();
        let whole = crate::engine::Crop {
            x: 0,
            y: 0,
            width: 80,
            height: 60,
        };
        assert_eq!(
            prepared.apply_region(&source, &linear, whole).unwrap(),
            apply(&linear, None, 80, 60, &edit).unwrap()
        );
        let unrelated = crate::engine::Crop {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        };
        assert!(
            prepared
                .apply_region(&source, &[[0.; 4]; 4], unrelated)
                .is_none()
        );
    }
    #[test]
    fn cleanup_preparation_cancels_between_donor_candidates() {
        let source = RgbaImage::from_pixel(100, 100, image::Rgba([150, 100, 90, 255]));
        let mut edit = Edit::default();
        edit.strokes.push(crate::engine::Stroke {
            target: Target::Heal,
            center: [0.5, 0.5],
            radius: 0.05,
            erase: false,
            softness: 0.65,
        });
        let cancel = AtomicBool::new(false);
        let calls = std::cell::Cell::new(0);
        let result = PreparedCleanup::prepare_cancellable(
            &source,
            &edit,
            |x, y| {
                calls.set(calls.get() + 1);
                if calls.get() == 25 {
                    cancel.store(true, Ordering::Relaxed);
                }
                sample_image(&source, x, y)
            },
            Some(&cancel),
        );
        assert!(result.is_err());
        assert!(
            calls.get() < 50,
            "Donor search should stop at the next candidate"
        );
    }
    #[test]
    fn healing_replaces_dark_spot_with_intact_texture_not_a_flat_fill() {
        let mut pixels: Vec<_> = (0..10000)
            .map(|i| {
                let t = ((i % 100 + i / 100) % 2) as f32 * 0.03;
                [0.45 + t, 0.3 + t, 0.2 + t, 1.0]
            })
            .collect();
        let fine = vec![[0.465, 0.315, 0.215, 1.0].map(f16::from_f32); 10000];
        for y in 48..53 {
            for x in 48..53 {
                pixels[y * 100 + x] = [0.02, 0.01, 0.01, 1.0];
            }
        }
        let mut edit = Edit::default();
        edit.strokes.push(crate::engine::Stroke {
            target: Target::Heal,
            center: [0.5; 2],
            radius: 0.07,
            erase: false,
            softness: 0.5,
        });
        let result = apply(&pixels, Some(&fine), 100, 100, &edit).unwrap();
        assert!(result[5050][0] > 0.4);
        assert!((result[5050][0] - result[5051][0]).abs() > 0.02);
        assert_eq!(result[0], pixels[0]);
    }
    #[test]
    fn clone_samples_selected_pixels_and_zero_strength_preserves_source() {
        let pixels: Vec<_> = (0..10000)
            .map(|i| [i as f32 % 100.0 / 100.0, 0.0, 0.0, 1.0])
            .collect();
        let mut edit = Edit::default();
        edit.clones.push(CloneStamp {
            center: [0.7, 0.5],
            source: [0.2, 0.5],
            radius: 0.1,
            softness: 0.4,
            strength: 100.0,
        });
        let result = apply(&pixels, None, 100, 100, &edit).unwrap();
        assert!((result[5070][0] - 0.2).abs() < 0.0001);
        edit.clones[0].strength = 0.0;
        assert_eq!(
            apply(&pixels, None, 100, 100, &edit).unwrap_or_else(|| pixels.clone()),
            pixels
        );
    }

    #[test]
    fn patch_transfers_clean_source_texture_with_a_soft_boundary_and_preserves_outside() {
        let (w, h) = (40, 40);
        let mut pixels = vec![[0.2, 0.2, 0.2, 1.0]; (w * h) as usize];
        for y in 10..18 {
            for x in 24..32 {
                pixels[(y * w + x) as usize] = [0.8, 0.8, 0.8, 1.0];
            }
        }
        let mut edit = Edit::default();
        edit.patches.push(PatchStroke {
            boundary: vec![[0.25, 0.25], [0.45, 0.25], [0.45, 0.45], [0.25, 0.45]],
            offset: [0.35, 0.0],
            softness: 0.0,
            strength: 100.0,
        });

        let result = apply(&pixels, None, w, h, &edit).unwrap();
        assert!(result[(13 * w + 13) as usize][0] > 0.6);
        assert_eq!(result[(5 * w + 5) as usize], pixels[(5 * w + 5) as usize]);
    }
}
