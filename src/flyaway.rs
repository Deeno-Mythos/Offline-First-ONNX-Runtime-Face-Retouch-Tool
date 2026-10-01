//! Conservative thin-strand repair around detected heads, outside facial skin.
//! No full-image float buffers are allocated; only accepted strand pixels are stored.
use crate::{
    engine::{Segmentation, linear_byte},
    geometry::FaceMesh,
};
use anyhow::{Result, bail};
use image::RgbaImage;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Default)]
pub struct FlyawayMap {
    width: usize,
    rows: Vec<usize>,
    pixels: Vec<Correction>,
}
#[derive(Clone)]
struct Correction {
    x: u32,
    delta: [f32; 3],
}
impl FlyawayMap {
    /// Sparse lookup in the source image's coordinate space, before any deformation.
    pub fn delta(&self, index: usize) -> [f32; 3] {
        if self.width == 0 {
            return [0.0; 3];
        }
        let y = index / self.width;
        let Some((&start, &end)) = self.rows.get(y).zip(self.rows.get(y + 1)) else {
            return [0.0; 3];
        };
        let row = &self.pixels[start..end];
        row.binary_search_by_key(&((index % self.width) as u32), |p| p.x)
            .map_or([0.0; 3], |i| row[i].delta)
    }
    pub fn correction_count(&self) -> usize {
        self.pixels.len()
    }
    pub fn bytes(&self) -> usize {
        self.rows.len() * std::mem::size_of::<usize>()
            + self.pixels.len() * std::mem::size_of::<Correction>()
    }
}

struct Head {
    rect: [i32; 4],
    face: Vec<[f32; 2]>,
    bounds: [f32; 4],
    width: f32,
    step: i32,
    eligible: bool,
}
impl Head {
    fn new(face: &FaceMesh, w: u32, h: u32) -> Self {
        let [x, y, fw, fh] = face.bounds;
        let width = fw * w as f32;
        const OVAL: [usize; 36] = [
            10, 338, 297, 332, 284, 251, 389, 356, 454, 323, 361, 288, 397, 365, 379, 378, 400,
            377, 152, 148, 176, 149, 150, 136, 172, 58, 132, 93, 234, 127, 162, 21, 54, 103, 67,
            109,
        ];
        let polygon = OVAL
            .iter()
            .filter_map(|&i| face.landmarks.get(i))
            .map(|p| [p[0] * w as f32, p[1] * h as f32])
            .collect();
        Self {
            rect: [
                ((x - fw * 0.65) * w as f32).floor().max(0.0) as i32,
                ((y - fh * 0.85) * h as f32).floor().max(0.0) as i32,
                ((x + fw * 1.65) * w as f32).ceil().min(w as f32) as i32,
                ((y + fh * 0.85) * h as f32).ceil().min(h as f32) as i32,
            ],
            face: polygon,
            bounds: [x * w as f32, y * h as f32, fw * w as f32, fh * h as f32],
            width,
            step: (width * 0.009).round().clamp(2.0, 12.0) as i32,
            eligible: face.confidence >= 0.5,
        }
    }
    fn facial_skin(&self, x: i32, y: i32) -> bool {
        let p = [x as f32 + 0.5, y as f32 + 0.5];
        // Include a small safety margin at the facial contour, where wisps overlap skin.
        if self.face.len() < 3 {
            let [fx, fy, fw, fh] = self.bounds;
            return p[0] >= fx - fw * 0.06
                && p[0] <= fx + fw * 1.06
                && p[1] >= fy - fh * 0.06
                && p[1] <= fy + fh * 1.06;
        }
        let mut inside = false;
        let margin = self.width * 0.045;
        for (a, b) in self
            .face
            .iter()
            .zip(self.face.iter().cycle().skip(1))
            .take(self.face.len())
        {
            if (a[1] > p[1]) != (b[1] > p[1])
                && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
            {
                inside = !inside;
            }
            let d = [b[0] - a[0], b[1] - a[1]];
            let t = ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1])
                / (d[0] * d[0] + d[1] * d[1]).max(1.0);
            let t = t.clamp(0.0, 1.0);
            if (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1]) < margin {
                return true;
            }
        }
        inside
    }
}
fn rgb(image: &RgbaImage, x: i32, y: i32) -> [f32; 3] {
    let p = image.get_pixel(
        x.clamp(0, image.width() as i32 - 1) as u32,
        y.clamp(0, image.height() as i32 - 1) as u32,
    );
    [p[0] as f32, p[1] as f32, p[2] as f32]
}
fn luminance(p: [f32; 3]) -> f32 {
    p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722
}
fn difference(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max)
}
const DIRECTIONS: [(i32, i32); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (2, 1),
    (1, 2),
    (-1, 2),
    (-2, 1),
];

