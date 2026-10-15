//! Relative face/eye focus measurements; background edges never contribute.
use crate::geometry::FaceMesh;
use image::RgbaImage;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FaceFocus {
    pub bounds: [f32; 4],
    pub score: Option<f32>,
    pub eyes: [Option<f32>; 2],
    pub native_size: [u32; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FocusReport {
    pub dimensions: [u32; 2],
    pub faces: Vec<FaceFocus>,
    /// The least-focused measurable face matters in a group photograph.
    pub score: Option<f32>,
}
impl FocusReport {
    pub fn summary(&self) -> String {
        if self.faces.is_empty() {
            "No face detected".into()
        } else if let Some(score) = self.score {
            format!("Face focus {score:.0}/100 · {} face(s)", self.faces.len())
        } else {
            "Faces too small for a reliable focus score".into()
        }
    }
}

pub fn analyze(image: &RgbaImage, faces: &[FaceMesh]) -> FocusReport {
    let mut reports = Vec::new();
    for face in faces.iter().filter(|f| f.confidence >= 0.5) {
        let Some(bounds) = bounded(face.bounds) else {
            continue;
        };
        let native_size = [
            (bounds[2] * image.width() as f32).round() as u32,
            (bounds[3] * image.height() as f32).round() as u32,
        ];
        let mut eyes = [None; 2];
        if native_size.iter().all(|n| *n >= 48) {
            for (i, indices) in [[33, 133, 159, 145], [362, 263, 386, 374]]
                .iter()
                .enumerate()
            {
                if let Some(points) = indices
                    .iter()
                    .map(|index| face.landmarks.get(*index))
                    .collect::<Option<Vec<_>>>()
                {
                    if points.iter().any(|p| {
                        p.iter().any(|v| !v.is_finite())
                            || p[0] < bounds[0]
                            || p[0] > bounds[0] + bounds[2]
                            || p[1] < bounds[1]
                            || p[1] > bounds[1] + bounds[3]
                    }) {
                        continue;
                    }
                    let min = [
                        points.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min),
                        points.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min),
                    ];
                    let max = [
                        points
                            .iter()
                            .map(|p| p[0])
                            .fold(f32::NEG_INFINITY, f32::max),
                        points
                            .iter()
                            .map(|p| p[1])
                            .fold(f32::NEG_INFINITY, f32::max),
                    ];
                    let width = max[0] - min[0];
                    if let Some(area) = bounded([
                        min[0] - width * 0.18,
                        min[1] - width * 0.24,
                        width * 1.36,
                        (max[1] - min[1]) + width * 0.48,
                    ]) {
                        eyes[i] = measure(image, area, 96, 12);
                    }
                }
            }
        }
        let face_score = (native_size.iter().all(|n| *n >= 48))
            .then(|| measure(image, bounds, 144, 48))
            .flatten();
        let eye_score = eyes.iter().flatten().copied().reduce(f32::min);
        let score = face_score.map(|face| eye_score.map_or(face, |eye| eye * 0.75 + face * 0.25));
        reports.push(FaceFocus {
            bounds,
            score,
            eyes,
            native_size,
        });
    }
    let score = reports.iter().filter_map(|f| f.score).reduce(f32::min);
    FocusReport {
        dimensions: [image.width(), image.height()],
        faces: reports,
        score,
    }
}

