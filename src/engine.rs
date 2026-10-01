//! Deterministic linear-light edits. Originals are never mutated.
use crate::shared::SharedVec;
use anyhow::{Context, Result, bail};
use half::f16;
use image::{DynamicImage, ImageDecoder, ImageEncoder, RgbaImage};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[path = "tiles.rs"]
mod tiles;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub disabled: Vec<Adjustment>,
    pub smoothing: f32,
    pub blemishes: f32,
    pub tone_evenness: f32,
    pub redness: f32,
    pub teeth: f32,
    pub eyes: f32,
    pub under_eyes: f32,
    pub forehead: f32,
    pub laugh_lines: f32,
    pub contour: f32,
    pub face_highlight: f32,
    pub eye_size: f32,
    pub nose_width: f32,
    pub lip_plumpness: f32,
    pub jawline: f32,
    pub healing: f32,
    pub flyaway_hairs: f32,
    pub exposure: f32,
    pub contrast: f32,
    pub shadows: f32,
    pub highlights: f32,
    pub warmth: f32,
    pub tint: f32,
    pub saturation: f32,
    pub color: crate::color::ColorSettings,
    pub sharpening: f32,
    pub vignette: f32,
    pub background: Background,
    pub background_blur: f32,
    pub background_color: [u8; 3],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            disabled: vec![],
            smoothing: 0.0,
            blemishes: 0.0,
            tone_evenness: 0.0,
            redness: 0.0,
            teeth: 0.0,
            eyes: 0.0,
            under_eyes: 0.0,
            forehead: 0.0,
            laugh_lines: 0.0,
            contour: 0.0,
            face_highlight: 0.0,
            eye_size: 0.0,
            nose_width: 0.0,
            lip_plumpness: 0.0,
            jawline: 0.0,
            healing: 100.0,
            flyaway_hairs: 0.0,
            exposure: 0.0,
            contrast: 0.0,
            shadows: 0.0,
            highlights: 0.0,
            warmth: 0.0,
            tint: 0.0,
            saturation: 0.0,
            color: Default::default(),
            sharpening: 0.0,
            vignette: 0.0,
            background: Background::Original,
            background_blur: 35.0,
            background_color: [238, 238, 238],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Adjustment {
    Smoothing,
    Blemishes,
    ToneEvenness,
    Redness,
    Teeth,
    Eyes,
    UnderEyes,
    Forehead,
    LaughLines,
    Contour,
    FaceHighlight,
    EyeSize,
    NoseWidth,
    LipPlumpness,
    Jawline,
    Healing,
    FlyawayHairs,
    Exposure,
    Contrast,
    Shadows,
    Highlights,
    Warmth,
    Tint,
    Saturation,
    Hsl,
    Curves,
    ReferenceMatch,
    Sharpening,
    Vignette,
    BackgroundBlur,
}
impl Settings {
    /// Apply facial cleanup without replacing the user's color, shape or background edits.
    pub fn apply_auto_retouch(&mut self) {
        for (adjustment, strength) in [
            (Adjustment::Smoothing, 45.0),
            (Adjustment::Blemishes, 55.0),
            (Adjustment::UnderEyes, 45.0),
            (Adjustment::Forehead, 28.0),
            (Adjustment::LaughLines, 22.0),
            (Adjustment::ToneEvenness, 40.0),
            (Adjustment::Redness, 35.0),
        ] {
            *self.value_mut(adjustment) = strength;
            self.disabled.retain(|a| *a != adjustment);
        }
    }

