//! Non-destructive selective color, smooth tone curves and reusable reference grades.
//! Reference statistics are sampled from the complete source, never from individual render tiles.
use crate::engine::{linear_to_srgb, srgb_to_linear};
use anyhow::{Result, bail};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const HUE_NAMES: [&str; 8] = [
    "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta",
];
const HUE_CENTERS: [f32; 8] = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 270.0, 315.0];
pub const IDENTITY_CURVE: [f32; 9] = [0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.0];

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HueBand {
    pub hue: f32,
    pub saturation: f32,
    pub luminance: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorSettings {
    pub hsl: [HueBand; 8],
    pub hsl_strength: f32,
    /// Luminance, red, green, blue. Knot inputs are fixed at eighth intervals.
    pub curves: [[f32; 9]; 4],
    pub curves_strength: f32,
    /// The reference's statistics travel with presets and sessions. No linked file is required.
    pub reference: Option<ReferenceProfile>,
    pub reference_strength: f32,
}

impl Default for ColorSettings {
    fn default() -> Self {
        Self {
            hsl: [HueBand::default(); 8],
            hsl_strength: 100.0,
            curves: [IDENTITY_CURVE; 4],
            curves_strength: 100.0,
            reference: None,
            reference_strength: 0.0,
        }
    }
}

impl ColorSettings {
    pub fn active(&self) -> bool {
        (self.hsl_strength != 0.0 && self.hsl.iter().any(|b| *b != HueBand::default()))
            || (self.curves_strength != 0.0 && self.curves.iter().any(|c| *c != IDENTITY_CURVE))
            || (self.reference_strength != 0.0 && self.reference.is_some())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReferenceProfile {
    pub name: String,
    pub lightness: [f32; 17],
    pub chroma_mean: [f32; 2],
    pub chroma_sigma: [f32; 2],
}

pub fn load_reference(path: &Path) -> Result<ReferenceProfile> {
    let photo = crate::engine::load_photo(path)?;
    ReferenceProfile::from_image(photo.name.clone(), &photo.original)
}

impl ReferenceProfile {
    pub fn from_image(name: String, image: &RgbaImage) -> Result<Self> {
        if image.width() == 0 || image.height() == 0 {
            bail!("The reference image is empty");
        }
        let mut samples = Vec::with_capacity(128 * 128);
        // Fixed normalized locations keep preview, full-size export and tiled processing consistent.
        for y in 0..128u64 {
            for x in 0..128u64 {
                let sx = ((x * 2 + 1) * image.width() as u64 / 256).min(image.width() as u64 - 1);
                let sy = ((y * 2 + 1) * image.height() as u64 / 256).min(image.height() as u64 - 1);
                let p = image.get_pixel(sx as u32, sy as u32).0;
                if p[3] < 240 {
                    continue;
                }
                let rgb = [p[0], p[1], p[2]].map(|v| srgb_to_linear(v as f32 / 255.0));
                let lab = to_oklab(rgb);
                // Ignore clipped shadows/highlights; they otherwise dominate black studio backdrops.
                if (0.06..0.98).contains(&lab[0]) {
                    samples.push(lab);
                }
            }
        }
        if samples.len() < 16 {
            bail!("The reference needs visible, non-transparent midtones");
        }
        samples.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
        let lightness = std::array::from_fn(|i| {
            let q = 0.02 + i as f32 / 16.0 * 0.96;
            samples[(q * (samples.len() - 1) as f32).round() as usize][0]
        });
        // Trim the lightness tails before estimating color balance and variation.
        let a = samples.len() / 20;
        let b = samples.len() - a;
        let midtones = &samples[a..b];
        let n = midtones.len() as f32;
        let chroma_mean =
            std::array::from_fn(|c| midtones.iter().map(|v| v[c + 1]).sum::<f32>() / n);
        let chroma_sigma = std::array::from_fn(|c| {
            (midtones
                .iter()
                .map(|v| (v[c + 1] - chroma_mean[c]).powi(2))
                .sum::<f32>()
                / n)
                .sqrt()
        });
        Ok(Self {
            name,
            lightness,
            chroma_mean,
            chroma_sigma,
        })
    }
}

pub struct PreparedColor {
    hsl: [HueBand; 8],
    hsl_strength: f32,
    curves: [[f32; 1025]; 4],
    curves_strength: f32,
    curve_active: [bool; 4],
    reference: Option<(ReferenceProfile, ReferenceProfile, f32)>,
    active: bool,
}

impl PreparedColor {
    pub fn new(settings: &ColorSettings, source: &RgbaImage) -> Self {
        let curve_active = settings.curves.map(|c| c != IDENTITY_CURVE);
        let reference = settings
            .reference
            .as_ref()
            .filter(|_| settings.reference_strength > 0.0)
            .and_then(|target| {
                ReferenceProfile::from_image(String::new(), source)
                    .ok()
                    .map(|source| {
                        (
                            source,
                            target.clone(),
                            (settings.reference_strength / 100.0).clamp(0.0, 1.0),
                        )
                    })
            });
        Self {
            hsl: settings.hsl,
            hsl_strength: if settings.hsl.iter().any(|b| *b != HueBand::default()) {
                (settings.hsl_strength / 100.0).clamp(0.0, 1.0)
            } else {
                0.0
            },
            curves: std::array::from_fn(|c| {
                std::array::from_fn(|i| curve_value(&settings.curves[c], i as f32 / 1024.0))
            }),
            curves_strength: (settings.curves_strength / 100.0).clamp(0.0, 1.0),
            curve_active,
            reference,
            active: settings.active(),
        }
    }

    pub fn apply_linear(&self, mut pixel: [f32; 4]) -> [f32; 4] {
        if !self.active {
            return pixel;
        }
        let alpha = pixel[3];
        let mut rgb = [pixel[0], pixel[1], pixel[2]];
        if let Some((source, target, strength)) = &self.reference {
            let mut lab = to_oklab(rgb.map(|v| v.max(0.0)));
            let mapped = quantile_match(lab[0], &source.lightness, &target.lightness);
            lab[0] += (mapped - lab[0]) * strength;
            for c in 0..2 {
                let ratio =
                    (target.chroma_sigma[c] / source.chroma_sigma[c].max(0.005)).clamp(0.67, 1.5);
                let mapped = (lab[c + 1] - source.chroma_mean[c]) * ratio + target.chroma_mean[c];
                lab[c + 1] += (mapped - lab[c + 1]) * strength;
            }
            rgb = from_oklab(lab);
        }
        if self.hsl_strength > 0.0 {
            let srgb = rgb.map(|v| linear_to_srgb(v).clamp(0.0, 1.0));
            let [mut hue, mut sat, mut light] = rgb_to_hsl(srgb);
            let weights = hue_weights(hue);
            let change: [f32; 3] = std::array::from_fn(|c| {
                weights
                    .iter()
                    .zip(&self.hsl)
                    .map(|(w, b)| w * [b.hue, b.saturation, b.luminance][c])
                    .sum()
            });
            // Achromatic pixels have no meaningful hue and must not acquire a colored fringe.
            let chromatic = (sat / 0.08).clamp(0.0, 1.0);
            hue = (hue + change[0].clamp(-60.0, 60.0) * self.hsl_strength * chromatic)
                .rem_euclid(360.0);
            sat = adjust_unit(sat, change[1] / 100.0 * self.hsl_strength * chromatic);
            light = adjust_unit(light, change[2] / 100.0 * self.hsl_strength * chromatic);
            rgb = hsl_to_rgb([hue, sat, light]).map(srgb_to_linear);
        }
        if self.curves_strength > 0.0 && self.curve_active.iter().any(|a| *a) {
            let mut srgb = rgb.map(|v| linear_to_srgb(v).clamp(0.0, 1.0));
            if self.curve_active[0] {
                // Change luminance by a common scale so master-curve edits preserve hue.
                let linear = srgb.map(srgb_to_linear);
                let y = linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
                let encoded = linear_to_srgb(y).clamp(0.0, 1.0);
                let mapped = lut_value(&self.curves[0], encoded);
                let desired = srgb_to_linear(encoded + (mapped - encoded) * self.curves_strength);
                srgb = if y > 0.00001 {
                    linear.map(|v| linear_to_srgb((v * desired / y).max(0.0)).clamp(0.0, 1.0))
                } else {
                    [linear_to_srgb(desired); 3]
                };
            }
            for (c, value) in srgb.iter_mut().enumerate() {
                if self.curve_active[c + 1] {
                    *value +=
                        (lut_value(&self.curves[c + 1], *value) - *value) * self.curves_strength;
                }
            }
            rgb = srgb.map(srgb_to_linear);
        }
        pixel[..3].copy_from_slice(&rgb);
        pixel[3] = alpha;
        pixel
    }
}

fn adjust_unit(value: f32, adjustment: f32) -> f32 {
    if adjustment >= 0.0 {
        value + (1.0 - value) * adjustment.clamp(0.0, 1.0)
    } else {
        value * (1.0 + adjustment.clamp(-1.0, 0.0))
    }
}

fn hue_weights(hue: f32) -> [f32; 8] {
    let h = hue.rem_euclid(360.0);
    let upper = HUE_CENTERS.iter().position(|v| *v > h).unwrap_or(8);
    let lower = upper.saturating_sub(1);
    let end = if upper == 8 {
        360.0
    } else {
        HUE_CENTERS[upper]
    };
    let t = ((h - HUE_CENTERS[lower]) / (end - HUE_CENTERS[lower])).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    let mut weights = [0.0; 8];
    weights[lower] = 1.0 - t;
    weights[upper % 8] += t;
    weights
}

/// Shape-preserving cubic interpolation. Adjacent knots cannot produce an overshoot or halo.
pub fn curve_value(points: &[f32; 9], x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0) * 8.0;
    let i = (x.floor() as usize).min(7);
    let t = x - i as f32;
    let slope = |j: usize| {
        if j == 0 {
            return points[1] - points[0];
        }
        if j == 8 {
            return points[8] - points[7];
        }
        let a = points[j] - points[j - 1];
        let b = points[j + 1] - points[j];
        if a * b <= 0.0 {
            0.0
        } else {
            2.0 * a * b / (a + b)
        }
    };
    let y = (2.0 * t.powi(3) - 3.0 * t.powi(2) + 1.0) * points[i]
        + (t.powi(3) - 2.0 * t.powi(2) + t) * slope(i)
        + (-2.0 * t.powi(3) + 3.0 * t.powi(2)) * points[i + 1]
        + (t.powi(3) - t.powi(2)) * slope(i + 1);
    y.clamp(0.0, 1.0)
}

fn lut_value(lut: &[f32; 1025], x: f32) -> f32 {
    let position = x.clamp(0.0, 1.0) * 1024.0;
    let i = (position as usize).min(1023);
    lut[i] + (lut[i + 1] - lut[i]) * (position - i as f32)
}

fn quantile_match(value: f32, source: &[f32; 17], target: &[f32; 17]) -> f32 {
    // Preserve extreme black/white endpoints through gradual extrapolation to 0/1.
    let mut previous = (0.0, 0.0);
    for i in 0..=17 {
        let next = if i == 17 {
            (1.0, 1.0)
        } else {
            (source[i], target[i])
        };
        if value <= next.0 && next.0 - previous.0 > 0.00001 {
            let t = ((value - previous.0) / (next.0 - previous.0)).clamp(0.0, 1.0);
            // Bound single-grade brightness corrections to prevent extreme scene changes.
            return (previous.1 + (next.1 - previous.1) * t)
                .clamp((value - 0.25).max(0.0), (value + 0.25).min(1.0));
        }
        previous = next;
    }
    value
}

fn rgb_to_hsl([r, g, b]: [f32; 3]) -> [f32; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let light = (max + min) * 0.5;
    if delta < 0.000001 {
        return [0.0, 0.0, light];
    }
    let hue = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    } * 60.0;
    [
        hue,
        delta / (1.0 - (2.0 * light - 1.0).abs()).max(0.000001),
        light,
    ]
}

fn hsl_to_rgb([hue, sat, light]: [f32; 3]) -> [f32; 3] {
    let c = (1.0 - (2.0 * light - 1.0).abs()) * sat;
    let h = hue / 60.0;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let rgb = match h as usize {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    rgb.map(|v| v + light - c * 0.5)
}

fn to_oklab([r, g, b]: [f32; 3]) -> [f32; 3] {
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn from_oklab([light, a, b]: [f32; 3]) -> [f32; 3] {
    let l = (light + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (light - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (light - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn studio_image(warm: u8, gain: f32) -> RgbaImage {
        RgbaImage::from_fn(192, 128, |x, y| {
            let v = 40.0 + ((x + y * 2) % 150) as f32;
            image::Rgba([
                (v * gain + warm as f32).min(254.0) as u8,
                (v * gain) as u8,
                (v * gain * 0.85) as u8,
                255,
            ])
        })
    }

    #[test]
    fn neutral_settings_and_disabled_stored_changes_are_identity() {
        let source = studio_image(10, 1.0);
        let mut s = ColorSettings::default();
        s.hsl[1].hue = 50.0;
        s.hsl_strength = 0.0;
        s.curves[1][4] = 0.8;
        s.curves_strength = 0.0;
        s.reference = Some(ReferenceProfile::from_image("reference".into(), &source).unwrap());
        let grade = PreparedColor::new(&s, &source);
        for pixel in [
            [0.0, 0.0, 0.0, 0.2],
            [0.03, 0.1, 0.8, 0.6],
            [0.4, 0.2, 0.1, 1.0],
        ] {
            assert_eq!(grade.apply_linear(pixel), pixel);
        }
    }

    #[test]
    fn chosen_reference_loads_from_a_real_file_and_survives_file_removal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Approved studio look.png");
        let target = studio_image(25, 1.05);
        target.save(&path).unwrap();
        let reference = load_reference(&path).unwrap();
        assert_eq!(reference.name, "Approved studio look.png");
        let expected = ReferenceProfile::from_image(reference.name.clone(), &target).unwrap();
        assert_eq!(reference, expected);
        let settings = ColorSettings {
            reference: Some(reference),
            reference_strength: 80.0,
            ..Default::default()
        };
        let text = ron::to_string(&settings).unwrap();
        std::fs::remove_file(&path).unwrap();
        let restored: ColorSettings = ron::from_str(&text).unwrap();
        let source = studio_image(3, 0.85);
        let grade = PreparedColor::new(&restored, &source);
        let input = [0.2, 0.13, 0.08, 0.6];
        assert_ne!(grade.apply_linear(input), input);
        assert_eq!(grade.apply_linear(input)[3], input[3]);
        assert!(load_reference(&path).is_err());
    }

    #[test]
    fn legacy_presets_receive_neutral_color_defaults_and_auto_retouch_preserves_grades() {
        let mut settings: crate::engine::Settings = ron::from_str("(exposure:0.25,)").unwrap();
        assert_eq!(settings.color, ColorSettings::default());
        settings.color.hsl[1].hue = -12.0;
        settings.color.curves[2][5] = 0.7;
        settings.color.reference = Some(
            ReferenceProfile::from_image("Reference".into(), &studio_image(25, 1.05)).unwrap(),
        );
        settings.color.reference_strength = 65.0;
        let before = settings.color.clone();
        settings.apply_auto_retouch();
        assert_eq!(settings.color, before);
        assert_eq!(settings.exposure, 0.25);
    }

    #[test]
    fn selective_hsl_keeps_other_hue_sectors_and_gray_untouched() {
        let source = studio_image(0, 1.0);
        let mut settings = ColorSettings::default();
        settings.hsl[0].hue = 30.0;
        settings.hsl[0].saturation = -80.0;
        let grade = PreparedColor::new(&settings, &source);
        let red = [0.8, 0.04, 0.04, 0.7];
        assert!((grade.apply_linear(red)[0] - red[0]).abs() > 0.1);
        for rgb in [[0.02, 0.03, 0.8, 1.0], [0.25, 0.25, 0.25, 0.5]] {
            let out = grade.apply_linear(rgb);
            for c in 0..4 {
                assert!((out[c] - rgb[c]).abs() < 0.000002);
            }
        }
    }

    #[test]
    fn smooth_curves_hit_knots_without_overshoot_and_channels_are_independent() {
        let points = [0.0, 0.04, 0.12, 0.23, 0.5, 0.77, 0.9, 0.97, 1.0];
        let mut last = 0.0;
        for i in 0..1025 {
            let v = curve_value(&points, i as f32 / 1024.0);
            assert!(v >= last - 0.00001 && (0.0..=1.0).contains(&v));
            last = v;
        }
        for (i, v) in points.iter().enumerate() {
            assert!((curve_value(&points, i as f32 / 8.0) - v).abs() < 0.000001);
        }
        let mut settings = ColorSettings::default();
        settings.curves[1][4] = 0.8;
        let grade = PreparedColor::new(&settings, &studio_image(0, 1.0));
        let input = [srgb_to_linear(0.5), 0.2, 0.35, 0.4];
        let output = grade.apply_linear(input);
        assert!((linear_to_srgb(output[0]) - 0.8).abs() < 0.00001);
        assert!((output[1] - input[1]).abs() < 0.000001);
        assert!((output[2] - input[2]).abs() < 0.000001);
        assert_eq!(output[3], input[3]);
    }

    #[test]
    fn reference_grade_adapts_each_batch_source_and_embeds_profile() {
        let target = studio_image(25, 1.05);
        let source_a = studio_image(3, 0.85);
        let source_b = studio_image(12, 0.95);
        let settings = ColorSettings {
            reference: Some(
                ReferenceProfile::from_image("Studio reference".into(), &target).unwrap(),
            ),
            reference_strength: 100.0,
            ..Default::default()
        };
        let restored: ColorSettings = ron::from_str(&ron::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
        for source in [&source_a, &source_b] {
            let grade = PreparedColor::new(&restored, source);
            let output = RgbaImage::from_fn(source.width(), source.height(), |x, y| {
                let p = source.get_pixel(x, y).0;
                let linear = [
                    srgb_to_linear(p[0] as f32 / 255.0),
                    srgb_to_linear(p[1] as f32 / 255.0),
                    srgb_to_linear(p[2] as f32 / 255.0),
                    1.0,
                ];
                let out = grade.apply_linear(linear);
                image::Rgba([
                    (linear_to_srgb(out[0]).clamp(0.0, 1.0) * 255.0).round() as u8,
                    (linear_to_srgb(out[1]).clamp(0.0, 1.0) * 255.0).round() as u8,
                    (linear_to_srgb(out[2]).clamp(0.0, 1.0) * 255.0).round() as u8,
                    255,
                ])
            });
            let profile = ReferenceProfile::from_image(String::new(), &output).unwrap();
            let before = ReferenceProfile::from_image(String::new(), source).unwrap();
            let reference = restored.reference.as_ref().unwrap();
            let error = |p: &ReferenceProfile| {
                p.lightness
                    .iter()
                    .zip(reference.lightness)
                    .map(|(a, b)| (a - b).abs())
                    .sum::<f32>()
                    + p.chroma_mean
                        .iter()
                        .zip(reference.chroma_mean)
                        .map(|(a, b)| (a - b).abs())
                        .sum::<f32>()
            };
            assert!(
                error(&profile) < error(&before) * 0.2,
                "{} -> {}",
                error(&before),
                error(&profile)
            );
        }
    }
}