fn background_support(image: &RgbaImage, head: &Head, x: i32, y: i32, target: [f32; 3]) -> bool {
    let agrees = |sx, sy| {
        sx >= 0
            && sy >= 0
            && sx < image.width() as i32
            && sy < image.height() as i32
            && difference(target, rgb(image, sx, sy)) <= 18.0
    };
    let radius = (head.width * 0.055).round().clamp(4.0, 32.0);
    let ring_matches = DIRECTIONS
        .iter()
        .flat_map(|&(dx, dy)| [(dx, dy), (-dx, -dy)])
        .filter(|&(dx, dy)| {
            let scale = radius / (dx as f32).hypot(dy as f32);
            agrees(
                x + (dx as f32 * scale).round() as i32,
                y + (dy as f32 * scale).round() as i32,
            )
        })
        .count();
    if ring_matches < 12 {
        return false;
    }
    // A corridor through hair can itself reach a clipped frame edge. Its donor color
    // must also agree with the majority of the halo's widely spaced top/side samples;
    // a few crown reflections at the top cannot establish a background color family.
    let mut exterior_matches = 0;
    let mut exterior_samples = 0;
    let mut sample_exterior = |sx: i32, sy: i32| {
        if sx >= 0 && sy >= 0 && sx < image.width() as i32 && sy < image.height() as i32 {
            exterior_matches += usize::from(difference(target, rgb(image, sx, sy)) <= 30.0);
            exterior_samples += 1;
        }
    };
    for i in 0..=16 {
        sample_exterior(
            head.rect[0] + (head.rect[2] - 1 - head.rect[0]) * i / 16,
            head.rect[1],
        );
    }
    for i in 1..=16 {
        let sy = head.rect[1] + (head.rect[3] - 1 - head.rect[1]) * i / 16;
        sample_exterior(head.rect[0], sy);
        sample_exterior(head.rect[2] - 1, sy);
    }
    if exterior_samples < 25 || exterior_matches * 2 <= exterior_samples {
        return false;
    }
    // Narrow reflections in uniform hair can pass the ring check. A wider neighborhood
    // must contain both substantial donor-colored background and substantial foreground.
    let radius = (head.width * 0.4).round().max(12.0) as i32;
    let mut matches = 0;
    let mut samples = 0;
    for gy in -4..=4 {
        for gx in -4..=4 {
            let sx = x + gx * radius / 4;
            let sy = y + gy * radius / 4;
            if sx >= 0 && sy >= 0 && sx < image.width() as i32 && sy < image.height() as i32 {
                matches += usize::from(agrees(sx, sy));
                samples += 1;
            }
        }
    }
    if samples < 25 || matches * 100 < samples * 35 || matches * 100 > samples * 75 {
        return false;
    }
    // A matching corridor must reach the top or a side of the head halo. Interior
    // locks terminate at different-colored background; the lower halo is often hair.
    DIRECTIONS
        .iter()
        .flat_map(|&(dx, dy)| [(dx, dy), (-dx, -dy)])
        .any(|(dx, dy)| {
            let tx = if dx > 0 {
                (head.rect[2] - 1 - x) as f32 / dx as f32
            } else if dx < 0 {
                (head.rect[0] - x) as f32 / dx as f32
            } else {
                f32::INFINITY
            };
            let ty = if dy > 0 {
                (head.rect[3] - 1 - y) as f32 / dy as f32
            } else if dy < 0 {
                (head.rect[1] - y) as f32 / dy as f32
            } else {
                f32::INFINITY
            };
            let distance = tx.min(ty);
            if (dy > 0 && ty <= tx) || distance * (dx as f32).hypot(dy as f32) < head.width * 0.2 {
                return false;
            }
            (1..=12).all(|i| {
                let t = distance * i as f32 / 12.0;
                agrees(
                    x + (dx as f32 * t).round() as i32,
                    y + (dy as f32 * t).round() as i32,
                )
            })
        })
}