    pub fn value_mut(&mut self, adjustment: Adjustment) -> &mut f32 {
        match adjustment {
            Adjustment::Smoothing => &mut self.smoothing,
            Adjustment::Blemishes => &mut self.blemishes,
            Adjustment::ToneEvenness => &mut self.tone_evenness,
            Adjustment::Redness => &mut self.redness,
            Adjustment::Teeth => &mut self.teeth,
            Adjustment::Eyes => &mut self.eyes,
            Adjustment::UnderEyes => &mut self.under_eyes,
            Adjustment::Forehead => &mut self.forehead,
            Adjustment::LaughLines => &mut self.laugh_lines,
            Adjustment::Contour => &mut self.contour,
            Adjustment::FaceHighlight => &mut self.face_highlight,
            Adjustment::EyeSize => &mut self.eye_size,
            Adjustment::NoseWidth => &mut self.nose_width,
            Adjustment::LipPlumpness => &mut self.lip_plumpness,
            Adjustment::Jawline => &mut self.jawline,
            Adjustment::Healing => &mut self.healing,
            Adjustment::FlyawayHairs => &mut self.flyaway_hairs,
            Adjustment::Exposure => &mut self.exposure,
            Adjustment::Contrast => &mut self.contrast,
            Adjustment::Shadows => &mut self.shadows,
            Adjustment::Highlights => &mut self.highlights,
            Adjustment::Warmth => &mut self.warmth,
            Adjustment::Tint => &mut self.tint,
            Adjustment::Saturation => &mut self.saturation,
            Adjustment::Hsl => &mut self.color.hsl_strength,
            Adjustment::Curves => &mut self.color.curves_strength,
            Adjustment::ReferenceMatch => &mut self.color.reference_strength,
            Adjustment::Sharpening => &mut self.sharpening,
            Adjustment::Vignette => &mut self.vignette,
            Adjustment::BackgroundBlur => &mut self.background_blur,
        }
    }
    pub fn effective(&self) -> Self {
        let mut result = self.clone();
        for &adjustment in &self.disabled {
            *result.value_mut(adjustment) = 0.0;
            if adjustment == Adjustment::BackgroundBlur && result.background == Background::Blur {
                result.background = Background::Original;
            }
        }
        result
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Background {
    Original,
    Blur,
    Solid,
    Transparent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Skin,
    Teeth,
    Eyes,
    UnderEyes,
    Background,
    Heal,
    Liquify,
    Clone,
    Patch,
}

impl Target {
    pub fn label(self) -> &'static str {
        match self {
            Self::Skin => "Skin",
            Self::Teeth => "Teeth",
            Self::Eyes => "Eyes",
            Self::UnderEyes => "Under-eyes",
            Self::Background => "Background",
            Self::Heal => "Spot heal",
            Self::Liquify => "Liquify",
            Self::Clone => "Clone stamp",
            Self::Patch => "Patch",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub target: Target,
    pub center: [f32; 2],
    pub radius: f32,
    pub erase: bool,
    #[serde(default = "legacy_softness")]
    pub softness: f32,
}

fn legacy_softness() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Edit {
    pub settings: Settings,
    pub strokes: SharedVec<Stroke>,
    pub preset: Option<String>,
    pub warps: SharedVec<crate::geometry::WarpStroke>,
    pub clones: SharedVec<crate::cleanup::CloneStamp>,
    pub patches: SharedVec<crate::cleanup::PatchStroke>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub description: String,
    pub settings: Settings,
}

pub fn presets() -> Vec<Preset> {
    let mut natural = Settings {
        exposure: 0.08,
        shadows: 10.0,
        sharpening: 15.0,
        ..Settings::default()
    };
    natural.apply_auto_retouch();
    vec![
        Preset {
            name: "Natural Headshot".into(),
            description: "Clean. Confident. True to you.".into(),
            settings: natural.clone(),
        },
        Preset {
            name: "Wedding Soft".into(),
            description: "Warm light, a gentle glow.".into(),
            settings: Settings {
                smoothing: 40.0,
                warmth: 12.0,
                contrast: -8.0,
                shadows: 18.0,
                highlights: -20.0,
                ..natural.clone()
            },
        },
        Preset {
            name: "Studio Editorial".into(),
            description: "Sculpted light. Crisp detail.".into(),
            settings: Settings {
                smoothing: 40.0,
                contrast: 22.0,
                shadows: -8.0,
                sharpening: 28.0,
                ..natural.clone()
            },
        },
        Preset {
            name: "E-commerce".into(),
            description: "Neutral color, clean finish.".into(),
            settings: Settings {
                exposure: 0.12,
                shadows: 18.0,
                warmth: 0.0,
                ..natural.clone()
            },
        },
        Preset {
            name: "Moody Portrait".into(),
            description: "Deeper shadows, quieter tones.".into(),
            settings: Settings {
                exposure: -0.15,
                contrast: 18.0,
                saturation: -18.0,
                shadows: -20.0,
                vignette: 25.0,
                ..natural
            },
        },
    ]
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Segmentation {
    pub width: u32,
    pub height: u32,
    /// Generated face maps may store only their non-neutral rectangle. Explicit masks stay full size.
    pub map_crop: Option<Crop>,
    pub skin: SharedVec<f32>,
    pub teeth: SharedVec<f32>,
    pub eyes: SharedVec<f32>,
    pub background: SharedVec<f32>,
    pub faces: Vec<crate::geometry::FaceMesh>,
    pub under_eyes: SharedVec<f32>,
    pub forehead: SharedVec<f32>,
    pub laugh_lines: SharedVec<f32>,
    pub contour: SharedVec<f32>,
    pub highlight: SharedVec<f32>,
    pub blemish: SharedVec<f32>,
    #[serde(skip)]
    pub neural_blend: SharedVec<[f32; 3]>,
    #[serde(skip)]
    pub repair_delta: SharedVec<[f32; 3]>,
    pub status: String,
    pub custom_skin: bool,
}

impl Segmentation {
    pub fn map_bytes(&self) -> usize {
        [
            &self.skin,
            &self.teeth,
            &self.eyes,
            &self.background,
            &self.under_eyes,
            &self.forehead,
            &self.laugh_lines,
            &self.contour,
            &self.highlight,
            &self.blemish,
        ]
        .iter()
        .map(|v| v.len() * 4)
        .sum::<usize>()
            + (self.neural_blend.len() + self.repair_delta.len()) * 12
    }
    pub fn compact_generated_maps(&mut self) {
        if self.map_crop.is_some() || self.width == 0 || self.height == 0 {
            return;
        }
        let w = self.width as usize;
        let h = self.height as usize;
        let n = w * h;
        let maps = [
            &self.skin,
            &self.teeth,
            &self.eyes,
            &self.under_eyes,
            &self.forehead,
            &self.laugh_lines,
            &self.contour,
            &self.highlight,
            &self.blemish,
        ];
        let bounds = (0..h)
            .into_par_iter()
            .map(|y| {
                let mut min = w;
                let mut max = 0;
                for x in 0..w {
                    let i = y * w + x;
                    if maps.iter().any(|m| m.len() == n && m[i] != 0.0)
                        || (self.neural_blend.len() == n && self.neural_blend[i] != [0.5; 3])
                        || (self.repair_delta.len() == n && self.repair_delta[i] != [0.0; 3])
                    {
                        min = min.min(x);
                        max = max.max(x + 1);
                    }
                }
                if min < max {
                    (min, y, max, y + 1)
                } else {
                    (w, h, 0, 0)
                }
            })
            .reduce(
                || (w, h, 0, 0),
                |a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)),
            );
        if bounds == (0, 0, w, h) {
            return;
        }
        let crop = if bounds.2 == 0 {
            Crop {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }
        } else {
            Crop {
                x: bounds.0 as u32,
                y: bounds.1 as u32,
                width: (bounds.2 - bounds.0) as u32,
                height: (bounds.3 - bounds.1) as u32,
            }
        };
        fn trim<T: Clone>(data: &mut SharedVec<T>, width: usize, n: usize, crop: Crop) {
            if data.len() != n {
                return;
            }
            *data = (crop.y..crop.y + crop.height)
                .flat_map(|y| {
                    data[y as usize * width + crop.x as usize
                        ..y as usize * width + (crop.x + crop.width) as usize]
                        .iter()
                        .cloned()
                })
                .collect();
        }
        for map in [
            &mut self.skin,
            &mut self.teeth,
            &mut self.eyes,
            &mut self.under_eyes,
            &mut self.forehead,
            &mut self.laugh_lines,
            &mut self.contour,
            &mut self.highlight,
            &mut self.blemish,
        ] {
            trim(map, w, n, crop);
        }
        trim(&mut self.neural_blend, w, n, crop);
        trim(&mut self.repair_delta, w, n, crop);
        self.map_crop = Some(crop);
    }
    pub fn resample(&self, width: u32, height: u32) -> Self {
        if (width, height) == (self.width, self.height) {
            return self.clone();
        }
        let resize = |data: &[f32]| -> SharedVec<f32> {
            if data.is_empty() {
                return SharedVec::default();
            }
            (0..width * height)
                .into_par_iter()
                .map(|i| sample_mask(data, self, i % width, i / width, width, height))
                .collect()
        };
        Self {
            width,
            height,
            skin: resize(&self.skin),
            teeth: resize(&self.teeth),
            eyes: resize(&self.eyes),
            background: resize(&self.background),
            faces: self.faces.clone(),
            under_eyes: resize(&self.under_eyes),
            forehead: resize(&self.forehead),
            laugh_lines: resize(&self.laugh_lines),
            contour: resize(&self.contour),
            highlight: resize(&self.highlight),
            blemish: resize(&self.blemish),
            neural_blend: resize_rgb(&self.neural_blend, self, width, height, 0.5),
            repair_delta: resize_rgb(&self.repair_delta, self, width, height, 0.0),
            map_crop: None,
            status: self.status.clone(),
            custom_skin: self.custom_skin,
        }
    }
    /// Generated maps live in the disk cache; sessions retain explicit masks and face metadata.
    pub fn session_copy(&self) -> Self {
        let mut saved = Self {
            width: self.width,
            height: self.height,
            map_crop: self.map_crop,
            faces: self.faces.clone(),
            status: self.status.clone(),
            custom_skin: self.custom_skin,
            background: self.background.clone(),
            ..Default::default()
        };
        if self.faces.is_empty() || self.custom_skin {
            saved.skin = self.skin.clone();
            saved.teeth = self.teeth.clone();
            saved.eyes = self.eyes.clone();
        }
        saved
    }
}

#[derive(Clone)]
pub struct Photo {
    pub name: String,
    pub path: Option<PathBuf>,
    pub original: Arc<RgbaImage>,
    pub preview: Arc<RgbaImage>,
    pub analysis: Analysis,
}

#[derive(Clone, Debug)]
pub struct Analysis {
    pub mean: f32,
    pub clipped: f32,
    pub warmth: f32,
    pub sharpness: f32,
    pub notes: Vec<String>,
}

pub fn analyze(image: &RgbaImage) -> Analysis {
    let width = image.width() as usize;
    let (luma, warmth, clipped, edges) = image
        .as_raw()
        .par_chunks_exact(width.max(1) * 4)
        .enumerate()
        .map(|(y, row)| {
            let mut stats = (0f64, 0f64, 0u64, 0f64);
            for (x, p) in row.as_chunks::<4>().0.iter().enumerate() {
                let l = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.;
                stats.0 += l as f64;
                stats.1 += ((p[0] as f32 - p[2] as f32) / 255.) as f64;
                if !(0.015..=0.98).contains(&l) {
                    stats.2 += 1;
                }
                if x > 0 && y > 0 {
                    let a = row[(x - 1) * 4 + 1];
                    let b = image.as_raw()[((y - 1) * width + x) * 4 + 1];
                    stats.3 += ((p[1] as f32 - a as f32).abs() + (p[1] as f32 - b as f32).abs())
                        as f64
                        / 510.;
                }
            }
            stats
        })
        .reduce(
            || (0., 0., 0, 0.),
            |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2, a.3 + b.3),
        );
    let n = (image.width() as u64 * image.height() as u64).max(1) as f64;
    let mean = (luma / n) as f32;
    let warmth = (warmth / n) as f32;
    let sharpness = (edges / n) as f32;
    let mut notes = vec![
        if mean < 0.28 {
            "Low exposure · gently lift shadows"
        } else if mean > 0.76 {
            "Bright exposure · protect highlights"
        } else {
            "Balanced exposure · preserve natural depth"
        }
        .into(),
    ];
    notes.push(
        if warmth > 0.15 {
            "Warm color bias · check white balance"
        } else if warmth < -0.06 {
            "Cool color bias · check white balance"
        } else {
            "Neutral color balance"
        }
        .into(),
    );
    if clipped as f64 / n > 0.06 {
        notes.push("Clipped tones · inspect before export".into());
    }
    notes.push("Local skin mask is color-based; inspect or refine with the brush".into());
    Analysis {
        mean,
        warmth,
        sharpness,
        clipped: (clipped as f64 / n) as f32,
        notes,
    }
}

pub fn load_photo(path: &Path) -> Result<Photo> {
    let absolute_path = std::fs::canonicalize(path)?;
    let path = absolute_path.as_path();
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut decoder = reader.into_decoder()?;
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 80_000_000 {
        bail!("Image exceeds the 80 megapixel working limit");
    }
    let orientation = decoder.orientation()?;
    let icc = decoder.icc_profile()?;
    let mut decoded = DynamicImage::from_decoder(decoder)?;
    decoded.apply_orientation(orientation);
    let mut original = decoded.into_rgba8();
    if let Some(icc) = icc {
        let profile =
            moxcms::ColorProfile::new_from_slice(&icc).context("Invalid embedded ICC profile")?;
        if profile.color_space != moxcms::DataColorSpace::Rgb {
            bail!("Convert this non-RGB ICC image to sRGB before importing");
        }
        let transform = profile
            .create_transform_8bit(
                moxcms::Layout::Rgba,
                &moxcms::ColorProfile::new_srgb(),
                moxcms::Layout::Rgba,
                Default::default(),
            )
            .context("Cannot convert embedded profile to sRGB")?;
        let mut converted = vec![0; original.as_raw().len()];
        transform.transform(original.as_raw(), &mut converted)?;
        let (oriented_width, oriented_height) = original.dimensions();
        original = RgbaImage::from_raw(oriented_width, oriented_height, converted)
            .context("Invalid oriented ICC image dimensions")?;
    }
    Ok(photo_from_image(
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        Some(path.to_path_buf()),
        original,
    ))
}

pub fn photo_from_image(name: String, path: Option<PathBuf>, original: RgbaImage) -> Photo {
    let original = Arc::new(original);
    let longest = original.width().max(original.height());
    let preview = if longest > 1440 {
        let scale = 1440.0 / longest as f32;
        Arc::new(image::imageops::resize(
            original.as_ref(),
            (original.width() as f32 * scale).round().max(1.0) as u32,
            (original.height() as f32 * scale).round().max(1.0) as u32,
            image::imageops::FilterType::Lanczos3,
        ))
    } else {
        original.clone()
    };
    let analysis = analyze(&preview);
    Photo {
        name,
        path,
        original,
        preview,
        analysis,
    }
}

pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
/// Exact transfer values for the 256 possible encoded input levels.
pub fn linear_byte(value: u8) -> f32 {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0)))[value as usize]
}
pub fn linear_to_srgb(v: f32) -> f32 {
    let v = v.max(0.0);
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}
/// Exact 8-bit output quantization. A small coarse table selects at most one threshold;
/// thresholds are found using the same float transfer/rounding as the full calculation.
fn linear_output_byte(value: f32) -> u8 {
    struct Quantizer {
        buckets: [u8; 16_384],
        thresholds: [f32; 255],
    }
    static TABLE: std::sync::OnceLock<Quantizer> = std::sync::OnceLock::new();
    if value >= 1.0 {
        return 255;
    }
    if value <= 0.0 || value.is_nan() {
        return 0;
    }
    let table = TABLE.get_or_init(|| {
        let thresholds = std::array::from_fn(|i| {
            let (mut low, mut high) = (0u32, 1f32.to_bits());
            while low < high {
                let mid = low + (high - low) / 2;
                if (linear_to_srgb(f32::from_bits(mid)) * 255.).round() as usize > i {
                    high = mid;
                } else {
                    low = mid + 1;
                }
            }
            f32::from_bits(low)
        });
        let buckets =
            std::array::from_fn(|i| (linear_to_srgb(i as f32 / 16_384.) * 255.).round() as u8);
        Quantizer {
            buckets,
            thresholds,
        }
    });
    let mut byte = table.buckets[(value * 16_384.) as usize] as usize;
    if byte < 255 && value >= table.thresholds[byte] {
        byte += 1;
    }
    byte as u8
}
fn lum(p: &[f32; 4]) -> f32 {
    p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722
}
#[cfg(feature = "onnx")]
fn lum_half(p: &[f16; 4]) -> f32 {
    p[0].to_f32() * 0.2126 + p[1].to_f32() * 0.7152 + p[2].to_f32() * 0.0722
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// Conservative chroma mask; never advertised as face detection.
fn skin_weight(p: &[u8]) -> f32 {
    let r = p[0] as f32 / 255.0;
    let g = p[1] as f32 / 255.0;
    let b = p[2] as f32 / 255.0;
    let cb = 0.5 - 0.168736 * r - 0.331264 * g + 0.5 * b;
    let cr = 0.5 + 0.5 * r - 0.418688 * g - 0.081312 * b;
    let distance = ((cb - 0.43) / 0.075).powi(2) + ((cr - 0.58) / 0.095).powi(2);
    ((1.0 - distance / 2.0).clamp(0.0, 1.0) * ((r.max(g).max(b) - 0.06) / 0.14).clamp(0.0, 1.0))
        * p[3] as f32
        / 255.0
}

fn sample_mask(data: &[f32], seg: &Segmentation, x: u32, y: u32, w: u32, h: u32) -> f32 {
    if data.is_empty() {
        return 0.0;
    }
    if data.len() != (seg.width * seg.height) as usize {
        return sample_registered::<1>(
            bytemuck::cast_slice(data),
            seg,
            (x as f32 + 0.5) / w as f32,
            (y as f32 + 0.5) / h as f32,
            0.0,
        )[0]
        .clamp(0.0, 1.0);
    }
    sample_channel(
        data,
        seg.width,
        seg.height,
        (x as f32 + 0.5) / w as f32,
        (y as f32 + 0.5) / h as f32,
    )
    .clamp(0.0, 1.0)
}

pub(crate) fn sample_channel(data: &[f32], w: u32, h: u32, u: f32, v: f32) -> f32 {
    if w == 0 || h == 0 || data.len() != (w * h) as usize {
        return 0.0;
    }
    let x = (u * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let y = (v * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (ix, iy) = (x as u32, y as u32);
    let (tx, ty) = (x - ix as f32, y - iy as f32);
    let at = |x: u32, y: u32| data[(y.min(h - 1) * w + x.min(w - 1)) as usize];
    (at(ix, iy) * (1.0 - tx) + at(ix + 1, iy) * tx) * (1.0 - ty)
        + (at(ix, iy + 1) * (1.0 - tx) + at(ix + 1, iy + 1) * tx) * ty
}
fn sample_rgb(data: &[[f32; 3]], w: u32, h: u32, u: f32, v: f32, neutral: f32) -> [f32; 3] {
    if w == 0 || h == 0 || data.len() != (w * h) as usize {
        return [neutral; 3];
    }
    let x = (u * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let y = (v * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (ix, iy) = (x as u32, y as u32);
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
fn sample_seg_rgb(data: &[[f32; 3]], seg: &Segmentation, u: f32, v: f32, neutral: f32) -> [f32; 3] {
    if data.len() == (seg.width * seg.height) as usize {
        sample_rgb(data, seg.width, seg.height, u, v, neutral)
    } else {
        sample_registered(data, seg, u, v, neutral)
    }
}
fn sample_registered<const C: usize>(
    data: &[[f32; C]],
    seg: &Segmentation,
    u: f32,
    v: f32,
    neutral: f32,
) -> [f32; C] {
    let Some(crop) = seg.map_crop else {
        return [neutral; C];
    };
    if seg.width == 0 || seg.height == 0 || data.len() != (crop.width * crop.height) as usize {
        return [neutral; C];
    }
    let x = (u * seg.width as f32 - 0.5).clamp(0.0, (seg.width - 1) as f32);
    let y = (v * seg.height as f32 - 0.5).clamp(0.0, (seg.height - 1) as f32);
    let (ix, iy) = (x as u32, y as u32);
    let (tx, ty) = (x - ix as f32, y - iy as f32);
    let at = |x: u32, y: u32| {
        let (x, y) = (x.min(seg.width - 1), y.min(seg.height - 1));
        if x >= crop.x && y >= crop.y && x < crop.x + crop.width && y < crop.y + crop.height {
            data[((y - crop.y) * crop.width + x - crop.x) as usize]
        } else {
            [neutral; C]
        }
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
fn resize_rgb(
    data: &[[f32; 3]],
    seg: &Segmentation,
    ow: u32,
    oh: u32,
    neutral: f32,
) -> SharedVec<[f32; 3]> {
    if data.is_empty() {
        return SharedVec::default();
    }
    (0..ow * oh)
        .into_par_iter()
        .map(|i| {
            sample_seg_rgb(
                data,
                seg,
                (i % ow) as f32 / ow as f32,
                (i / ow) as f32 / oh as f32,
                neutral,
            )
        })
        .collect()
}

fn blur(input: &[[f32; 4]], w: usize, h: usize, radius: usize) -> Vec<[f16; 4]> {
    if radius == 0 {
        return input.iter().map(|pixel| pixel.map(f16::from_f32)).collect();
    }
    let radius = radius.min(w.max(h));
    let mut horizontal = vec![[f16::ZERO; 4]; input.len()];
    horizontal
        .par_chunks_mut(w)
        .enumerate()
        .for_each(|(y, row)| {
            let mut sum = [0.0; 4];
            for dx in -(radius as isize)..=radius as isize {
                let p = input[y * w + dx.clamp(0, w as isize - 1) as usize];
                for c in 0..4 {
                    sum[c] += p[c];
                }
            }
            let n = (2 * radius + 1) as f32;
            for (x, out) in row.iter_mut().enumerate() {
                for c in 0..4 {
                    out[c] = f16::from_f32(sum[c] / n);
                }
                let a =
                    input[y * w + (x as isize - radius as isize).clamp(0, w as isize - 1) as usize];
                let b = input[y * w + (x + radius + 1).min(w - 1)];
                for c in 0..4 {
                    sum[c] += b[c] - a[c];
                }
            }
        });
    // Sliding vertical windows keep cost linear even for full-resolution exports.
    let mut columns = vec![[f16::ZERO; 4]; input.len()];
    columns
        .par_chunks_mut(h)
        .enumerate()
        .for_each(|(x, column)| {
            let mut sum = [0.0; 4];
            for dy in -(radius as isize)..=radius as isize {
                let p = horizontal[dy.clamp(0, h as isize - 1) as usize * w + x];
                for c in 0..4 {
                    sum[c] += p[c].to_f32();
                }
            }
            let n = (2 * radius + 1) as f32;
            for (y, out) in column.iter_mut().enumerate() {
                for c in 0..4 {
                    out[c] = f16::from_f32(sum[c] / n);
                }
                let a = horizontal
                    [(y as isize - radius as isize).clamp(0, h as isize - 1) as usize * w + x];
                let b = horizontal[(y + radius + 1).min(h - 1) * w + x];
                for c in 0..4 {
                    sum[c] += b[c].to_f32() - a[c].to_f32();
                }
            }
        });
    let mut output = horizontal;
    output.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            *out = columns[x * h + y];
        }
    });
    output
}

struct LocalMasks {
    offsets: [Option<usize>; 5],
    channels_per_pixel: usize,
    data: Vec<u8>,
}
impl LocalMasks {
    fn value(&self, pixel: usize, channel: usize) -> f32 {
        let Some(offset) = self.offsets[channel] else {
            return 0.0;
        };
        self.data
            .get(pixel * self.channels_per_pixel + offset)
            .map_or(0.0, |value| *value as f32 / 255.0)
    }
    fn pixels(&self) -> usize {
        self.data
            .len()
            .checked_div(self.channels_per_pixel)
            .unwrap_or(0)
    }
    fn bytes(&self) -> usize {
        self.data.len()
    }
}

fn local_layers(
    image: &RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    targets: [bool; 5],
) -> LocalMasks {
    let (w, h) = image.dimensions();
    local_layers_region(
        image,
        edit,
        seg,
        targets,
        Crop {
            x: 0,
            y: 0,
            width: w,
            height: h,
        },
    )
}

fn local_layers_region(
    image: &RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    targets: [bool; 5],
    crop: Crop,
) -> LocalMasks {
    let (w, h) = image.dimensions();
    let channels_per_pixel = targets.iter().filter(|active| **active).count();
    let mut offsets = [None; 5];
    let mut channel_offset = 0;
    for (index, active) in targets.into_iter().enumerate() {
        if active {
            offsets[index] = Some(channel_offset);
            channel_offset += 1;
        }
    }
    let pixel_count = (crop.width * crop.height) as usize;
    let mut data = vec![0; pixel_count * channels_per_pixel];
    if channels_per_pixel != 0 {
        data.par_chunks_mut(channels_per_pixel)
            .enumerate()
            .for_each(|(i, packed)| {
                let x = crop.x + i as u32 % crop.width;
                let y = crop.y + i as u32 / crop.width;
                let source_index = (y * w + x) as usize * 4;
                let pixel = &image.as_raw()[source_index..source_index + 4];
                let mut offset = 0;
                let mut store = |channel: usize, value: f32| {
                    if targets[channel] {
                        packed[offset] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                        offset += 1;
                    }
                };
                let skin = if targets[0] {
                    if seg.is_none_or(|s| s.skin.is_empty()) {
                        skin_weight(pixel)
                    } else {
                        sample_mask(&seg.unwrap().skin, seg.unwrap(), x, y, w, h)
                    }
                } else {
                    0.0
                };
                store(0, skin);
                for (channel, mask) in [
                    (1, seg.map(|s| &s.teeth[..])),
                    (2, seg.map(|s| &s.eyes[..])),
                    (3, seg.map(|s| &s.under_eyes[..])),
                    (4, seg.map(|s| &s.background[..])),
                ] {
                    store(
                        channel,
                        mask.filter(|_| targets[channel])
                            .map_or(0.0, |data| sample_mask(data, seg.unwrap(), x, y, w, h)),
                    );
                }
            });
    }
    for stroke in &edit.strokes {
        let cx = stroke.center[0] * w as f32;
        let cy = stroke.center[1] * h as f32;
        let radius = (stroke.radius * w.min(h) as f32).max(0.01);
        let x0 = ((cx - radius).floor().max(0.0) as u32).max(crop.x);
        let x1 = ((cx + radius).ceil().min(w as f32) as u32).min(crop.x + crop.width);
        let y0 = ((cy - radius).floor().max(0.0) as u32).max(crop.y);
        let y1 = ((cy + radius).ceil().min(h as f32) as u32).min(crop.y + crop.height);
        if matches!(
            stroke.target,
            Target::Heal | Target::Liquify | Target::Clone | Target::Patch
        ) {
            continue;
        }
        let target = match stroke.target {
            Target::Skin => 0,
            Target::Teeth => 1,
            Target::Eyes => 2,
            Target::UnderEyes => 3,
            Target::Background => 4,
            _ => unreachable!(),
        };
        if !targets[target] {
            continue;
        }
        for y in y0..y1 {
            for x in x0..x1 {
                let distance = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2))
                    / radius.powi(2);
                if distance >= 1.0 {
                    continue;
                }
                let weight = brush_weight(distance.sqrt(), stroke.softness);
                let i = ((y - crop.y) * crop.width + x - crop.x) as usize;
                let channel = offsets[target].unwrap();
                let mask_index = i * channels_per_pixel + channel;
                let before = data[mask_index] as f32 / 255.0;
                data[mask_index] = (lerp(before, if stroke.erase { 0.0 } else { 1.0 }, weight)
                    .clamp(0.0, 1.0)
                    * 255.0)
                    .round() as u8;
            }
        }
    }
    LocalMasks {
        offsets,
        channels_per_pixel,
        data,
    }
}

#[cfg(feature = "onnx")]
#[derive(Clone, Copy)]
struct UnderEyeReference {
    eye: crate::model::portrait::EyeRegion,
    luma: f32,
}

#[cfg(feature = "onnx")]
fn under_eye_shadow_lift(
    low: &[[f16; 4]],
    masks: &LocalMasks,
    seg: Option<&Segmentation>,
    w: u32,
    h: u32,
) -> Vec<f32> {
    let Some(seg) = seg.filter(|s| !s.faces.is_empty()) else {
        return vec![0.0; masks.pixels()];
    };
    let references: Vec<UnderEyeReference> = seg
        .faces
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
                    let index = (py * w + px) as usize;
                    if !seg.skin.is_empty() && sample_mask(&seg.skin, seg, px, py, w, h) < 0.25 {
                        continue;
                    }
                    if masks.value(index, 3) > 0.18 {
                        continue;
                    }
                    sum += lum_half(&low[index]);
                    count += 1;
                }
            }
            if count == 0 {
                let x = (eye.lower[0] + eye.down[0] * eye.width).round() as u32;
                let y = (eye.lower[1] + eye.down[1] * eye.width).round() as u32;
                let index = (y.min(h - 1) * w + x.min(w - 1)) as usize;
                UnderEyeReference {
                    eye,
                    luma: lum_half(&low[index]),
                }
            } else {
                UnderEyeReference {
                    eye,
                    luma: sum / count as f32,
                }
            }
        })
        .collect();
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
            let p = [(i as u32 % w) as f32 + 0.5, (i as u32 / w) as f32 + 0.5];
            let reference = references
                .iter()
                .min_by(|a, b| {
                    let distance = |r: &UnderEyeReference| {
                        (p[0] - r.eye.lower[0]).powi(2) + (p[1] - r.eye.lower[1]).powi(2)
                    };
                    distance(a).total_cmp(&distance(b))
                })
                .unwrap();
            let shadow = (reference.luma - lum_half(&low[i]) - 0.012).clamp(0.0, 0.12);
            shadow * region * 0.9
        })
        .collect()
}

