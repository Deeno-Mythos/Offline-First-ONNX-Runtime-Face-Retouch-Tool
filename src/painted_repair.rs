//! Source-registered, brush-sized repairs. Automatic flyaway detection stays separate.
use crate::{
    engine::{Stroke, Target, brush_weight, linear_byte},
    flyaway::FlyawayMap,
};
use anyhow::{Result, bail};
use image::RgbaImage;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

// Small summed-area tiles make wide shading samples cheap without allocating a
// full-resolution float image. Every tile reads the same original coordinates.
struct FabricTile {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    sums: Vec<[u32; 4]>,
}
impl FabricTile {
    fn new(image: &RgbaImage, tx: u32, ty: u32, halo: u32) -> Self {
        let x = (tx * 128).saturating_sub(halo);
        let y = (ty * 128).saturating_sub(halo);
        let right = ((tx + 1) * 128 + halo).min(image.width());
        let bottom = ((ty + 1) * 128 + halo).min(image.height());
        let (width, height) = (right - x, bottom - y);
        let stride = (width + 1) as usize;
        let mut sums = vec![[0; 4]; stride * (height + 1) as usize];
        for iy in 0..height {
            let mut row = [0; 4];
            for ix in 0..width {
                let p = image.get_pixel(x + ix, y + iy);
                let i = (iy as usize + 1) * stride + ix as usize + 1;
                for c in 0..4 {
                    row[c] += if p[3] < 250 {
                        0
                    } else if c == 3 {
                        1
                    } else {
                        p[c] as u32
                    };
                    sums[i][c] = sums[i - stride][c] + row[c];
                }
            }
        }
        Self {
            x,
            y,
            width,
            height,
            sums,
        }
    }
    fn mean(&self, x: f32, y: f32, r: f32) -> Option<[f32; 3]> {
        let x = x.clamp(self.x as f32, (self.x + self.width - 1) as f32);
        let y = y.clamp(self.y as f32, (self.y + self.height - 1) as f32);
        let left = (x - r).floor().max(self.x as f32) as u32 - self.x;
        let top = (y - r).floor().max(self.y as f32) as u32 - self.y;
        let right = ((x + r).ceil() as u32 + 1).min(self.x + self.width) - self.x;
        let bottom = ((y + r).ceil() as u32 + 1).min(self.y + self.height) - self.y;
        let stride = (self.width + 1) as usize;
        let at = |x, y| self.sums[(y as usize) * stride + x as usize];
        let (a, b, c, d) = (
            at(left, top),
            at(right, top),
            at(left, bottom),
            at(right, bottom),
        );
        let count = (d[3] + a[3]) - (b[3] + c[3]);
        (count > 0)
            .then(|| std::array::from_fn(|i| ((d[i] + a[i]) - (b[i] + c[i])) as f32 / count as f32))
    }
    fn correction(&self, image: &RgbaImage, x: u32, y: u32, radius: f32) -> [f32; 3] {
        let weave = (image.width().min(image.height()) as f32 * 0.0006)
            .round()
            .clamp(1., 8.);
        let Some(fine) = self.mean(x as f32, y as f32, weave) else {
            return [0.; 3];
        };
        let chroma = |p: [f32; 3]| p.map(|v| v / luma(p).max(8.));
        let reference = chroma(fine);
        let radius = radius.clamp(6., 120.);
        let mut sum = [0.; 3];
        let mut total = 0.;
        for j in -1_i32..=1 {
            for i in -1_i32..=1 {
                let dx = x as f32 + i as f32 * radius * 0.65;
                let dy = y as f32 + j as f32 * radius * 0.65;
                let r = radius * 0.35;
                let Some(donor) = self.mean(dx, dy, r) else {
                    continue;
                };
                // A box spanning a colored print is not a clean fabric donor,
                // even when its averaged color resembles the surrounding cloth.
                let donor_chroma = chroma(donor);
                let mut variance = 0_f32;
                for sy in [-0.5, 0.5] {
                    for sx in [-0.5, 0.5] {
                        if let Some(part) = self.mean(dx + sx * r, dy + sy * r, r * 0.5) {
                            variance = variance.max(distance(donor_chroma, chroma(part)));
                        }
                    }
                }
                if let Some(center) = self.mean(dx, dy, weave) {
                    variance = variance.max(distance(donor_chroma, chroma(center)));
                }
                let coherent = (-(variance / 0.06).powi(4)).exp();
                let difference = distance(reference, chroma(donor));
                let weight = (-(difference / 0.1).powi(2)).exp()
                    * coherent
                    * if i == 0 { 2. } else { 1. }
                    * if j == 0 { 2. } else { 1. };
                total += weight;
                for c in 0..3 {
                    sum[c] += donor[c] * weight;
                }
            }
        }
        if total < 0.000001 {
            return [0.; 3];
        }
        let target = sum.map(|v| v / total);
        // Low-confidence color samples fade toward the untouched source instead
        // of switching the correction off at a hard per-pixel threshold.
        let confidence = (total / 16.).clamp(0., 1.);
        let gain =
            (linear_float(luma(target)) / linear_float(luma(fine)).max(0.001)).clamp(0.25, 4.);
        let p = image.get_pixel(x, y);
        std::array::from_fn(|c| linear_byte(p[c]) * (gain - 1.) * 0.95 * confidence)
    }
}