fn bounded(b: [f32; 4]) -> Option<[f32; 4]> {
    if !b.iter().all(|v| v.is_finite()) || b[2] <= 0. || b[3] <= 0. {
        return None;
    }
    let x = b[0].clamp(0., 1.);
    let y = b[1].clamp(0., 1.);
    let right = (b[0] + b[2]).clamp(0., 1.);
    let bottom = (b[1] + b[3]).clamp(0., 1.);
    (right > x && bottom > y).then_some([x, y, right - x, bottom - y])
}
fn measure(image: &RgbaImage, bounds: [f32; 4], edge: u32, minimum: u32) -> Option<f32> {
    let w = bounds[2] * image.width() as f32;
    let h = bounds[3] * image.height() as f32;
    if w.min(h) < minimum as f32 {
        return None;
    }
    let scale = (edge as f32 / w.max(h)).min(1.);
    let (ow, oh) = (
        (w * scale).round().max(8.) as usize,
        (h * scale).round().max(8.) as usize,
    );
    let mut pixels = vec![0.; ow * oh];
    for y in 0..oh {
        for x in 0..ow {
            let uv = [
                bounds[0] + bounds[2] * (x as f32 + 0.5) / ow as f32,
                bounds[1] + bounds[3] * (y as f32 + 0.5) / oh as f32,
            ];
            let p = crate::geometry::sample_rgba(
                image,
                uv[0] * image.width() as f32 - 0.5,
                uv[1] * image.height() as f32 - 0.5,
            );
            if p[3] < 128. {
                return None;
            }
            pixels[y * ow + x] = (p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722) / 255.;
        }
    }
    Some(focus_score(&pixels, ow, oh))
}
fn focus_score(pixels: &[f32], w: usize, h: usize) -> f32 {
    let mut sorted = pixels.to_vec();
    sorted.sort_by(f32::total_cmp);
    let contrast = sorted[sorted.len() * 9 / 10] - sorted[sorted.len() / 10];
    let mut filtered = vec![0.; pixels.len()];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            filtered[i] = (pixels[i] * 4.
                + (pixels[i - 1] + pixels[i + 1] + pixels[i - w] + pixels[i + w]) * 2.
                + pixels[i - w - 1]
                + pixels[i - w + 1]
                + pixels[i + w - 1]
                + pixels[i + w + 1])
                / 16.;
        }
    }
    let mut lap = Vec::new();
    let mut noise = Vec::new();
    for y in 2..h - 2 {
        for x in 2..w - 2 {
            let i = y * w + x;
            let gradient = (filtered[i - 1] - filtered[i + 1]).abs()
                + (filtered[i - w] - filtered[i + w]).abs();
            lap.push(
                (filtered[i] * 4.
                    - filtered[i - 1]
                    - filtered[i + 1]
                    - filtered[i - w]
                    - filtered[i + w])
                    .abs(),
            );
            if gradient < contrast.max(0.04) * 0.12 {
                noise.push((pixels[i] - filtered[i]).abs());
            }
        }
    }
    lap.sort_by(f32::total_cmp);
    // Focus relies on coherent edge curvature after noise filtering, rather than
    // full-image high-frequency energy that rewards textured backgrounds or noise.
    let start = lap.len() * 3 / 4;
    let signal = lap[start..].iter().sum::<f32>() / (lap.len() - start).max(1) as f32;
    let noise = noise.iter().sum::<f32>() / noise.len().max(1) as f32;
    let normalization = contrast.max(0.04);
    let signal = signal / normalization;
    let noise = noise / normalization;
    (100. * signal / (signal + 0.025 + noise * 2.) * (contrast / 0.08).min(1.)).clamp(0., 100.)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (RgbaImage, FaceMesh) {
        let image = RgbaImage::from_fn(320, 240, |x, y| {
            let value = if (x / 12 + y / 9) % 2 == 0 { 70 } else { 185 };
            image::Rgba([value, value, value, 255])
        });
        let face = FaceMesh {
            bounds: [0.25, 0.2, 0.5, 0.6],
            confidence: 0.99,
            landmarks: vec![],
        };
        (image, face)
    }
    #[test]
    fn blurred_faces_score_lower_and_background_texture_is_excluded() {
        let (image, face) = fixture();
        let sharp = analyze(&image, std::slice::from_ref(&face)).score.unwrap();
        let blurred = image::imageops::blur(&image, 4.);
        let moderate = analyze(&blurred, std::slice::from_ref(&face))
            .score
            .unwrap();
        let heavy = analyze(
            &image::imageops::blur(&image, 8.),
            std::slice::from_ref(&face),
        )
        .score
        .unwrap();
        assert!(
            moderate < sharp * 0.85,
            "Moderate blur did not reduce focus: {sharp} -> {moderate}"
        );
        assert!(
            heavy < moderate * 0.65,
            "Heavier blur did not further reduce focus: {moderate} -> {heavy}"
        );
        let mut background = image.clone();
        for (x, y, p) in background.enumerate_pixels_mut() {
            if !(80..240).contains(&x) || !(48..192).contains(&y) {
                *p = image::Rgba([255; 4]);
            }
        }
        assert!((analyze(&background, &[face]).score.unwrap() - sharp).abs() < 0.001);
    }
    #[test]
    fn unknown_and_tiny_faces_are_not_presented_as_focus_scores() {
        let (image, mut face) = fixture();
        assert!(analyze(&image, &[]).score.is_none());
        face.bounds = [0.2, 0.2, 0.04, 0.04];
        let tiny = analyze(&image, &[face]);
        assert_eq!(tiny.faces.len(), 1);
        assert!(tiny.score.is_none());
        assert!(tiny.summary().contains("too small"));
    }
    #[test]
    fn a_group_score_uses_the_least_focused_face() {
        let (mut image, mut face) = fixture();
        face.bounds = [0.05, 0.2, 0.4, 0.6];
        let mut second = face.clone();
        second.bounds[0] = 0.55;
        let blurred = image::imageops::blur(&image, 5.);
        for y in 0..image.height() {
            for x in 160..image.width() {
                image.put_pixel(x, y, *blurred.get_pixel(x, y));
            }
        }
        let report = analyze(&image, &[face, second]);
        assert!(report.faces[0].score.unwrap() > report.faces[1].score.unwrap());
        assert_eq!(report.score, report.faces[1].score);
    }
}