#[cfg(not(feature = "onnx"))]
fn under_eye_shadow_lift(
    _: &[[f16; 4]],
    masks: &LocalMasks,
    _: Option<&Segmentation>,
    _: u32,
    _: u32,
) -> Vec<f32> {
    vec![0.0; masks.pixels()]
}

pub fn brush_weight(distance: f32, softness: f32) -> f32 {
    if distance >= 1.0 {
        return 0.0;
    }
    let core = 1.0 - softness.clamp(0.0, 1.0);
    if distance <= core {
        return 1.0;
    }
    let t = (distance - core) / (1.0 - core).max(0.0001);
    (1.0 - t * t).powi(2)
}

/// Uses the same base mask, ordered stamps, softness and erasure as the renderer.
pub fn mask_overlay(
    image: &RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    target: Target,
) -> RgbaImage {
    mask_overlay_region(
        image,
        edit,
        seg,
        target,
        Crop {
            x: 0,
            y: 0,
            width: image.width(),
            height: image.height(),
        },
    )
}
pub fn mask_overlay_region(
    image: &RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    target: Target,
    crop: Crop,
) -> RgbaImage {
    let (w, h) = image.dimensions();
    if crate::geometry::active(edit, seg) {
        let mapping = crate::geometry::Mapping::new(edit, seg, (w, h));
        let mut output = vec![0u8; (crop.width * crop.height * 4) as usize];
        output.par_chunks_mut(4).enumerate().for_each(|(i, out)| {
            let (x, y) = (i as u32 % crop.width, i as u32 / crop.width);
            let uv = mapping.source([
                (crop.x + x) as f32 / w as f32,
                (crop.y + y) as f32 / h as f32,
            ]);
            let (sx, sy) = (
                (uv[0] * w as f32).clamp(0.0, (w - 1) as f32) as u32,
                (uv[1] * h as f32).clamp(0.0, (h - 1) as f32) as u32,
            );
            let data = seg.map(|s| match target {
                Target::Skin => &s.skin,
                Target::Teeth => &s.teeth,
                Target::Eyes => &s.eyes,
                Target::UnderEyes => &s.under_eyes,
                Target::Background => &s.background,
                _ => &s.skin,
            });
            let mut value = if target == Target::Skin && data.is_none_or(|d| d.is_empty()) {
                skin_weight(&image.get_pixel(sx, sy).0)
            } else {
                seg.zip(data)
                    .map_or(0.0, |(s, d)| sample_mask(d, s, sx, sy, w, h))
            };
            for stroke in edit.strokes.iter().filter(|s| s.target == target) {
                let distance = ((uv[0] - stroke.center[0]) * w as f32)
                    .hypot((uv[1] - stroke.center[1]) * h as f32)
                    / (stroke.radius * w.min(h) as f32).max(0.01);
                value = lerp(
                    value,
                    if stroke.erase { 0.0 } else { 1.0 },
                    brush_weight(distance, stroke.softness),
                );
            }
            out.copy_from_slice(&[255, 214, 10, (value * 90.0).round() as u8]);
        });
        return RgbaImage::from_raw(crop.width, crop.height, output).unwrap();
    }
    let mut masks: Vec<f32> = (0..crop.width * crop.height)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (crop.x + i % crop.width, crop.y + i / crop.width);
            let data = seg.map(|s| match target {
                Target::Skin => &s.skin,
                Target::Teeth => &s.teeth,
                Target::Eyes => &s.eyes,
                Target::Background => &s.background,
                Target::UnderEyes => &s.under_eyes,
                _ => &s.skin,
            });
            match target {
                Target::Skin if data.is_none_or(|d| d.is_empty()) => {
                    skin_weight(&image.get_pixel(x, y).0)
                }
                Target::Heal | Target::Liquify | Target::Clone | Target::Patch => 0.0,
                _ => seg
                    .zip(data)
                    .map_or(0.0, |(s, d)| sample_mask(d, s, x, y, w, h)),
            }
        })
        .collect();
    for stroke in edit.strokes.iter().filter(|s| s.target == target) {
        let cx = stroke.center[0] * w as f32;
        let cy = stroke.center[1] * h as f32;
        let radius = (stroke.radius * w.min(h) as f32).max(0.01);
        let x0 = ((cx - radius).floor().max(0.0) as u32).max(crop.x);
        let y0 = ((cy - radius).floor().max(0.0) as u32).max(crop.y);
        let x1 = ((cx + radius).ceil().min(w as f32) as u32).min(crop.x + crop.width);
        let y1 = ((cy + radius).ceil().min(h as f32) as u32).min(crop.y + crop.height);
        for y in y0..y1 {
            for x in x0..x1 {
                let weight = brush_weight(
                    (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) / radius,
                    stroke.softness,
                );
                let i = ((y - crop.y) * crop.width + x - crop.x) as usize;
                masks[i] = lerp(masks[i], if stroke.erase { 0.0 } else { 1.0 }, weight);
            }
        }
    }
    RgbaImage::from_fn(crop.width, crop.height, |x, y| {
        image::Rgba([
            255,
            214,
            10,
            (masks[(y * crop.width + x) as usize] * 90.0).round() as u8,
        ])
    })
}

