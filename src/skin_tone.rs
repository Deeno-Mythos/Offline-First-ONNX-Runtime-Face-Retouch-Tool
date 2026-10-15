//! Skin-only chroma matching with preserved perceptual lightness and fine detail.
//! Oklab matrices: https://bottosson.github.io/posts/oklab/ (public domain / MIT).
use crate::engine::{self, Segmentation};
use anyhow::{Result, bail};
use image::RgbaImage;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkinTarget {
    pub name: String,
    pub chroma: [f32; 2],
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkinToneSettings {
    pub target: Option<SkinTarget>,
    pub strength: f32,
}
impl SkinToneSettings {
    pub fn active(&self) -> bool {
        self.strength.is_finite()
            && self.strength > 0.
            && self
                .target
                .as_ref()
                .is_some_and(|t| t.chroma.iter().all(|v| v.is_finite() && v.abs() <= 0.5))
    }
    pub fn apply(&self, mut pixel: [f32; 4], low: [f32; 4], mask: f32) -> [f32; 4] {
        if !self.active() || !mask.is_finite() {
            return pixel;
        }
        let Some(target) = &self.target else {
            return pixel;
        };
        let amount = (self.strength / 100.).clamp(0., 1.) * mask.clamp(0., 1.);
        if amount == 0. {
            return pixel;
        }
        let original = to_lab(pixel);
        let base = to_lab(low);
        let mut desired = original;
        for c in 0..2 {
            desired[c + 1] += (target.chroma[c] - base[c + 1]).clamp(-0.06, 0.06) * amount;
        }
        let mut rgb = to_rgb(desired);
        if rgb.iter().any(|v| !(0.0..=1.0).contains(v)) {
            rgb = [pixel[0], pixel[1], pixel[2]];
            // Compress chroma toward the original instead of clipping RGB and
            // unintentionally changing skin brightness at a gamut boundary.
            let (mut min, mut max) = (0., 1.);
            for _ in 0..10 {
                let t = (min + max) * 0.5;
                let candidate = to_rgb([
                    original[0],
                    original[1] + (desired[1] - original[1]) * t,
                    original[2] + (desired[2] - original[2]) * t,
                ]);
                if candidate.iter().all(|v| (0.0..=1.0).contains(v)) {
                    min = t;
                    rgb = candidate;
                } else {
                    max = t;
                }
            }
        }
        for c in 0..3 {
            pixel[c] = rgb[c].clamp(0., 1.);
        }
        pixel
    }
}
impl SkinTarget {
    pub fn from_face(
        name: String,
        image: &RgbaImage,
        seg: &Segmentation,
        index: usize,
    ) -> Result<Self> {
        let Some(face) = seg.faces.get(index) else {
            bail!("Detect a face before choosing its skin tone")
        };
        let [left, top, w, h] = face.bounds;
        if !face.bounds.iter().all(|v| v.is_finite()) || w <= 0. || h <= 0. {
            bail!("The selected face has invalid bounds")
        }
        let mut chroma = [Vec::new(), Vec::new()];
        for gy in 0..48 {
            for gx in 0..48 {
                let uv = [
                    left + w * (gx as f32 + 0.5) / 48.,
                    top + h * (gy as f32 + 0.5) / 48.,
                ];
                if uv.iter().any(|v| !(0.0..1.0).contains(v)) {
                    continue;
                }
                let x = (uv[0] * image.width() as f32) as u32;
                let y = (uv[1] * image.height() as f32) as u32;
                if engine::sample_mask(&seg.skin, seg, x, y, image.width(), image.height()) < 0.6 {
                    continue;
                }
                let rgba = image.get_pixel(x, y).0;
                if rgba[3] < 128 {
                    continue;
                }
                let lab = to_lab([
                    engine::linear_byte(rgba[0]),
                    engine::linear_byte(rgba[1]),
                    engine::linear_byte(rgba[2]),
                    1.,
                ]);
                if !(0.2..0.93).contains(&lab[0]) {
                    continue;
                }
                chroma[0].push(lab[1]);
                chroma[1].push(lab[2]);
            }
        }
        if chroma[0].len() < 24 {
            bail!("Not enough visible skin in the selected face")
        }
        let mut target = [0.; 2];
        for c in 0..2 {
            chroma[c].sort_by(f32::total_cmp);
            target[c] = chroma[c][chroma[c].len() / 2];
        }
        Ok(Self {
            name,
            chroma: target,
        })
    }
}
pub fn to_lab(p: [f32; 4]) -> [f32; 3] {
    let l = (0.412_221_46 * p[0] + 0.536_332_55 * p[1] + 0.051_445_995 * p[2]).cbrt();
    let m = (0.211_903_5 * p[0] + 0.680_699_5 * p[1] + 0.107_396_96 * p[2]).cbrt();
    let s = (0.088_302_46 * p[0] + 0.281_718_85 * p[1] + 0.629_978_7 * p[2]).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}
fn to_rgb(lab: [f32; 3]) -> [f32; 3] {
    let l = (lab[0] + 0.396_337_78 * lab[1] + 0.215_803_76 * lab[2]).powi(3);
    let m = (lab[0] - 0.105_561_346 * lab[1] - 0.063_854_17 * lab[2]).powi(3);
    let s = (lab[0] - 0.089_484_18 * lab[1] - 1.291_485_5 * lab[2]).powi(3);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skin_matching_preserves_lightness_texture_and_alpha_with_neutral_identity() {
        let p = [0.4, 0.19, 0.12, 0.7];
        let original = to_lab(p);
        assert_eq!(SkinToneSettings::default().apply(p, p, 1.), p);
        let target = SkinToneSettings {
            strength: 80.,
            target: Some(SkinTarget {
                name: "test".into(),
                chroma: [original[1] - 0.03, original[2] + 0.015],
            }),
        };
        let edited = target.apply(p, p, 1.);
        let lab = to_lab(edited);
        assert!((lab[0] - original[0]).abs() < 0.00001);
        assert!((lab[1] - original[1]).abs() > 0.02);
        assert_eq!(edited[3], 0.7);
        assert_eq!(target.apply(p, p, 0.), p);
        let highlight = [0.5, 0.24, 0.15, 0.7];
        assert!((to_lab(target.apply(highlight, p, 1.))[0] - to_lab(highlight)[0]).abs() < 0.00001);
    }
    #[test]
    fn conversion_round_trips_neutrals_and_saturated_linear_colors() {
        // Published forward Oklab values, independent of the inverse matrix.
        for (rgb, expected) in [
            ([1., 0., 0., 1.], [0.627_955_4, 0.22486306, 0.125_846_3]),
            ([0., 1., 0., 1.], [0.866_439_6, -0.23388757, 0.17949848]),
            ([0., 0., 1., 1.], [0.45201372, -0.03245698, -0.31152815]),
        ] {
            for (actual, reference) in to_lab(rgb).into_iter().zip(expected) {
                assert!((actual - reference).abs() < 0.000001);
            }
        }
        for p in [
            [0.2, 0.2, 0.2, 1.],
            [0.9, 0.05, 0.02, 1.],
            [0.01, 0.15, 0.9, 1.],
            [1., 1., 1., 1.],
        ] {
            let rgb = to_rgb(to_lab(p));
            for c in 0..3 {
                assert!((rgb[c] - p[c]).abs() < 0.00001);
            }
        }
    }

    #[test]
    fn malformed_saved_targets_never_poison_pixels() {
        let p = [0.4, 0.2, 0.1, 0.5];
        for chroma in [[f32::NAN, 0.], [0., f32::INFINITY], [2., 0.]] {
            let settings = SkinToneSettings {
                strength: 100.,
                target: Some(SkinTarget {
                    name: "invalid".into(),
                    chroma,
                }),
            };
            assert!(!settings.active());
            assert_eq!(settings.apply(p, p, 1.), p);
        }
    }
}