const NORMALS: [[f32; 2]; 8] = [
    [1., 0.],
    [0.92388, 0.38268],
    [
        std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
    ],
    [0.38268, 0.92388],
    [0., 1.],
    [-0.38268, 0.92388],
    [
        -std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
    ],
    [-0.92388, 0.38268],
];

fn sample(image: &RgbaImage, x: f32, y: f32) -> Option<[f32; 3]> {
    let (x, y) = (x.round() as i32, y.round() as i32);
    if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
        return None;
    }
    let p = image.get_pixel(x as u32, y as u32);
    (p[3] >= 250).then(|| [p[0] as f32, p[1] as f32, p[2] as f32])
}
fn luma(p: [f32; 3]) -> f32 {
    p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|c| (a[c] - b[c]).abs()).fold(0., f32::max)
}
fn mean(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|c| (a[c] + b[c]) * 0.5)
}
fn polarity(image: &RgbaImage, stroke: &Stroke) -> f32 {
    let x = stroke.center[0] * image.width() as f32;
    let y = stroke.center[1] * image.height() as f32;
    let radius = (stroke.radius * image.width().min(image.height()) as f32 * 0.8).clamp(3., 48.);
    let Some(center) = sample(image, x, y) else {
        return 1.;
    };
    let mut ring: Vec<_> = NORMALS
        .iter()
        .flat_map(|[nx, ny]| {
            [
                sample(image, x + nx * radius, y + ny * radius),
                sample(image, x - nx * radius, y - ny * radius),
            ]
        })
        .flatten()
        .map(luma)
        .collect();
    ring.sort_by(f32::total_cmp);
    // Infer the strand's polarity at the traced center. Do not mistake the
    // bright gaps between dark hairs for pale hairs and darken those gaps.
    if ring
        .get(ring.len() / 2)
        .is_some_and(|level| *level < luma(center) - 4.)
    {
        -1.
    } else {
        1.
    }
}