/// Detect narrow, elongated dark or light strands close to coherent hair masses.
/// Opposite sides must agree in color; wide locks and edges therefore remain untouched.
/// Textured backgrounds and low-contrast strands intentionally produce fewer repairs.
pub fn detect(
    image: &RgbaImage,
    seg: &Segmentation,
    cancel: Option<&AtomicBool>,
) -> Result<FlyawayMap> {
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        bail!("Flyaway cleanup cancelled");
    }
    if image.width() == 0 || image.height() == 0 || seg.faces.is_empty() {
        return Ok(FlyawayMap::default());
    }
    let heads: Vec<_> = seg
        .faces
        .iter()
        .filter(|f| f.bounds[2] > 0.0 && f.bounds[3] > 0.0)
        .map(|f| Head::new(f, image.width(), image.height()))
        .collect();
    let rows: Vec<Vec<Correction>> = (0..image.height())
        .into_par_iter()
        .map(|y| {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return vec![];
            }
            let mut corrections = vec![];
            let y = y as i32;
            for head in &heads {
                if !head.eligible || y < head.rect[1] || y >= head.rect[3] {
                    continue;
                }
                for x in head.rect[0]..head.rect[2] {
                    // A halo may overlap another person's face. Protect all detected faces,
                    // including uncertain detections that cannot initiate their own repair halo.
                    if heads.iter().any(|face| face.facial_skin(x, y))
                        || image.get_pixel(x as u32, y as u32)[3] < 250
                    {
                        continue;
                    }
                    let center = rgb(image, x, y);
                    let level = luminance(center);
                    let mut best = None;
                    for (dx, dy) in DIRECTIONS {
                        let scale = if dx.abs() + dy.abs() > 2 {
                            (head.step / 2).max(1)
                        } else {
                            head.step
                        };
                        let a = rgb(image, x + dx * scale, y + dy * scale);
                        let b = rgb(image, x - dx * scale, y - dy * scale);
                        if difference(a, b) > 16.0 {
                            continue;
                        }
                        let target = std::array::from_fn(|c| (a[c] + b[c]) * 0.5);
                        let contrast = luminance(target) - level;
                        if contrast.abs() < 18.0 {
                            continue;
                        }
                        let polarity = contrast.signum();
                        // Both sides, including a farther sample, must be intact background.
                        if polarity * (luminance(a) - level) < 14.0
                            || polarity * (luminance(b) - level) < 14.0
                        {
                            continue;
                        }
                        if difference(target, rgb(image, x + dx * scale * 2, y + dy * scale * 2))
                            > 25.0
                            || difference(
                                target,
                                rgb(image, x - dx * scale * 2, y - dy * scale * 2),
                            ) > 25.0
                        {
                            continue;
                        }
                        // Hair must continue along the ridge; isolated spots and grain are rejected.
                        let tangent = (-dy, dx);
                        let along = [
                            rgb(image, x + tangent.0 * scale, y + tangent.1 * scale),
                            rgb(image, x - tangent.0 * scale, y - tangent.1 * scale),
                        ];
                        if along.iter().any(|p| {
                            polarity * (luminance(target) - luminance(*p)) < contrast.abs() * 0.6
                        }) {
                            continue;
                        }
                        // Require a thicker same-toned hair mass nearby, rather than arbitrary thin background lines.
                        let reach = (head.width * 0.14).round().max(6.0) as i32;
                        let mass = DIRECTIONS.iter().any(|&(mx, my)| {
                            let mx = mx.signum();
                            let my = my.signum();
                            let q = rgb(image, x + mx * reach, y + my * reach);
                            polarity * (luminance(target) - luminance(q)) > contrast.abs() * 0.75
                                && difference(q, rgb(image, x + mx * reach + scale, y + my * reach))
                                    < 22.0
                                && difference(q, rgb(image, x + mx * reach, y + my * reach + scale))
                                    < 22.0
                        });
                        if !mass {
                            continue;
                        }
                        let confidence = ((contrast.abs() - 14.0) / 28.0).clamp(0.0, 1.0);
                        if best.as_ref().is_none_or(|(weight, _)| confidence > *weight) {
                            best = Some((confidence, target));
                        }
                    }
                    if let Some((confidence, target)) = best
                        && background_support(image, head, x, y, target)
                    {
                        let delta = std::array::from_fn(|c| {
                            (linear_byte(target[c].round().clamp(0.0, 255.0) as u8)
                                - linear_byte(center[c] as u8))
                                * confidence
                        });
                        corrections.push(Correction { x: x as u32, delta });
                    }
                }
            }
            corrections.sort_unstable_by_key(|p| p.x);
            corrections.dedup_by_key(|p| p.x);
            corrections
        })
        .collect();
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        bail!("Flyaway cleanup cancelled");
    }
    let mut result = FlyawayMap {
        width: image.width() as usize,
        rows: Vec::with_capacity(rows.len() + 1),
        pixels: vec![],
    };
    for row in rows {
        result.rows.push(result.pixels.len());
        result.pixels.extend(row);
    }
    result.rows.push(result.pixels.len());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    fn fixture(light: bool) -> (RgbaImage, Segmentation) {
        let (bg, hair) = if light { (30, 220) } else { (210, 35) };
        let mut image = RgbaImage::from_pixel(200, 200, Rgba([bg, bg, bg, 255]));
        for y in 25..105 {
            for x in 40..65 {
                image.put_pixel(x, y, Rgba([hair, hair, hair, 255]));
            }
        }
        for y in 28..100 {
            image.put_pixel(72, y, Rgba([hair, hair, hair, 255]));
        }
        let face = FaceMesh {
            landmarks: vec![],
            bounds: [0.35, 0.42, 0.35, 0.48],
            confidence: 1.0,
        };
        (
            image,
            Segmentation {
                faces: vec![face],
                ..Default::default()
            },
        )
    }
    #[test]
    fn repairs_thin_light_and_dark_hairs_and_preserves_mass_face_and_background() {
        for light in [false, true] {
            let (image, seg) = fixture(light);
            let map = detect(&image, &seg, None).unwrap();
            assert!(map.correction_count() > 20, "missing thin strand");
            assert_ne!(map.delta(60 * 200 + 72), [0.0; 3]);
            assert_eq!(
                map.delta(60 * 200 + 50),
                [0.0; 3],
                "coherent hair mass changed"
            );
            assert_eq!(map.delta(130 * 200 + 100), [0.0; 3], "face changed");
            assert_eq!(map.delta(15 * 200 + 150), [0.0; 3], "background changed");
            assert!(map.bytes() < 200 * 200, "sparse map unexpectedly large");
        }
    }
    #[test]
    fn absent_face_and_cancel_are_safe() {
        let (image, _) = fixture(false);
        assert_eq!(
            detect(&image, &Segmentation::default(), None)
                .unwrap()
                .correction_count(),
            0
        );
        assert!(
            detect(
                &image,
                &Segmentation::default(),
                Some(&AtomicBool::new(true))
            )
            .is_err()
        );
    }
    #[test]
    fn overlapping_head_halos_protect_every_face_regardless_of_order_or_confidence() {
        for light in [false, true] {
            let (image, original_seg) = fixture(light);
            let original = detect(&image, &original_seg, None).unwrap();
            let covered_strand = 60 * 200 + 72;
            let outside_strand = 35 * 200 + 72;
            assert_ne!(original.delta(covered_strand), [0.0; 3]);
            assert_ne!(original.delta(outside_strand), [0.0; 3]);
            for confidence in [1.0, 0.4] {
                let second = FaceMesh {
                    landmarks: vec![],
                    bounds: [0.325, 0.23, 0.14, 0.15],
                    confidence,
                };
                let protected = Head::new(&second, image.width(), image.height());
                for reverse in [false, true] {
                    let mut seg = original_seg.clone();
                    seg.faces.push(second.clone());
                    if reverse {
                        seg.faces.reverse();
                    }
                    let map = detect(&image, &seg, None).unwrap();
                    assert_eq!(
                        map.delta(covered_strand),
                        [0.0; 3],
                        "another head's halo repaired a facial mark"
                    );
                    assert_ne!(map.delta(outside_strand), [0.0; 3]);
                    for y in 40..85 {
                        for x in 60..100 {
                            if protected.facial_skin(x, y) {
                                assert_eq!(map.delta(y as usize * 200 + x as usize), [0.0; 3]);
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn preserves_internal_dark_ridges_and_glossy_highlights_but_repairs_exterior_strands() {
        for light in [false, true] {
            let (mut image, seg) = fixture(light);
            let (background, hair) = if light { (30, 220) } else { (210, 35) };
            for y in 25..105 {
                for x in 40..96 {
                    image.put_pixel(x, y, Rgba([hair, hair, hair, 255]));
                }
            }
            for y in 28..100 {
                image.put_pixel(72, y, Rgba([background, background, background, 255]));
            }
            // The broader reflection supplies the same-tone "hair mass" that fooled
            // the former narrow detector into accepting the internal contrasting ridge.
            for y in 40..86 {
                for x in 80..90 {
                    image.put_pixel(x, y, Rgba([background, background, background, 255]));
                }
            }
            for y in 28..70 {
                image.put_pixel(102, y, Rgba([hair, hair, hair, 255]));
            }
            let map = detect(&image, &seg, None).unwrap();
            assert_eq!(map.delta(60 * 200 + 72), [0.0; 3], "internal ridge changed");
            assert_ne!(
                map.delta(60 * 200 + 102),
                [0.0; 3],
                "exterior strand missed"
            );

            // A uniformly dark/light hair field must not count as exterior background
            // simply because it provides an uninterrupted corridor to the halo border.
            let mut uniform = RgbaImage::from_pixel(200, 200, Rgba([hair, hair, hair, 255]));
            for y in 28..100 {
                uniform.put_pixel(72, y, Rgba([background, background, background, 255]));
            }
            for y in 40..86 {
                for x in 80..90 {
                    uniform.put_pixel(x, y, Rgba([background, background, background, 255]));
                }
            }
            assert_eq!(
                detect(&uniform, &seg, None).unwrap().delta(60 * 200 + 72),
                [0.0; 3],
                "uniform coherent hair was mistaken for background"
            );
        }
    }
    #[test]
    fn preserves_reported_native_demo_hair_ridge_and_reflection() {
        let photo = crate::engine::load_photo(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/demo-portrait.png"),
        )
        .unwrap();
        let seg = Segmentation {
            faces: vec![FaceMesh {
                landmarks: vec![],
                bounds: [0.28593835, 0.21322566, 0.4460198, 0.35691807],
                confidence: 1.0,
            }],
            ..Default::default()
        };
        let map = detect(&photo.original, &seg, None).unwrap();
        for (x, y) in [
            (914, 428),
            (907, 668),
            (526, 96),
            (545, 100),
            (590, 119),
            (701, 121),
            (716, 207),
        ] {
            assert_eq!(
                map.delta(y * 1122 + x),
                [0.0; 3],
                "native glossy hair changed"
            );
        }
        for (x, y) in [(119, 663), (122, 456), (980, 673), (172, 339)] {
            assert_ne!(
                map.delta(y * 1122 + x),
                [0.0; 3],
                "native exterior strand missed"
            );
        }
    }
    #[test]
    fn preserves_isolated_marks_and_unrelated_background_lines() {
        let (mut image, seg) = fixture(false);
        image.put_pixel(120, 25, Rgba([35, 35, 35, 255]));
        for y in 20..70 {
            image.put_pixel(140, y, Rgba([35, 35, 35, 255]));
        }
        let map = detect(&image, &seg, None).unwrap();
        assert_eq!(
            map.delta(25 * 200 + 120),
            [0.0; 3],
            "isolated texture removed"
        );
        assert_eq!(
            map.delta(40 * 200 + 140),
            [0.0; 3],
            "background line away from hair removed"
        );
    }
}