#[derive(Debug)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Export cancelled")
    }
}
impl std::error::Error for Cancelled {}
fn check_cancel(cancel: Option<&AtomicBool>) -> Result<()> {
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Err(Cancelled.into());
    }
    Ok(())
}
/// Worker-owned, bounded reuse of immutable source/filter data between slider previews.
#[derive(Default)]
pub struct Renderer {
    sources: Vec<SourceBuffers>,
    native_sources: Vec<(Arc<RgbaImage>, tiles::NativeCache)>,
}
struct SourceBuffers {
    image: Arc<RgbaImage>,
    warp: crate::geometry::WarpCache,
    background: Option<(usize, Arc<Vec<[f16; 4]>>)>,
    linear: Option<Arc<Vec<[f32; 4]>>>,
    fine: Option<Arc<Vec<[f16; 4]>>>,
    low: Option<Arc<Vec<[f16; 4]>>>,
    masks: Option<MaskBuffers>,
    under_eye_lift: Option<UnderEyeBuffers>,
    cleanup: Option<CleanupBuffers>,
    flyaway: Option<FlyawayBuffers>,
}
struct FlyawayBuffers {
    seg: Option<Arc<Segmentation>>,
    data: Arc<crate::flyaway::FlyawayMap>,
}
struct MaskBuffers {
    targets: [bool; 5],
    strokes: SharedVec<Stroke>,
    seg: Option<Arc<Segmentation>>,
    data: Arc<LocalMasks>,
}
struct UnderEyeBuffers {
    masks: Arc<LocalMasks>,
    low: Arc<Vec<[f16; 4]>>,
    data: Arc<Vec<f32>>,
}
struct CleanupBuffers {
    strokes: SharedVec<Stroke>,
    clones: SharedVec<crate::cleanup::CloneStamp>,
    patches: SharedVec<crate::cleanup::PatchStroke>,
    healing: f32,
    data: Option<Arc<Vec<[f32; 4]>>>,
}
impl Renderer {
    pub fn render_region(
        &mut self,
        image: &Arc<RgbaImage>,
        edit: &Edit,
        seg: Option<&Arc<Segmentation>>,
        crop: Crop,
        cancel: Option<&AtomicBool>,
    ) -> Result<RgbaImage> {
        let index = self
            .native_sources
            .iter()
            .position(|(source, _)| Arc::ptr_eq(source, image));
        let mut entry = index
            .map(|i| self.native_sources.remove(i))
            .unwrap_or_else(|| (image.clone(), tiles::NativeCache::default()));
        let result = tiles::render_region_cached(
            image,
            edit,
            seg.map(Arc::as_ref),
            crop,
            cancel,
            Some(&mut entry.1),
            seg,
        );
        self.native_sources.push(entry);
        while self.native_sources.len() > 2
            || self
                .native_sources
                .iter()
                .map(|(_, data)| data.bytes())
                .sum::<usize>()
                > 64 * 1024 * 1024
        {
            self.native_sources.remove(0);
        }
        result
    }
    pub fn render(
        &mut self,
        image: &Arc<RgbaImage>,
        edit: &Edit,
        seg: Option<&Arc<Segmentation>>,
        cancel: Option<&AtomicBool>,
    ) -> Result<RgbaImage> {
        // Never retain full-resolution buffers for large exports; previews share at most 160 MiB.
        if image.width() as u64 * image.height() as u64 > 2_100_000 {
            return render_cancellable(image, edit, seg.map(Arc::as_ref), cancel);
        }
        let index = self
            .sources
            .iter()
            .position(|s| Arc::ptr_eq(&s.image, image));
        let mut buffers = index
            .map(|i| self.sources.remove(i))
            .unwrap_or_else(|| SourceBuffers {
                image: image.clone(),
                warp: Default::default(),
                background: None,
                linear: None,
                fine: None,
                low: None,
                masks: None,
                under_eye_lift: None,
                cleanup: None,
                flyaway: None,
            });
        let result = render_inner(
            image,
            edit,
            seg.map(Arc::as_ref),
            cancel,
            Some(&mut buffers),
            seg,
        );
        self.sources.push(buffers);
        while self.sources.len() > 2
            || self.sources.iter().map(SourceBuffers::bytes).sum::<usize>() > 160 * 1024 * 1024
        {
            self.sources.remove(0);
        }
        result
    }
}
impl SourceBuffers {
    fn bytes(&self) -> usize {
        let linear = self.linear.as_ref().map_or(0, |v| v.len() * 16);
        let blurred = self
            .fine
            .iter()
            .chain(self.low.iter())
            .map(|v| v.len() * std::mem::size_of::<[f16; 4]>())
            .sum::<usize>();
        linear
            + blurred
            + self.warp.bytes()
            + self
                .background
                .as_ref()
                .map_or(0, |(_, v)| v.len() * std::mem::size_of::<[f16; 4]>())
            + self.masks.as_ref().map_or(0, |m| m.data.bytes())
            + self
                .under_eye_lift
                .as_ref()
                .map_or(0, |m| m.data.len() * std::mem::size_of::<f32>())
            + self
                .cleanup
                .as_ref()
                .and_then(|m| m.data.as_ref())
                .map_or(0, |m| m.len() * 16)
            + self.flyaway.as_ref().map_or(0, |m| m.data.bytes())
    }
}

pub fn render(image: &RgbaImage, edit: &Edit, seg: Option<&Segmentation>) -> RgbaImage {
    render_cancellable(image, edit, seg, None).expect("uncancelled render")
}
pub fn render_cancellable(
    image: &RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    cancel: Option<&AtomicBool>,
) -> Result<RgbaImage> {
    if u64::from(image.width()) * u64::from(image.height()) > 2_100_000 {
        tiles::render(image, edit, seg, cancel)
    } else {
        render_inner(image, edit, seg, cancel, None, None)
    }
}