/// Only pixels covered by paint are stored; donors may lie beyond the brush edge.
/// Sampling distances follow brush width at the original resolution, including native tiles.
pub(crate) fn selected(
    image: &RgbaImage,
    strokes: &[Stroke],
    target: Target,
    cancel: Option<&AtomicBool>,
) -> Result<FlyawayMap> {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return Ok(FlyawayMap::default());
    }
    let strokes: Vec<_> = strokes.iter().filter(|s| s.target == target).collect();
    let short = w.min(h) as f32;
    let fabric = target == Target::Clothing;
    let polarities: Vec<_> = strokes
        .iter()
        .map(|s| if fabric { 1. } else { polarity(image, s) })
        .collect();
    let mut fabric_tiles = HashMap::new();
    if fabric {
        let mut needed: HashMap<(u32, u32), u32> = HashMap::new();
        for s in strokes.iter().filter(|s| !s.erase && s.strength > 0.) {
            let r = s.radius * short;
            let halo = r.clamp(6., 120.).ceil() as u32 + 10;
            let x0 = (s.center[0] * w as f32 - r).max(0.) as u32 / 128;
            let x1 = ((s.center[0] * w as f32 + r).max(0.) as u32).min(w - 1) / 128;
            let y0 = (s.center[1] * h as f32 - r).max(0.) as u32 / 128;
            let y1 = ((s.center[1] * h as f32 + r).max(0.) as u32).min(h - 1) / 128;
            for ty in y0..=y1 {
                for tx in x0..=x1 {
                    needed
                        .entry((tx, ty))
                        .and_modify(|old| *old = (*old).max(halo))
                        .or_insert(halo);
                }
            }
        }
        for ((tx, ty), halo) in needed {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                bail!("Fabric repair cancelled");
            }
            fabric_tiles.insert((tx, ty), FabricTile::new(image, tx, ty, halo));
        }
    }
    let rows = (0..h)
        .into_par_iter()
        .map(|y| {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return vec![];
            }
            let active: Vec<_> = strokes
                .iter()
                .zip(&polarities)
                .filter(|(s, _)| (y as f32 + 0.5 - s.center[1] * h as f32).abs() < s.radius * short)
                .collect();
            let mut spans: Vec<_> = active
                .iter()
                .map(|(s, _)| {
                    let r = s.radius * short;
                    (
                        (s.center[0] * w as f32 - r).floor().max(0.) as u32,
                        (s.center[0] * w as f32 + r).ceil().min(w as f32) as u32,
                    )
                })
                .collect();
            spans.sort_unstable();
            let mut merged: Vec<(u32, u32)> = vec![];
            for (a, b) in spans {
                if let Some(last) = merged.last_mut().filter(|last| a <= last.1) {
                    last.1 = last.1.max(b);
                } else {
                    merged.push((a, b));
                }
            }
            let mut repaired = vec![];
            for (a, b) in merged {
                for x in a..b {
                    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                        break;
                    }
                    let mut amount = 0.;
                    let mut radius: f32 = 0.;
                    let mut nearest = f32::MAX;
                    let mut pixel_polarity = 1.;
                    for (s, polarity) in &active {
                        let r = (s.radius * short).max(0.001);
                        let d = (x as f32 + 0.5 - s.center[0] * w as f32)
                            .hypot(y as f32 + 0.5 - s.center[1] * h as f32)
                            / r;
                        let weight =
                            brush_weight(d, s.softness) * (s.strength / 100.).clamp(0., 1.);
                        amount += (if s.erase { 0. } else { 1. } - amount) * weight;
                        if !s.erase && weight > 0. {
                            radius = radius.max(r);
                            if d < nearest {
                                nearest = d;
                                pixel_polarity = **polarity;
                            }
                        }
                    }
                    if amount < 0.001 {
                        continue;
                    }
                    if image.get_pixel(x, y)[3] < 250 {
                        continue;
                    }
                    let correction = if fabric {
                        fabric_tiles
                            .get(&(x / 128, y / 128))
                            .map(|tile| tile.correction(image, x, y, radius))
                    } else {
                        repair(image, x as f32, y as f32, radius, pixel_polarity)
                    };
                    if let Some(delta) = correction.filter(|d| d.iter().any(|v| v.abs() > 0.000001))
                    {
                        repaired.push((x, delta.map(|v| v * amount)));
                    }
                }
            }
            repaired
        })
        .collect();
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        bail!("Painted repair cancelled");
    }
    let raw = FlyawayMap::from_rows(w as usize, rows);
    if fabric {
        Ok(raw)
    } else {
        finish_hair(image, &strokes, &polarities, raw, cancel)
    }
}

/// Bridge small gaps along an accepted strand using repaired neighbors. This
/// avoids a dotted remnant where curvature/anti-aliasing breaks ridge detection.
/// Two-sided support keeps nearby isolated marks out of the repair.
fn finish_hair(
    image: &RgbaImage,
    strokes: &[&Stroke],
    polarities: &[f32],
    raw: FlyawayMap,
    cancel: Option<&AtomicBool>,
) -> Result<FlyawayMap> {
    let (w, h) = image.dimensions();
    let short = w.min(h) as f32;
    let coverage = |x: i32, y: i32| {
        let mut amount = 0.;
        for s in strokes {
            let d = (x as f32 + 0.5 - s.center[0] * w as f32)
                .hypot(y as f32 + 0.5 - s.center[1] * h as f32)
                / (s.radius * short).max(0.001);
            let weight = brush_weight(d, s.softness) * (s.strength / 100.).clamp(0., 1.);
            amount += (if s.erase { 0. } else { 1. } - amount) * weight;
        }
        amount
    };
    let reference = |x: i32, y: i32| {
        let p = sample(image, x as f32, y as f32)?;
        let delta = raw.delta((y as u32 * w + x as u32) as usize);
        if delta == [0.; 3] {
            return None;
        }
        let alpha = coverage(x, y);
        (alpha > 0.05).then(|| std::array::from_fn(|c| linear_float(p[c]) + delta[c] / alpha))
    };
    let rows = (0..h)
        .into_par_iter()
        .map(|y| {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return vec![];
            }
            let active: Vec<_> = strokes
                .iter()
                .zip(polarities)
                .filter(|(s, _)| (y as f32 + 0.5 - s.center[1] * h as f32).abs() < s.radius * short)
                .collect();
            let x0 = active
                .iter()
                .map(|(s, _)| (s.center[0] * w as f32 - s.radius * short).floor().max(0.) as u32)
                .min()
                .unwrap_or(0);
            let x1 = active
                .iter()
                .map(|(s, _)| {
                    (s.center[0] * w as f32 + s.radius * short)
                        .ceil()
                        .min(w as f32) as u32
                })
                .max()
                .unwrap_or(0);
            let mut row = vec![];
            for x in x0..x1 {
                if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                    break;
                }
                let delta = raw.delta((y * w + x) as usize);
                if delta != [0.; 3] {
                    row.push((x, delta));
                    continue;
                }
                let amount = coverage(x as i32, y as i32);
                if amount < 0.001 {
                    continue;
                }
                let Some(p) = sample(image, x as f32, y as f32) else {
                    continue;
                };
                let sign = active
                    .iter()
                    .filter(|(s, _)| !s.erase && s.strength > 0.)
                    .min_by(|(a, _), (b, _)| {
                        let d = |s: &Stroke| {
                            (x as f32 + 0.5 - s.center[0] * w as f32)
                                .hypot(y as f32 + 0.5 - s.center[1] * h as f32)
                                / (s.radius * short).max(0.001)
                        };
                        d(a).total_cmp(&d(b))
                    })
                    .map_or(1., |(_, p)| **p);
                let mut repair = None;
                'directions: for step in 1..=4 {
                    for [nx, ny] in NORMALS {
                        let dx = (nx * step as f32).round() as i32;
                        let dy = (ny * step as f32).round() as i32;
                        let Some(a) = reference(x as i32 + dx, y as i32 + dy) else {
                            continue;
                        };
                        let Some(b) = reference(x as i32 - dx, y as i32 - dy) else {
                            continue;
                        };
                        if distance(a, b) > 0.06 {
                            continue;
                        }
                        let target = mean(a, b);
                        let original = p.map(linear_float);
                        if (luma(target) - luma(original)) * sign < 0.005 {
                            continue;
                        }
                        repair = Some(std::array::from_fn(|c| (target[c] - original[c]) * amount));
                        break 'directions;
                    }
                }
                if let Some(delta) = repair {
                    row.push((x, delta));
                }
            }
            row
        })
        .collect();
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        bail!("Strand repair cancelled");
    }
    Ok(FlyawayMap::from_rows(w as usize, rows))
}