/// Render only a native-resolution viewport, retaining global brush and mask coordinates.
pub fn render_region(
    image: &RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    crop: Crop,
    cancel: Option<&AtomicBool>,
) -> Result<RgbaImage> {
    tiles::render_region(image, edit, seg, crop, cancel)
}
fn render_inner(
    image: &RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    cancel: Option<&AtomicBool>,
    mut cache: Option<&mut SourceBuffers>,
    shared_seg: Option<&Arc<Segmentation>>,
) -> Result<RgbaImage> {
    check_cancel(cancel)?;
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return Ok(image.clone());
    }
    let effective = edit.settings.effective();
    let s = &effective;
    let color_grade = crate::color::PreparedColor::new(&s.color, image);
    let flyaway = if s.flyaway_hairs == 0.0 {
        None
    } else {
        let make = || {
            seg.map_or_else(
                || Ok(Arc::new(crate::flyaway::FlyawayMap::default())),
                |seg| crate::flyaway::detect(image, seg, cancel).map(Arc::new),
            )
        };
        if let Some(c) = cache.as_mut() {
            let same = c
                .flyaway
                .as_ref()
                .is_some_and(|m| match (&m.seg, shared_seg) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                });
            if !same {
                c.flyaway = Some(FlyawayBuffers {
                    seg: shared_seg.cloned(),
                    data: make()?,
                });
            }
            c.flyaway.as_ref().map(|m| m.data.clone())
        } else {
            Some(make()?)
        }
    };
    let healing = s.healing != 0.0
        && edit
            .strokes
            .iter()
            .any(|s| s.target == Target::Heal && !s.erase);
    let cloning = edit.clones.iter().any(|s| s.strength != 0.0);
    let patching = edit.patches.iter().any(|s| s.strength != 0.0);
    let texture = s.smoothing != 0.0
        || s.tone_evenness != 0.0
        || s.under_eyes != 0.0
        || s.forehead != 0.0
        || s.laugh_lines != 0.0;
    let targets = [
        s.smoothing != 0.0 || s.tone_evenness != 0.0 || s.redness != 0.0,
        s.teeth != 0.0,
        s.eyes != 0.0,
        s.under_eyes != 0.0,
        s.background != Background::Original,
    ];
    let color = [
        s.exposure,
        s.contrast,
        s.shadows,
        s.highlights,
        s.warmth,
        s.tint,
        s.saturation,
        s.sharpening,
        s.vignette,
    ]
    .iter()
    .any(|v| *v != 0.0);
    if !texture
        && !color
        && !s.color.active()
        && s.flyaway_hairs == 0.0
        && !targets.iter().any(|v| *v)
        && s.blemishes == 0.0
        && s.contour == 0.0
        && s.face_highlight == 0.0
        && !healing
        && !cloning
        && !patching
    {
        let output = if let Some(c) = cache.as_mut() {
            crate::geometry::warp_cached(image.clone(), edit, seg, &mut c.warp)
        } else {
            crate::geometry::warp(image.clone(), edit, seg)
        };
        check_cancel(cancel)?;
        return Ok(output);
    }
    let make_linear = || {
        image
            .as_raw()
            .par_chunks_exact(4)
            .map(|p| {
                [
                    linear_byte(p[0]),
                    linear_byte(p[1]),
                    linear_byte(p[2]),
                    p[3] as f32 / 255.0,
                ]
            })
            .collect::<Vec<[f32; 4]>>()
    };
    let linear = if let Some(c) = cache.as_mut() {
        c.linear
            .get_or_insert_with(|| Arc::new(make_linear()))
            .clone()
    } else {
        Arc::new(make_linear())
    };
    let radius = (w.min(h) as f32 * 0.003).round().max(1.0) as usize;
    check_cancel(cancel)?;
    let fine = (texture || s.eyes != 0.0 || s.sharpening != 0.0 || healing).then(|| {
        let make = || Arc::new(blur(&linear, w as usize, h as usize, radius));
        if let Some(c) = cache.as_mut() {
            c.fine.get_or_insert_with(make).clone()
        } else {
            make()
        }
    });
    check_cancel(cancel)?;
    let low = texture.then(|| {
        let make = || Arc::new(blur(&linear, w as usize, h as usize, radius * 4));
        if let Some(c) = cache.as_mut() {
            c.low.get_or_insert_with(make).clone()
        } else {
            make()
        }
    });
    check_cancel(cancel)?;
    let background = if s.background == Background::Blur {
        let radius = (w.min(h) as f32 * s.background_blur / 3000.0)
            .round()
            .max(1.0) as usize;
        let make = || Arc::new(blur(&linear, w as usize, h as usize, radius));
        Some(if let Some(c) = cache.as_mut() {
            if c.background.as_ref().is_none_or(|(r, _)| *r != radius) {
                c.background = Some((radius, make()));
            }
            c.background.as_ref().unwrap().1.clone()
        } else {
            make()
        })
    } else {
        None
    };
    check_cancel(cancel)?;
    let make_masks = || Arc::new(local_layers(image, edit, seg, targets));
    let local_masks = if let Some(c) = cache.as_mut() {
        let same = c.masks.as_ref().is_some_and(|m| {
            m.targets == targets
                && m.strokes.same_storage(&edit.strokes)
                && match (&m.seg, shared_seg) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                }
        });
        if !same {
            c.masks = Some(MaskBuffers {
                targets,
                strokes: edit.strokes.clone(),
                seg: shared_seg.cloned(),
                data: make_masks(),
            });
        }
        c.masks.as_ref().unwrap().data.clone()
    } else {
        make_masks()
    };
    let under_eye_lift = if s.under_eyes == 0.0 {
        if let Some(c) = cache.as_mut() {
            c.under_eye_lift = None;
        }
        None
    } else {
        let low_map = low
            .as_ref()
            .expect("under-eye retouch creates a low-frequency map")
            .clone();
        if let Some(c) = cache.as_mut() {
            let same = c.under_eye_lift.as_ref().is_some_and(|m| {
                Arc::ptr_eq(&m.masks, &local_masks) && Arc::ptr_eq(&m.low, &low_map)
            });
            if same {
                c.under_eye_lift.as_ref().map(|m| m.data.clone())
            } else {
                let data = Arc::new(under_eye_shadow_lift(
                    low_map.as_slice(),
                    &local_masks,
                    seg,
                    w,
                    h,
                ));
                c.under_eye_lift = Some(UnderEyeBuffers {
                    masks: local_masks.clone(),
                    low: low_map,
                    data: data.clone(),
                });
                Some(data)
            }
        } else {
            Some(Arc::new(under_eye_shadow_lift(
                low_map.as_slice(),
                &local_masks,
                seg,
                w,
                h,
            )))
        }
    };
    let make_cleanup = || {
        crate::cleanup::apply(&linear, fine.as_ref().map(|f| f.as_slice()), w, h, edit)
            .map(Arc::new)
    };
    let healed = if let Some(c) = cache.as_mut() {
        let same = c.cleanup.as_ref().is_some_and(|m| {
            m.strokes.same_storage(&edit.strokes)
                && m.clones.same_storage(&edit.clones)
                && m.patches.same_storage(&edit.patches)
                && m.healing == s.healing
        });
        if !same {
            c.cleanup = Some(CleanupBuffers {
                strokes: edit.strokes.clone(),
                clones: edit.clones.clone(),
                patches: edit.patches.clone(),
                healing: s.healing,
                data: make_cleanup(),
            });
        }
        c.cleanup.as_ref().unwrap().data.clone()
    } else {
        make_cleanup()
    };
    check_cancel(cancel)?;
    let output = render_pixels(PixelPass {
        w,
        h,
        region: Crop {
            x: 0,
            y: 0,
            width: w,
            height: h,
        },
        s,
        color: &color_grade,
        texture,
        linear: &linear,
        fine: fine.as_ref().map(|f| f.as_slice()),
        low: low.as_ref().map(|l| l.as_slice()),
        background: background.as_ref().map(|b| b.as_slice()),
        masks: &local_masks,
        under_eye_lift: under_eye_lift.as_deref().map(Vec::as_slice),
        healed: healed.as_ref().map(|p| p.as_slice()),
        flyaway: flyaway.as_deref(),
        seg,
        cancel,
    });
    check_cancel(cancel)?;
    let image = RgbaImage::from_raw(w, h, output).expect("render dimensions are unchanged");
    let image = if let Some(c) = cache {
        crate::geometry::warp_cached(image, edit, seg, &mut c.warp)
    } else {
        crate::geometry::warp(image, edit, seg)
    };
    check_cancel(cancel)?;
    Ok(image)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportSize {
    Original,
    Web,
    Instagram,
    Story,
    LinkedIn,
}
impl ExportSize {
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original · full resolution",
            Self::Web => "Web · longest edge 2048",
            Self::Instagram => "Instagram · 1080 × 1350",
            Self::Story => "Story · 1080 × 1920",
            Self::LinkedIn => "LinkedIn · 1200 × 1200",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
pub fn output_dimensions(w: u32, h: u32, size: ExportSize) -> (u32, u32) {
    match size {
        ExportSize::Original => (w, h),
        ExportSize::Web => {
            let scale = (2048.0 / w.max(h) as f64).min(1.0);
            (
                (w as f64 * scale).round().max(1.0) as u32,
                (h as f64 * scale).round().max(1.0) as u32,
            )
        }
        ExportSize::Instagram => (1080, 1350),
        ExportSize::Story => (1080, 1920),
        ExportSize::LinkedIn => (1200, 1200),
    }
}
pub fn crop_rect(w: u32, h: u32, size: ExportSize, center: [f32; 2]) -> Crop {
    if matches!(size, ExportSize::Original | ExportSize::Web) {
        return Crop {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
    }
    let (ow, oh) = output_dimensions(w, h, size);
    let aspect = ow as f64 / oh as f64;
    let (cw, ch) = if w as f64 / h as f64 > aspect {
        ((h as f64 * aspect).round() as u32, h)
    } else {
        (w, (w as f64 / aspect).round() as u32)
    };
    let cw = cw.clamp(1, w);
    let ch = ch.clamp(1, h);
    Crop {
        x: (center[0] * w as f32 - cw as f32 / 2.0)
            .round()
            .clamp(0.0, (w - cw) as f32) as u32,
        y: (center[1] * h as f32 - ch as f32 / 2.0)
            .round()
            .clamp(0.0, (h - ch) as f32) as u32,
        width: cw,
        height: ch,
    }
}
pub fn resize_export(image: RgbaImage, size: ExportSize) -> RgbaImage {
    resize_export_at(image, size, [0.5; 2])
}
pub fn resize_export_at(image: RgbaImage, size: ExportSize, center: [f32; 2]) -> RgbaImage {
    let (w, h) = image.dimensions();
    let (ow, oh) = output_dimensions(w, h, size);
    let crop = crop_rect(w, h, size, center);
    let cropped =
        image::imageops::crop_imm(&image, crop.x, crop.y, crop.width, crop.height).to_image();
    if cropped.dimensions() == (ow, oh) {
        cropped
    } else {
        image::imageops::resize(&cropped, ow, oh, image::imageops::FilterType::Lanczos3)
    }
}
#[derive(Clone)]
pub struct ExportOptions {
    pub size: ExportSize,
    pub png: bool,
    pub quality: u8,
    pub center: [f32; 2],
    pub cancel: Option<Arc<AtomicBool>>,
}
struct CancelWriter<W: Write> {
    inner: W,
    cancel: Option<Arc<AtomicBool>>,
}
impl<W: Write> Write for CancelWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
        {
            return Err(std::io::Error::other(Cancelled));
        }
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Create-new provides race-safe, versioned exports and never overwrites a source.
pub fn export(
    photo: &Photo,
    edit: &Edit,
    seg: Option<&Segmentation>,
    directory: &Path,
    size: ExportSize,
    png: bool,
    quality: u8,
) -> Result<PathBuf> {
    export_with_options(
        photo,
        edit,
        seg,
        directory,
        &ExportOptions {
            size,
            png,
            quality,
            center: [0.5; 2],
            cancel: None,
        },
    )
}
pub fn export_with_options(
    photo: &Photo,
    edit: &Edit,
    seg: Option<&Segmentation>,
    directory: &Path,
    options: &ExportOptions,
) -> Result<PathBuf> {
    let ExportOptions {
        size,
        png,
        quality,
        center,
        cancel,
    } = options;
    let (size, png, quality, center) = (*size, *png, *quality, *center);
    check_cancel(cancel.as_deref())?;
    let rendered = render_cancellable(&photo.original, edit, seg, cancel.as_deref())?;
    let image = resize_export_at(rendered, size, center);
    check_cancel(cancel.as_deref())?;
    std::fs::create_dir_all(directory)?;
    let stem = Path::new(&photo.name)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let extension = if png { "png" } else { "jpg" };
    let (path, file) = (1..=10_000)
        .find_map(|v| {
            let path = directory.join(format!("{stem}_v{v}.{extension}"));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => Some(Ok((path, file))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .context("No available versioned filename")??;
    let result: Result<()> = (|| {
        let mut writer = BufWriter::new(CancelWriter {
            inner: file,
            cancel: cancel.clone(),
        });
        let icc = moxcms::ColorProfile::new_srgb().encode()?;
        if png {
            let mut encoder = image::codecs::png::PngEncoder::new(&mut writer);
            encoder.set_icc_profile(icc)?;
            encoder.write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )?;
        } else {
            // JPEG has no alpha. Composite on white in linear light.
            let mut rgb = vec![0u8; (image.width() * image.height() * 3) as usize];
            rgb.par_chunks_mut(3)
                .zip(image.as_raw().par_chunks_exact(4))
                .for_each(|(out, p)| {
                    if p[3] == 255 {
                        out.copy_from_slice(&p[..3]);
                        return;
                    }
                    let alpha = p[3] as f32 / 255.0;
                    for c in 0..3 {
                        out[c] = (linear_to_srgb(linear_byte(p[c]) * alpha + 1.0 - alpha) * 255.0)
                            .round() as u8;
                    }
                });
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut writer,
                quality.clamp(1, 100),
            );
            encoder.set_icc_profile(icc)?;
            encoder.encode(
                &rgb,
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgb8,
            )?;
        }
        writer.flush()?;
        check_cancel(cancel.as_deref())?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

#[derive(Clone, Default)]
pub struct History {
    past: VecDeque<Edit>,
    future: Vec<Edit>,
}

const HISTORY_INSTRUCTION_BYTES: usize = 64 * 1024 * 1024;

pub fn changelog(edit: &Edit) -> String {
    let effective = edit.settings.effective();
    let s = &effective;
    let mut result = format!(
        "Skin {} · tone {} · redness {} · auto blemishes {} ({} healing strokes) · under-eyes {} · teeth {} · eyes {} · exposure {:+.2} EV · contrast {} · highlights {} · shadows {} · warmth {} · tint {} · saturation {} · sharpening {} · vignette {} · background {:?}",
        s.smoothing,
        s.tone_evenness,
        s.redness,
        s.blemishes,
        edit.strokes
            .iter()
            .filter(|p| p.target == Target::Heal)
            .count(),
        s.under_eyes,
        s.teeth,
        s.eyes,
        s.exposure,
        s.contrast,
        s.highlights,
        s.shadows,
        s.warmth,
        s.tint,
        s.saturation,
        s.sharpening,
        s.vignette,
        s.background
    );
    result.push_str(&format!(" · healing strength {} · forehead {} · laugh lines {} · eye size {} · nose width {} · lip volume {} · jawline {} · contour {} · face highlight {} · {} warp segments · {} clone stamps · {} patches",s.healing,s.forehead,s.laugh_lines,s.eye_size,s.nose_width,s.lip_plumpness,s.jawline,s.contour,s.face_highlight,edit.warps.len(),edit.clones.len(),edit.patches.len()));
    if s.flyaway_hairs != 0.0 {
        result.push_str(&format!(" · flyaway cleanup {}", s.flyaway_hairs));
    }
    if s.color.hsl_strength != 0.0 {
        for (name, band) in crate::color::HUE_NAMES.iter().zip(&s.color.hsl) {
            if *band != crate::color::HueBand::default() {
                result.push_str(&format!(
                    " · {name} HSL hue {:+}°, saturation {:+}, luminance {:+} (amount {})",
                    band.hue, band.saturation, band.luminance, s.color.hsl_strength
                ));
            }
        }
    }
    if s.color.curves_strength != 0.0 {
        for (name, curve) in ["Luma", "Red", "Green", "Blue"].iter().zip(&s.color.curves) {
            if *curve != crate::color::IDENTITY_CURVE {
                result.push_str(&format!(
                    " · {name} curve {:?} (amount {})",
                    curve, s.color.curves_strength
                ));
            }
        }
    }
    if s.color.reference_strength != 0.0
        && let Some(reference) = &s.color.reference
    {
        result.push_str(&format!(
            " · reference match {} (amount {})",
            reference.name, s.color.reference_strength
        ));
    }
    result
}

pub fn export_report(
    directory: &Path,
    rows: &[(String, String, String, String)],
) -> Result<PathBuf> {
    let escape = |s: &str| s.replace('|', "\\|").replace(['\n', '\r'], " ");
    let mut text = String::from(
        "# Astra Retouch export\n\nOriginal files were preserved. Photo-local masks were not synchronized between subjects.\n\n| Photo | Output | Edits applied | Notes |\n| --- | --- | --- | --- |\n",
    );
    for (name, output, edits, notes) in rows {
        text.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            escape(name),
            escape(output),
            escape(edits),
            escape(notes)
        ));
    }
    for version in 1..=10_000 {
        let path = directory.join(format!("Astra-export-v{version}.md"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(text.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    bail!("No available report filename")
}
impl History {
    fn same_action_storage(a: &Edit, b: &Edit) -> bool {
        a.strokes.same_storage(&b.strokes)
            && a.warps.same_storage(&b.warps)
            && a.clones.same_storage(&b.clones)
            && a.patches.same_storage(&b.patches)
    }
    /// Snapshots contain edit instructions, never full-resolution pixels.
    pub fn snapshots(&self) -> (impl Iterator<Item = &Edit>, impl Iterator<Item = &Edit>) {
        (self.past.iter(), self.future.iter())
    }
    pub fn from_snapshots(past: Vec<Edit>, future: Vec<Edit>) -> Self {
        let mut history = Self {
            past: past
                .into_iter()
                .rev()
                .take(100)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
            future: future
                .into_iter()
                .rev()
                .take(100)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
        };
        history.trim_storage();
        history
    }
    pub fn depths(&self) -> (usize, usize) {
        (self.past.len(), self.future.len())
    }
    /// Deserialization reconstructs independent current-edit vectors. Reuse equal history storage
    /// so reopened sessions retain the same cheap snapshots as an uninterrupted editing session.
    pub fn share_current_storage(&self, current: &mut Edit) {
        for snapshot in self.past.iter().chain(self.future.iter()) {
            if snapshot.strokes == current.strokes {
                current.strokes = snapshot.strokes.clone();
            }
            if snapshot.warps == current.warps {
                current.warps = snapshot.warps.clone();
            }
            if snapshot.clones == current.clones {
                current.clones = snapshot.clones.clone();
            }
            if snapshot.patches == current.patches {
                current.patches = snapshot.patches.clone();
            }
        }
    }
    /// Counts each shared action allocation once, including lasso boundaries. Pixel caches are
    /// managed separately by the UI and source resolution does not enter the history budget.
    pub fn instruction_storage_bytes(&self) -> usize {
        fn allocation<T>(values: &Vec<T>, seen: &mut std::collections::HashSet<usize>) -> usize {
            if values.capacity() > 0 && seen.insert(values.as_ptr() as usize) {
                values.capacity() * std::mem::size_of::<T>()
            } else {
                0
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut bytes = 0;
        for edit in self.past.iter().chain(self.future.iter()) {
            bytes += allocation(&edit.strokes, &mut seen);
            bytes += allocation(&edit.warps, &mut seen);
            bytes += allocation(&edit.clones, &mut seen);
            let patch_bytes = allocation(&edit.patches, &mut seen);
            bytes += patch_bytes;
            if patch_bytes > 0 {
                for patch in &edit.patches {
                    bytes += allocation(&patch.boundary, &mut seen);
                }
            }
        }
        bytes
    }
    fn trim_storage(&mut self) {
        // Keep at least one undo/redo step even when that one instruction set exceeds the budget.
        while self.past.len() + self.future.len() > 1
            && self.instruction_storage_bytes() > HISTORY_INSTRUCTION_BYTES
        {
            if self.past.len() >= self.future.len() {
                self.past.pop_front();
            } else {
                self.future.remove(0);
            }
        }
    }
    pub fn record(&mut self, edit: Edit) {
        let new_storage = self
            .past
            .back()
            .is_none_or(|previous| !Self::same_action_storage(previous, &edit));
        let branched = !self.future.is_empty();
        self.past.push_back(edit);
        if self.past.len() > 100 {
            self.past.pop_front();
        }
        self.future.clear();
        // Slider changes reuse action allocations, so their record path stays constant time.
        if new_storage || branched {
            self.trim_storage();
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
    pub fn undo(&mut self, current: &mut Edit) -> bool {
        if let Some(previous) = self.past.pop_back() {
            let new_storage = !Self::same_action_storage(&previous, current);
            self.future.push(std::mem::replace(current, previous));
            if new_storage {
                self.trim_storage();
            }
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self, current: &mut Edit) -> bool {
        if let Some(next) = self.future.pop() {
            let new_storage = !Self::same_action_storage(&next, current);
            self.past.push_back(std::mem::replace(current, next));
            if new_storage {
                self.trim_storage();
            }
            true
        } else {
            false
        }
    }
}

struct PixelPass<'a> {
    w: u32,
    h: u32,
    region: Crop,
    s: &'a Settings,
    color: &'a crate::color::PreparedColor,
    texture: bool,
    linear: &'a [[f32; 4]],
    fine: Option<&'a [[f16; 4]]>,
    low: Option<&'a [[f16; 4]]>,
    background: Option<&'a [[f16; 4]]>,
    masks: &'a LocalMasks,
    under_eye_lift: Option<&'a [f32]>,
    healed: Option<&'a [[f32; 4]]>,
    flyaway: Option<&'a crate::flyaway::FlyawayMap>,
    seg: Option<&'a Segmentation>,
    cancel: Option<&'a AtomicBool>,
}
#[inline(never)]
fn render_pixels(pass: PixelPass<'_>) -> Vec<u8> {
    let PixelPass {
        w,
        h,
        region,
        s,
        color,
        texture,
        linear,
        fine,
        low,
        background,
        masks,
        under_eye_lift,
        healed,
        flyaway,
        seg,
        cancel,
    } = pass;
    let pass_settings = s;
    let mut output = vec![0u8; linear.len() * 4];
    let exposure = 2.0f32.powf(s.exposure);
    let solid = s.background_color.map(linear_byte);
    output
        .par_chunks_mut(16_384)
        .enumerate()
        .for_each(|(chunk_index, chunk)| {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return;
            }
            for (local, out) in chunk.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let i = chunk_index * 4096 + local;
                let x = region.x + i as u32 % region.width;
                let y = region.y + i as u32 / region.width;
                let uv = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
                let masks: [f32; 5] = std::array::from_fn(|channel| masks.value(i, channel));
                let fine = fine.as_ref().map_or(linear[i], |f| f[i].map(f16::to_f32));
                let low = low.as_ref().map_or(linear[i], |f| f[i].map(f16::to_f32));
                let mut p = healed.as_ref().map_or(linear[i], |pixels| pixels[i]);
                if let Some(flyaway) = flyaway {
                    let delta = flyaway.delta((y * w + x) as usize);
                    for c in 0..3 {
                        p[c] += delta[c] * s.flyaway_hairs / 100.0;
                    }
                }
                let skin = masks[0];
                let edge = if texture {
                    ((lum(&linear[i]) - lum(&low)).abs() * 12.0).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let region = |data: &[f32]| seg.map_or(0.0, |s| sample_mask(data, s, x, y, w, h));
                let (forehead, laugh, contour, highlight) = seg.map_or((0.0, 0.0, 0.0, 0.0), |s| {
                    (
                        if pass_settings.forehead != 0.0 {
                            region(&s.forehead)
                        } else {
                            0.0
                        },
                        if pass_settings.laugh_lines != 0.0 {
                            region(&s.laugh_lines)
                        } else {
                            0.0
                        },
                        if pass_settings.contour != 0.0 {
                            region(&s.contour)
                        } else {
                            0.0
                        },
                        if pass_settings.face_highlight != 0.0 {
                            region(&s.highlight)
                        } else {
                            0.0
                        },
                    )
                });
                let targeted =
                    (masks[3] * s.under_eyes + forehead * s.forehead + laugh * s.laugh_lines)
                        / 100.0;
                let neural = seg
                    .filter(|seg| {
                        !seg.neural_blend.is_empty()
                            && (skin * s.smoothing != 0.0 || targeted != 0.0)
                    })
                    .map(|seg| sample_seg_rgb(&seg.neural_blend, seg, uv[0], uv[1], 0.5));
                let repair = seg
                    .filter(|seg| s.blemishes != 0.0 && !seg.repair_delta.is_empty())
                    .map(|seg| sample_seg_rgb(&seg.repair_delta, seg, uv[0], uv[1], 0.0));
                for c in 0..3 {
                    if let Some(delta) = repair
                        && delta[c] != 0.0
                    {
                        let corrected =
                            (linear_to_srgb(p[c]) + delta[c] * s.blemishes / 100.0).clamp(0.0, 1.0);
                        p[c] = srgb_to_linear(corrected);
                    }
                    if let Some(blend) = neural {
                        let value = linear_to_srgb(p[c]).clamp(0.0, 1.0);
                        let strength =
                            (skin * s.smoothing / 100.0 + targeted * 0.8).clamp(0.0, 1.0);
                        let mg = 0.5 + (blend[c] - 0.5) * strength;
                        p[c] = srgb_to_linear(
                            ((1.0 - 2.0 * mg) * value * value + 2.0 * mg * value).clamp(0.0, 1.0),
                        );
                    }
                    // Separate medium-scale unevenness from fine texture; full strength is deliberately visible.
                    let soften = (skin * s.smoothing / 100.0 * 0.95 + targeted).clamp(0.0, 1.0);
                    p[c] -= (fine[c] - low[c]) * soften * (1.0 - edge * 0.6);
                    p[c] -= (linear[i][c] - fine[c])
                        * (skin * s.smoothing / 100.0 * 0.28 + targeted * 0.4).clamp(0.0, 0.6)
                        * (1.0 - edge * 0.6);
                    p[c] = lerp(
                        p[c],
                        low[c] + linear[i][c] - fine[c],
                        skin * s.tone_evenness / 100.0 * 0.65 * (1.0 - edge),
                    );
                }
                let l = lum(&p);
                p[0] = lerp(p[0], l + (p[0] - l) * 0.7, skin * s.redness / 100.0 * 0.5);
                for c in 0..3 {
                    p[c] = lerp(
                        p[c],
                        lerp(l, p[c], 0.6) + 0.06,
                        masks[1] * s.teeth / 100.0 * 0.85,
                    );
                    p[c] += masks[2] * s.eyes / 100.0 * (0.05 + (linear[i][c] - fine[c]) * 0.7);
                    p[c] *= 2.0f32.powf(
                        -contour * s.contour / 100.0 * 0.8
                            + highlight * s.face_highlight / 100.0 * 0.65,
                    );
                    p[c] *= exposure;
                    p[c] += s.shadows / 100.0 * 0.15 * (1.0 - l).powi(3);
                    p[c] += s.highlights / 100.0 * 0.2 * l.powi(2);
                    p[c] = (p[c] - 0.18) * (1.0 + s.contrast / 150.0) + 0.18;
                }
                p[0] *= 1.0 + s.warmth / 500.0;
                p[2] *= 1.0 - s.warmth / 500.0;
                p[1] *= 1.0 - s.tint / 600.0;
                let l = lum(&p);
                for c in 0..3 {
                    p[c] = lerp(l, p[c], 1.0 + s.saturation / 100.0);
                    p[c] += (linear[i][c] - fine[c]) * s.sharpening / 100.0 * 0.6;
                    let v = ((uv[0] - 0.5).powi(2) + (uv[1] - 0.5).powi(2)) * 1.5;
                    p[c] *= 1.0 - v * s.vignette / 100.0 * 0.65;
                }
                if let Some(lift) = under_eye_lift
                    .and_then(|map| map.get(i))
                    .copied()
                    .filter(|lift| *lift > 0.0)
                {
                    let lift = lift * s.under_eyes / 100.0;
                    let current = lum(&p);
                    let scale = (current + lift) / current.max(0.005);
                    for channel in &mut p[..3] {
                        *channel *= scale;
                    }
                }
                p = color.apply_linear(p);
                if s.background != Background::Original {
                    let b = masks[4];
                    match s.background {
                        Background::Blur => {
                            if let Some(bg) = &background {
                                for c in 0..3 {
                                    p[c] = lerp(p[c], bg[i][c].to_f32(), b);
                                }
                            }
                        }
                        Background::Solid => {
                            for (c, value) in p.iter_mut().enumerate().take(3) {
                                *value = lerp(*value, solid[c], b);
                            }
                        }
                        Background::Transparent => p[3] *= 1.0 - b,
                        Background::Original => {}
                    }
                }
                for c in 0..3 {
                    out[c] = linear_output_byte(p[c]);
                }
                out[3] = (p[3].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        });
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_region_masks_store_only_active_byte_channels() {
        let image = RgbaImage::from_pixel(3, 2, image::Rgba([120, 80, 70, 255]));
        let seg = Segmentation {
            width: 3,
            height: 2,
            skin: vec![0.0, 0.4, 1.0, 0.25, 0.5, 0.75].into(),
            under_eyes: vec![1.0, 0.75, 0.5, 0.25, 0.4, 0.0].into(),
            ..Default::default()
        };
        let masks = local_layers(
            &image,
            &Edit::default(),
            Some(&seg),
            [true, false, false, true, false],
        );

        assert_eq!(
            masks.bytes(),
            image.width() as usize * image.height() as usize * 2
        );
        assert_eq!(masks.value(1, 0), 102.0 / 255.0);
        assert_eq!(masks.value(1, 3), 191.0 / 255.0);
        assert_eq!(masks.value(1, 2), 0.0);
    }

    #[test]
    fn half_precision_blur_stays_close_to_full_precision_reference() {
        let (width, height, radius) = (11usize, 9usize, 2usize);
        let input: Vec<[f32; 4]> = (0..width * height)
            .map(|i| {
                [
                    (i * 17 % 251) as f32 / 250.0,
                    (i * 43 % 241) as f32 / 240.0,
                    (i * 71 % 239) as f32 / 238.0,
                    1.0,
                ]
            })
            .collect();
        let result = blur(&input, width, height, radius);
        let span = (radius * 2 + 1) as f32;
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0.0; 4];
                for dy in -(radius as isize)..=radius as isize {
                    for dx in -(radius as isize)..=radius as isize {
                        let sx = (x as isize + dx).clamp(0, width as isize - 1) as usize;
                        let sy = (y as isize + dy).clamp(0, height as isize - 1) as usize;
                        for (channel, value) in input[sy * width + sx].iter().enumerate() {
                            sum[channel] += value;
                        }
                    }
                }
                let actual = result[y * width + x].map(f16::to_f32);
                for channel in 0..4 {
                    let expected = sum[channel] / span.powi(2);
                    assert!((actual[channel] - expected).abs() < 0.001);
                }
            }
        }
    }

    #[test]
    fn automatic_face_cleanup_does_not_grade_unmasked_pixels() {
        let source = RgbaImage::from_fn(64, 96, |x, y| {
            image::Rgba([80 + (x % 7) as u8 * 20, 55 + (y % 5) as u8 * 30, 90, 255])
        });
        let mut seg = Segmentation {
            width: 64,
            height: 96,
            skin: vec![0.0; 64 * 96].into(),
            ..Default::default()
        };
        for y in 24..72 {
            for x in 16..48 {
                seg.skin[y * 64 + x] = 1.0;
            }
        }
        let mut edit = Edit::default();
        edit.settings.apply_auto_retouch();
        let result = render(&source, &edit, Some(&seg));
        assert_ne!(result, source);
        for (x, y, pixel) in source.enumerate_pixels() {
            if !(16..48).contains(&x) || !(24..72).contains(&y) {
                assert_eq!(result.get_pixel(x, y), pixel);
            }
        }
    }

    #[test]
    fn disabled_adjustments_keep_values_and_remove_their_effect() {
        let image = fixture();
        let mut edit = Edit::default();
        edit.settings.exposure = 1.0;
        assert_ne!(render(&image, &edit, None), image);
        edit.settings.disabled.push(Adjustment::Exposure);
        assert_eq!(render(&image, &edit, None), image);
        assert_eq!(edit.settings.exposure, 1.0);
        let saved = ron::ser::to_string(&edit).unwrap();
        assert_eq!(ron::from_str::<Edit>(&saved).unwrap(), edit);
    }

    #[test]
    fn softness_and_erasure_overlay_match_effective_mask() {
        assert_eq!(brush_weight(0.8, 0.0), 1.0);
        assert!(brush_weight(0.8, 1.0) < 0.2);
        let image = RgbaImage::from_pixel(100, 100, image::Rgba([100, 100, 100, 255]));
        let mut edit = Edit::default();
        edit.settings.teeth = 100.0;
        let stroke = Stroke {
            target: Target::Teeth,
            center: [0.5; 2],
            radius: 0.2,
            erase: false,
            softness: 0.0,
        };
        edit.strokes.push(stroke.clone());
        assert_ne!(
            render(&image, &edit, None).get_pixel(50, 50),
            image.get_pixel(50, 50)
        );
        edit.strokes.push(Stroke {
            erase: true,
            ..stroke
        });
        let overlay = mask_overlay(&image, &edit, None, Target::Teeth);
        assert_eq!(overlay.get_pixel(50, 50)[3], 0);
        assert_eq!(render(&image, &edit, None), image);
        let legacy = "(target:Teeth,center:(0.5,0.5),radius:0.2,erase:false)";
        assert_eq!(ron::from_str::<Stroke>(legacy).unwrap().softness, 1.0);
    }

    #[test]
    fn native_mask_region_matches_full_mask_including_erasure() {
        let image = RgbaImage::from_pixel(200, 300, image::Rgba([100, 100, 100, 255]));
        let mut edit = Edit::default();
        edit.strokes.push(Stroke {
            target: Target::Teeth,
            center: [0.5; 2],
            radius: 0.2,
            softness: 0.75,
            erase: false,
        });
        edit.strokes.push(Stroke {
            target: Target::Teeth,
            center: [0.55, 0.5],
            radius: 0.08,
            softness: 0.2,
            erase: true,
        });
        let region = Crop {
            x: 80,
            y: 120,
            width: 60,
            height: 50,
        };
        let full = mask_overlay(&image, &edit, None, Target::Teeth);
        assert_eq!(
            mask_overlay_region(&image, &edit, None, Target::Teeth, region),
            image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
                .to_image()
        );
    }

    #[test]
    fn social_crop_focus_changes_export_and_has_exact_output_dimensions() {
        let source = RgbaImage::from_fn(200, 100, |x, _| image::Rgba([x as u8, 20, 70, 255]));
        let left = resize_export_at(source.clone(), ExportSize::LinkedIn, [0.0, 0.5]);
        let right = resize_export_at(source, ExportSize::LinkedIn, [1.0, 0.5]);
        assert_eq!(left.dimensions(), (1200, 1200));
        assert!(right.get_pixel(600, 600)[0] > left.get_pixel(600, 600)[0] + 90);
        assert_eq!(
            crop_rect(200, 100, ExportSize::LinkedIn, [1.0, 0.5]),
            Crop {
                x: 100,
                y: 0,
                width: 100,
                height: 100
            }
        );
        assert_eq!(output_dimensions(6000, 4000, ExportSize::Web), (2048, 1365));
        assert_eq!(output_dimensions(600, 400, ExportSize::Web), (600, 400));
    }

    #[test]
    fn cancelled_export_creates_no_output_and_writer_stops_after_signal() {
        let directory = tempfile::tempdir().unwrap();
        let photo = photo_from_image("cancel.png".into(), None, fixture());
        let cancel = Arc::new(AtomicBool::new(true));
        let options = ExportOptions {
            size: ExportSize::Original,
            png: true,
            quality: 95,
            center: [0.5; 2],
            cancel: Some(cancel.clone()),
        };
        let err = export_with_options(&photo, &Edit::default(), None, directory.path(), &options)
            .unwrap_err();
        assert!(err.is::<Cancelled>());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        cancel.store(false, Ordering::Relaxed);
        let mut writer = CancelWriter {
            inner: Vec::new(),
            cancel: Some(cancel.clone()),
        };
        writer.write_all(&[1, 2, 3]).unwrap();
        cancel.store(true, Ordering::Relaxed);
        assert!(writer.write_all(&[4, 5, 6]).is_err());
        assert_eq!(writer.inner, vec![1, 2, 3]);
    }
    fn fixture() -> RgbaImage {
        RgbaImage::from_fn(31, 19, |x, y| {
            image::Rgba([(x * 8) as u8, (y * 13) as u8, ((x + y) * 5) as u8, 180])
        })
    }
    #[test]
    fn neutral_is_byte_identical() {
        let image = fixture();
        assert_eq!(render(&image, &Edit::default(), None), image);
    }
    #[test]
    fn edits_preserve_original_geometry_and_alpha() {
        let image = fixture();
        let original = image.clone();
        let edit = Edit {
            settings: presets()[0].settings.clone(),
            ..Default::default()
        };
        let result = render(&image, &edit, None);
        assert_eq!(image, original);
        assert_eq!(result.dimensions(), original.dimensions());
        assert!(result.pixels().all(|p| p[3] == 180));
        assert_ne!(result, image);
    }
    #[test]
    fn undo_redo_and_branch() {
        let mut history = History::default();
        let mut edit = Edit::default();
        history.record(edit.clone());
        edit.settings.smoothing = 40.0;
        assert!(history.undo(&mut edit));
        assert_eq!(edit.settings.smoothing, 0.0);
        assert!(history.redo(&mut edit));
        assert_eq!(edit.settings.smoothing, 40.0);
        history.undo(&mut edit);
        history.record(edit.clone());
        assert!(!history.can_redo());
    }
    #[test]
    fn masks_only_change_painted_regions() {
        let image = fixture();
        let mut edit = Edit::default();
        edit.settings.teeth = 100.0;
        assert_eq!(render(&image, &edit, None), image);
        edit.strokes.push(Stroke {
            target: Target::Teeth,
            center: [0.5, 0.5],
            radius: 0.2,
            erase: false,
            softness: 1.0,
        });
        let result = render(&image, &edit, None);
        assert_eq!(result.get_pixel(0, 0), image.get_pixel(0, 0));
        assert_ne!(result.get_pixel(15, 9), image.get_pixel(15, 9));
    }
    #[test]
    fn export_never_overwrites_and_retains_size() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("portrait.png");
        let image = fixture();
        image.save(&source).unwrap();
        let bytes = std::fs::read(&source).unwrap();
        let photo = load_photo(&source).unwrap();
        let a = export(
            &photo,
            &Edit::default(),
            None,
            dir.path(),
            ExportSize::Original,
            true,
            95,
        )
        .unwrap();
        let b = export(
            &photo,
            &Edit::default(),
            None,
            dir.path(),
            ExportSize::Original,
            true,
            95,
        )
        .unwrap();
        assert!(a.ends_with("portrait_v1.png"));
        assert!(b.ends_with("portrait_v2.png"));
        assert_eq!(std::fs::read(source).unwrap(), bytes);
        assert_eq!(image::open(a).unwrap().into_rgba8(), image);
    }
    #[test]
    fn transparent_background_respects_mask() {
        let image = fixture();
        let mut edit = Edit::default();
        edit.settings.background = Background::Transparent;
        edit.strokes.push(Stroke {
            target: Target::Background,
            center: [0.5, 0.5],
            radius: 0.3,
            erase: false,
            softness: 1.0,
        });
        let result = render(&image, &edit, None);
        assert_eq!(result.get_pixel(0, 0)[3], 180);
        assert!(result.get_pixel(15, 9)[3] < 5);
    }
    #[test]
    fn preset_serialization_round_trip() {
        let preset = &presets()[2];
        let data = ron::to_string(preset).unwrap();
        let loaded: Preset = ron::from_str(&data).unwrap();
        assert_eq!(loaded.settings, preset.settings);
    }
    #[test]
    fn linear_transfer_round_trip() {
        for v in 0..=255 {
            let f = v as f32 / 255.0;
            assert!((linear_to_srgb(srgb_to_linear(f)) - f).abs() < 0.00001);
        }
    }
    #[test]
    fn import_applies_exif_orientation_with_icc() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("rotated.jpg");
        let image = fixture();
        let file = std::fs::File::create(&source).unwrap();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, 100);
        encoder
            .set_icc_profile(moxcms::ColorProfile::new_srgb().encode().unwrap())
            .unwrap();
        // Little-endian TIFF EXIF with a single Orientation=6 entry.
        encoder
            .set_exif_metadata(vec![
                0x49, 0x49, 0x2a, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0,
                0, 0,
            ])
            .unwrap();
        let rgb = DynamicImage::ImageRgba8(image.clone()).into_rgb8();
        encoder
            .encode(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        let photo = load_photo(&source).unwrap();
        assert_eq!(photo.original.dimensions(), (19, 31));
        assert_eq!(photo.preview.dimensions(), (19, 31));
        let mut expected = image::open(&source).unwrap();
        expected.apply_orientation(image::metadata::Orientation::Rotate90);
        let expected = expected.into_rgba8();
        for (actual, expected) in photo.original.pixels().zip(expected.pixels()) {
            assert!(
                actual
                    .0
                    .iter()
                    .zip(expected.0)
                    .all(|(&a, b)| (a as i16 - b as i16).abs() <= 1)
            );
        }
    }
    #[test]
    fn mask_resolution_changes_preserve_other_targets() {
        let mask = Segmentation {
            width: 2,
            height: 2,
            skin: vec![0.0, 1.0, 0.0, 1.0].into(),
            background: vec![1.0, 0.0, 1.0, 0.0].into(),
            ..Default::default()
        };
        let resized = mask.resample(4, 4);
        assert_eq!(resized.skin.len(), 16);
        assert_eq!(resized.background.len(), 16);
        assert_eq!(resized.skin[0], 0.0);
        assert_eq!(resized.skin[3], 1.0);
        assert_eq!(resized.background[0], 1.0);
        assert_eq!(resized.background[3], 0.0);
        assert!(resized.eyes.is_empty());
    }

    #[test]
    fn compact_maps_preserve_fractional_edges_full_resolution_and_explicit_background_masks() {
        let (w, h) = (48u32, 37u32);
        let n = (w * h) as usize;
        let mut dense = Segmentation {
            width: w,
            height: h,
            skin: vec![0.; n].into(),
            teeth: vec![0.; n].into(),
            eyes: vec![0.; n].into(),
            under_eyes: vec![0.; n].into(),
            forehead: vec![0.; n].into(),
            laugh_lines: vec![0.; n].into(),
            contour: vec![0.; n].into(),
            highlight: vec![0.; n].into(),
            neural_blend: vec![[0.5; 3]; n].into(),
            repair_delta: vec![[0.; 3]; n].into(),
            background: vec![0.3; n].into(),
            ..Default::default()
        };
        for y in 8..29 {
            for x in 11..35 {
                let i = (y * w + x) as usize;
                dense.skin[i] = 0.8;
                dense.teeth[i] = 0.2;
                dense.eyes[i] = 0.4;
                dense.under_eyes[i] = 0.5;
                dense.forehead[i] = 0.1;
                dense.laugh_lines[i] = 0.6;
                dense.contour[i] = 0.3;
                dense.highlight[i] = 0.25;
                dense.neural_blend[i] = [0.43, 0.57, 0.61];
                dense.repair_delta[i] = [0.02, -0.01, 0.04];
            }
        }
        let mut compact = dense.clone();
        compact.compact_generated_maps();
        assert!(compact.map_bytes() < dense.map_bytes() / 2);
        assert_eq!(compact.background.len(), n);
        let large = dense.resample(83, 61);
        let small = compact.resample(83, 61);
        assert_eq!(large.skin, small.skin);
        assert_eq!(large.neural_blend, small.neural_blend);
        assert_eq!(large.repair_delta, small.repair_delta);
        let image = RgbaImage::from_fn(91, 73, |x, y| {
            image::Rgba([(x * 3 % 256) as u8, (y * 7 % 256) as u8, 110, 217])
        });
        let edit = Edit {
            settings: Settings {
                smoothing: 65.,
                blemishes: 80.,
                under_eyes: 70.,
                forehead: 50.,
                laugh_lines: 80.,
                contour: 70.,
                face_highlight: 50.,
                eyes: 35.,
                teeth: 40.,
                background: Background::Blur,
                ..presets()[0].settings.clone()
            },
            ..Default::default()
        };
        assert_eq!(
            render(&image, &edit, Some(&dense)),
            render(&image, &edit, Some(&compact))
        );
        let empty = Segmentation {
            width: w,
            height: h,
            skin: vec![0.; n].into(),
            ..Default::default()
        };
        let mut packed = empty.clone();
        packed.compact_generated_maps();
        assert_eq!(packed.skin.len(), 1);
        assert_eq!(
            render(&image, &edit, Some(&empty)),
            render(&image, &edit, Some(&packed))
        );
    }
    #[test]
    fn preview_reuse_tracks_slider_mask_clone_geometry_and_segmentation_changes() {
        let image = Arc::new(RgbaImage::from_fn(160, 192, |x, y| {
            image::Rgba([(x * 7 % 256) as u8, (y * 5 % 256) as u8, 80, 255])
        }));
        let mut seg = Arc::new(Segmentation {
            width: 160,
            height: 192,
            skin: vec![0.6; 160 * 192].into(),
            background: vec![0.4; 160 * 192].into(),
            ..Default::default()
        });
        let mut edit = Edit {
            settings: presets()[0].settings.clone(),
            ..Default::default()
        };
        let mut renderer = Renderer::default();
        for step in 0..9 {
            match step {
                1 => edit.settings.smoothing = 70.,
                2 => edit.settings.exposure = 0.3,
                3 => edit.strokes.push(Stroke {
                    target: Target::Skin,
                    center: [0.5; 2],
                    radius: 0.1,
                    erase: true,
                    softness: 0.5,
                }),
                4 => edit.clones.push(crate::cleanup::CloneStamp {
                    center: [0.4; 2],
                    source: [0.7, 0.6],
                    radius: 0.08,
                    softness: 0.5,
                    strength: 60.,
                }),
                5 => edit.warps.push(crate::geometry::WarpStroke {
                    center: [0.5; 2],
                    delta: [0.03, -0.01],
                    radius: 0.18,
                    softness: 0.7,
                    strength: 80.,
                }),
                6 => {
                    edit.settings.background = Background::Blur;
                    edit.settings.background_blur = 30.;
                }
                7 => edit.settings.background_blur = 100.,
                8 => Arc::make_mut(&mut seg).skin.fill(0.2),
                _ => {}
            }
            assert_eq!(
                renderer.render(&image, &edit, Some(&seg), None).unwrap(),
                render(&image, &edit, Some(&seg)),
                "stale preview at step {step}"
            );
        }
        let cancelled = AtomicBool::new(true);
        assert!(
            renderer
                .render(&image, &edit, Some(&seg), Some(&cancelled))
                .unwrap_err()
                .is::<Cancelled>()
        );
    }

    #[test]
    fn under_eye_lighting_map_is_reused_across_strength_changes() {
        let image = Arc::new(RgbaImage::from_fn(96, 112, |x, y| {
            image::Rgba([85 + (y % 28) as u8 * 3, 62 + (x % 24) as u8 * 3, 72, 255])
        }));
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
        let seg = Arc::new(Segmentation {
            width: 96,
            height: 112,
            faces: vec![crate::geometry::FaceMesh {
                landmarks,
                bounds: [0.2, 0.2, 0.6, 0.62],
                confidence: 0.9,
            }],
            skin: vec![1.0; 96 * 112].into(),
            under_eyes: vec![0.7; 96 * 112].into(),
            ..Default::default()
        });
        let mut edit = Edit::default();
        edit.settings.under_eyes = 20.0;
        let mut renderer = Renderer::default();
        let first = renderer.render(&image, &edit, Some(&seg), None).unwrap();
        let lift = renderer.sources[0]
            .under_eye_lift
            .as_ref()
            .unwrap()
            .data
            .clone();

        edit.settings.under_eyes = 70.0;
        let second = renderer.render(&image, &edit, Some(&seg), None).unwrap();
        let reused = &renderer.sources[0].under_eye_lift.as_ref().unwrap().data;
        assert!(Arc::ptr_eq(&lift, reused));
        assert_ne!(first, second);
    }

    #[test]
    fn parallel_import_analysis_has_consistent_statistics_on_large_uniform_photos() {
        let image = RgbaImage::from_pixel(1600, 2000, image::Rgba([130, 90, 80, 255]));
        let small = analyze(&RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([130, 90, 80, 255]),
        ));
        let large = analyze(&image);
        assert_eq!(large.mean, small.mean);
        assert_eq!(large.warmth, small.warmth);
        assert_eq!(large.sharpness, 0.);
        assert_eq!(large.clipped, 0.);
        assert_eq!(large.notes, small.notes);
    }

    #[test]
    fn fast_output_quantization_matches_float_transfer_at_boundaries_and_across_the_range() {
        let expected = |v: f32| (linear_to_srgb(v).clamp(0., 1.) * 255.).round() as u8;
        for i in 0..1_000_000 {
            let v = i as f32 / 999_999.;
            assert_eq!(linear_output_byte(v), expected(v));
        }
        for i in 0..256 {
            let v = srgb_to_linear((i as f32 + 0.5) / 255.);
            for offset in -10i32..=10 {
                let value = f32::from_bits(v.to_bits().wrapping_add_signed(offset));
                assert_eq!(linear_output_byte(value), expected(value));
            }
        }
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.3, 1.7] {
            assert_eq!(linear_output_byte(v), expected(v));
        }
    }
}