fn repair(image: &RgbaImage, x: f32, y: f32, radius: f32, polarity: f32) -> Option<[f32; 3]> {
    let original = sample(image, x, y)?;
    let at = |x, y| sample(image, x, y);
    let center = at(x, y)?;
    let level = luma(center);
    let maximum = (radius * 1.2)
        .clamp(3., 48.)
        .min((image.width().min(image.height()) as f32 * 0.02).max(4.));
    let mut best = None;
    // Geometric spacing covers anti-aliased hairs through wide fabric creases.
    let mut step = 1.5;
    while step <= maximum {
        for [nx, ny] in NORMALS {
            let Some(a) = at(x + nx * step, y + ny * step) else {
                continue;
            };
            let Some(b) = at(x - nx * step, y - ny * step) else {
                continue;
            };
            let donor = mean(a, b);
            let contrast = luma(donor) - level;
            if contrast * polarity < 3. {
                continue;
            }
            // A crease/strand has matching material on both sides; an edge does not.
            if [a, b]
                .iter()
                .any(|p| (luma(*p) - level) * polarity < contrast.abs() * 0.3)
            {
                continue;
            }
            if distance(a, b) > (12. + contrast.abs() * 0.45).min(65.) {
                continue;
            }
            let Some(outer_a) = at(x + nx * step * 1.65, y + ny * step * 1.65) else {
                continue;
            };
            let Some(outer_b) = at(x - nx * step * 1.65, y - ny * step * 1.65) else {
                continue;
            };
            if distance(mean(outer_a, outer_b), donor) > (5. + contrast.abs() * 0.35) {
                continue;
            }
            let along = (step * 1.3).clamp(3., 8.);
            let Some(t1) = at(x - ny * along, y + nx * along) else {
                continue;
            };
            let Some(t2) = at(x + ny * along, y - nx * along) else {
                continue;
            };
            if [t1, t2]
                .iter()
                .any(|p| (luma(donor) - luma(*p)) * polarity < contrast.abs() * 0.35)
            {
                continue;
            }
            let score = contrast.abs() - distance(a, b) * 0.25;
            if best.as_ref().is_none_or(|(old, _, _)| score > *old) {
                best = Some((score, donor, contrast));
            }
        }
        step *= 1.55;
    }
    best.map(|(_, donor, contrast)| {
        let confidence = (contrast.abs() / 7.).min(1.);
        std::array::from_fn(|c| (linear_float(donor[c]) - linear_float(original[c])) * confidence)
    })
}
fn linear_float(value: f32) -> f32 {
    let lo = value.floor().clamp(0., 255.) as u8;
    let hi = lo.saturating_add(1);
    let a = linear_byte(lo);
    a + (linear_byte(hi) - a) * value.fract()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{self, Crop, Edit, Renderer};
    use image::Rgba;
    use std::sync::Arc;

    fn stroke(target: Target, x: f32, y: f32, radius: f32, size: u32) -> Stroke {
        Stroke {
            target,
            center: [x / size as f32, y / size as f32],
            radius: radius / size as f32,
            strength: 100.,
            softness: 0.4,
            erase: false,
        }
    }
    #[test]
    fn curved_antialiased_hairs_on_textured_gradients_are_removed_at_multiple_scales() {
        for (size, width) in [(256, 0.7), (512, 2.), (1024, 4.)] {
            for contrast in [-18., 30.] {
                let clean = RgbaImage::from_fn(size, size, |x, y| {
                    let grain = ((x * 13 + y * 7) % 5) as f32 - 2.;
                    let base =
                        150. + x as f32 / size as f32 * 20. + y as f32 / size as f32 * 10. + grain;
                    Rgba([base as u8, (base + 13.) as u8, (base + 25.) as u8, 255])
                });
                let line =
                    |y: f32| size as f32 * 0.5 + (y / size as f32 * 9.).sin() * size as f32 * 0.025;
                let damaged = RgbaImage::from_fn(size, size, |x, y| {
                    let weight = (-0.5 * ((x as f32 - line(y as f32)) / width).powi(2)).exp();
                    let mut p = *clean.get_pixel(x, y);
                    for v in &mut p.0[..3] {
                        *v = (*v as f32 + contrast * weight).round() as u8;
                    }
                    p
                });
                let radius = (width * 4.).max(5.);
                let strokes: Vec<_> = (size / 4..size * 3 / 4)
                    .step_by(radius as usize / 2)
                    .map(|y| stroke(Target::StrayHair, line(y as f32), y as f32, radius, size))
                    .collect();
                let edit = Edit {
                    strokes: strokes.into(),
                    ..Default::default()
                };
                let out = engine::render(&damaged, &edit, None);
                let mut before = 0.;
                let mut after = 0.;
                for y in size / 3..size * 2 / 3 {
                    let x = line(y as f32).round() as u32;
                    before += damaged.get_pixel(x, y)[0].abs_diff(clean.get_pixel(x, y)[0]) as f64;
                    after += out.get_pixel(x, y)[0].abs_diff(clean.get_pixel(x, y)[0]) as f64;
                }
                assert!(
                    after < before * 0.4,
                    "{size}px width {width} contrast {contrast}: error {before} -> {after}"
                );
                println!(
                    "Hair {size}px / width {width} / contrast {contrast}: centerline error reduced {:.1}%",
                    (1. - after / before) * 100.
                );
                assert_eq!(out.get_pixel(0, 0), clean.get_pixel(0, 0));
            }
        }
    }
    #[test]
    fn broad_fabric_folds_recover_shading_without_blurring_weave_and_match_native_tiles() {
        let size = 256;
        for amplitude in [-0.45, 0.3] {
            let damaged = Arc::new(RgbaImage::from_fn(size, size, |x, y| {
                let fold = (-0.5 * ((x as f32 - 128.) / 12.).powi(2)).exp() * amplitude;
                let weave = if (x + y) % 2 == 0 { 3. } else { -3. };
                let gain = 1. + fold;
                Rgba([
                    (100. * gain + weave) as u8,
                    (140. * gain + weave) as u8,
                    (180. * gain + weave) as u8,
                    255,
                ])
            }));
            let paint = stroke(Target::Clothing, 128., 128., 72., size);
            let mut edit = Edit {
                strokes: vec![paint.clone()].into(),
                ..Default::default()
            };
            let mut renderer = Renderer::default();
            let out = renderer.render(&damaged, &edit, None, None).unwrap();
            let before = (damaged.get_pixel(128, 128)[1] as f32 - 143.).abs();
            let after = (out.get_pixel(128, 128)[1] as f32 - 143.).abs();
            assert!(
                after < before * 0.4,
                "fabric amplitude {amplitude}: error {before} -> {after}"
            );
            println!("Fabric amplitude {amplitude}: crease error {before} -> {after}");
            let weave = out.get_pixel(128, 128)[1] as f32 - out.get_pixel(128, 129)[1] as f32;
            assert!(weave >= 4., "fabric weave lost: {weave}");
            assert_eq!(out.get_pixel(128, 20), damaged.get_pixel(128, 20));
            let crop = Crop {
                x: 90,
                y: 85,
                width: 80,
                height: 83,
            };
            assert_eq!(
                renderer
                    .render_region(&damaged, &edit, None, crop, None)
                    .unwrap(),
                image::imageops::crop_imm(&out, crop.x, crop.y, crop.width, crop.height).to_image()
            );
            let mut erase = paint;
            erase.erase = true;
            erase.softness = 0.;
            edit.strokes.push(erase);
            assert!(renderer.render(&damaged, &edit, None, None).unwrap() == *damaged);
        }
    }

    #[test]
    fn printed_color_boundaries_transparency_and_zero_strength_stay_intact() {
        let image = RgbaImage::from_fn(160, 160, |x, _| {
            if (77..84).contains(&x) {
                Rgba([180, 45, 60, 255])
            } else {
                Rgba([170, 180, 190, 255])
            }
        });
        let paint = stroke(Target::Clothing, 80., 80., 35., 160);
        let edit = Edit {
            strokes: vec![paint.clone()].into(),
            ..Default::default()
        };
        let out = engine::render(&image, &edit, None);
        for (x, y, p) in out.enumerate_pixels() {
            let original = image.get_pixel(x, y);
            if (77..84).contains(&x) {
                assert_eq!(p, original, "printed color stripe changed");
            } else {
                assert!(
                    (0..4).all(|c| p[c].abs_diff(original[c]) <= 2),
                    "halo beside printed color stripe"
                );
            }
        }
        let mut transparent = image.clone();
        for p in transparent.pixels_mut() {
            p[3] = 120;
        }
        assert_eq!(
            selected(
                &transparent,
                std::slice::from_ref(&paint),
                Target::Clothing,
                None
            )
            .unwrap()
            .correction_count(),
            0
        );
        let mut zero = paint;
        zero.strength = 0.;
        assert_eq!(
            selected(&image, &[zero], Target::Clothing, None)
                .unwrap()
                .correction_count(),
            0
        );
        let cancel = AtomicBool::new(true);
        assert!(selected(&image, &[], Target::StrayHair, Some(&cancel)).is_err());
    }

    #[test]
    fn fabric_samples_stay_registered_across_tiles_and_image_edges() {
        let image = Arc::new(RgbaImage::from_fn(2200, 1600, |x, y| {
            let crease = if x % 128 < 9 { 35 } else { 0 };
            let grain = ((x + y) % 2) * 4;
            Rgba([
                (130 - crease + grain) as u8,
                (160 - crease + grain) as u8,
                (190 - crease + grain) as u8,
                if y < 10 { 0 } else { 255 },
            ])
        }));
        let mut edit = Edit::default();
        for [x, y] in [[0., 0.], [2199., 1599.], [128., 128.]] {
            edit.strokes.push(Stroke {
                target: Target::Clothing,
                center: [x / 2200., y / 1600.],
                radius: 0.03,
                softness: 0.5,
                strength: 100.,
                erase: false,
            });
        }
        let full = engine::render(&image, &edit, None);
        let mut renderer = Renderer::default();
        for crop in [
            Crop {
                x: 0,
                y: 0,
                width: 200,
                height: 200,
            },
            Crop {
                x: 2100,
                y: 1500,
                width: 100,
                height: 100,
            },
        ] {
            let tile = renderer
                .render_region(&image, &edit, None, crop, None)
                .unwrap();
            assert!(
                tile == image::imageops::crop_imm(&full, crop.x, crop.y, crop.width, crop.height)
                    .to_image()
            );
        }
        assert_eq!(full.get_pixel(2, 2), image.get_pixel(2, 2));
    }
}
