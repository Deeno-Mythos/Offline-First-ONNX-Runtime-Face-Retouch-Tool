//! Deterministic linear-light edits. Originals are never mutated.
use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageDecoder, ImageEncoder, RgbaImage};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub smoothing: f32,
    pub blemishes: f32,
    pub tone_evenness: f32,
    pub redness: f32,
    pub teeth: f32,
    pub eyes: f32,
    pub under_eyes: f32,
    pub exposure: f32,
    pub contrast: f32,
    pub shadows: f32,
    pub highlights: f32,
    pub warmth: f32,
    pub tint: f32,
    pub saturation: f32,
    pub sharpening: f32,
    pub vignette: f32,
    pub background: Background,
    pub background_blur: f32,
    pub background_color: [u8; 3],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            smoothing: 0.0,
            blemishes: 45.0,
            tone_evenness: 0.0,
            redness: 0.0,
            teeth: 0.0,
            eyes: 0.0,
            under_eyes: 0.0,
            exposure: 0.0,
            contrast: 0.0,
            shadows: 0.0,
            highlights: 0.0,
            warmth: 0.0,
            tint: 0.0,
            saturation: 0.0,
            sharpening: 0.0,
            vignette: 0.0,
            background: Background::Original,
            background_blur: 35.0,
            background_color: [238, 238, 238],
        }
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
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub target: Target,
    pub center: [f32; 2],
    pub radius: f32,
    pub erase: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Edit {
    pub settings: Settings,
    pub strokes: Vec<Stroke>,
    pub preset: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub description: String,
    pub settings: Settings,
}

pub fn presets() -> Vec<Preset> {
    let natural = Settings {
        smoothing: 35.0,
        tone_evenness: 35.0,
        redness: 30.0,
        exposure: 0.08,
        shadows: 10.0,
        sharpening: 15.0,
        ..Settings::default()
    };
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
pub struct Segmentation {
    pub width: u32,
    pub height: u32,
    pub skin: Vec<f32>,
    pub teeth: Vec<f32>,
    pub eyes: Vec<f32>,
    pub background: Vec<f32>,
}

impl Segmentation {
    pub fn resample(&self, width: u32, height: u32) -> Self {
        let resize = |data: &[f32]| -> Vec<f32> {
            if data.is_empty() {
                return vec![];
            }
            (0..width * height)
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
        }
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
    let mut luma = 0.0;
    let mut warmth = 0.0;
    let mut clipped = 0;
    let mut edges = 0.0;
    let n = (image.width() * image.height()).max(1) as f32;
    for (x, y, p) in image.enumerate_pixels() {
        let l = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0;
        luma += l;
        warmth += (p[0] as f32 - p[2] as f32) / 255.0;
        if l > 0.98 || l < 0.015 {
            clipped += 1;
        }
        if x > 0 && y > 0 {
            let a = image.get_pixel(x - 1, y);
            let b = image.get_pixel(x, y - 1);
            edges +=
                ((p[1] as f32 - a[1] as f32).abs() + (p[1] as f32 - b[1] as f32).abs()) / 510.0;
        }
    }
    let mean = luma / n;
    let warmth = warmth / n;
    let sharpness = edges / n;
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
    if clipped as f32 / n > 0.06 {
        notes.push("Clipped tones · inspect before export".into());
    }
    notes.push("Local skin mask is color-based; inspect or refine with the brush".into());
    Analysis {
        mean,
        warmth,
        sharpness,
        clipped: clipped as f32 / n,
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
pub fn linear_to_srgb(v: f32) -> f32 {
    let v = v.max(0.0);
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}
fn lum(p: &[f32; 4]) -> f32 {
    p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722
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
    if data.len() != (seg.width * seg.height) as usize || data.is_empty() {
        return 0.0;
    }
    let sx = ((x as f32 + 0.5) / w as f32 * seg.width as f32) as u32;
    let sy = ((y as f32 + 0.5) / h as f32 * seg.height as f32) as u32;
    data[(sy.min(seg.height - 1) * seg.width + sx.min(seg.width - 1)) as usize].clamp(0.0, 1.0)
}

fn blur(input: &[[f32; 4]], w: usize, h: usize, radius: usize) -> Vec<[f32; 4]> {
    if radius == 0 {
        return input.to_vec();
    }
    let radius = radius.min(w.max(h));
    let mut horizontal = vec![[0.0; 4]; input.len()];
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
                    out[c] = sum[c] / n;
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
    let mut columns = vec![[0.0; 4]; input.len()];
    columns
        .par_chunks_mut(h)
        .enumerate()
        .for_each(|(x, column)| {
            let mut sum = [0.0; 4];
            for dy in -(radius as isize)..=radius as isize {
                let p = horizontal[dy.clamp(0, h as isize - 1) as usize * w + x];
                for c in 0..4 {
                    sum[c] += p[c];
                }
            }
            let n = (2 * radius + 1) as f32;
            for (y, out) in column.iter_mut().enumerate() {
                for c in 0..4 {
                    out[c] = sum[c] / n;
                }
                let a = horizontal
                    [(y as isize - radius as isize).clamp(0, h as isize - 1) as usize * w + x];
                let b = horizontal[(y + radius + 1).min(h - 1) * w + x];
                for c in 0..4 {
                    sum[c] += b[c] - a[c];
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

type LocalLayers = (Vec<[f32; 5]>, Option<Vec<[f32; 4]>>);

fn local_layers(
    image: &RgbaImage,
    edit: &Edit,
    seg: Option<&Segmentation>,
    linear: &[[f32; 4]],
    fine: &[[f32; 4]],
) -> LocalLayers {
    let (w, h) = image.dimensions();
    let mut masks: Vec<[f32; 5]> = image
        .as_raw()
        .par_chunks_exact(4)
        .enumerate()
        .map(|(i, pixel)| {
            let x = i as u32 % w;
            let y = i as u32 / w;
            let mut result = [skin_weight(pixel), 0.0, 0.0, 0.0, 0.0];
            if let Some(seg) = seg {
                if !seg.skin.is_empty() {
                    result[0] = sample_mask(&seg.skin, seg, x, y, w, h);
                }
                result[1] = sample_mask(&seg.teeth, seg, x, y, w, h);
                result[2] = sample_mask(&seg.eyes, seg, x, y, w, h);
                result[4] = sample_mask(&seg.background, seg, x, y, w, h);
            }
            result
        })
        .collect();
    let mut healed = edit
        .strokes
        .iter()
        .any(|s| s.target == Target::Heal)
        .then(|| linear.to_vec());
    for stroke in &edit.strokes {
        let cx = stroke.center[0] * w as f32;
        let cy = stroke.center[1] * h as f32;
        let radius = (stroke.radius * w.min(h) as f32).max(0.01);
        let x0 = (cx - radius).floor().max(0.0) as u32;
        let x1 = (cx + radius).ceil().min(w as f32) as u32;
        let y0 = (cy - radius).floor().max(0.0) as u32;
        let y1 = (cy + radius).ceil().min(h as f32) as u32;
        let mut ring = [0.0; 3];
        if stroke.target == Target::Heal {
            if stroke.erase {
                continue;
            }
            for j in 0..16 {
                let angle = j as f32 * std::f32::consts::TAU / 16.0;
                let sx = (cx + angle.cos() * (radius * 1.8).max(2.0)).clamp(0.0, (w - 1) as f32)
                    as usize;
                let sy = (cy + angle.sin() * (radius * 1.8).max(2.0)).clamp(0.0, (h - 1) as f32)
                    as usize;
                for c in 0..3 {
                    ring[c] += linear[sy * w as usize + sx][c] / 16.0;
                }
            }
        }
        for y in y0..y1 {
            for x in x0..x1 {
                let distance = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2))
                    / radius.powi(2);
                if distance >= 1.0 {
                    continue;
                }
                let weight = (1.0 - distance).powi(2);
                let i = (y * w + x) as usize;
                if stroke.target == Target::Heal {
                    if let Some(pixels) = &mut healed {
                        for c in 0..3 {
                            pixels[i][c] = lerp(
                                pixels[i][c],
                                ring[c] + (linear[i][c] - fine[i][c]) * 0.35,
                                weight * edit.settings.blemishes / 100.0,
                            );
                        }
                    }
                } else {
                    let target = match stroke.target {
                        Target::Skin => 0,
                        Target::Teeth => 1,
                        Target::Eyes => 2,
                        Target::UnderEyes => 3,
                        Target::Background => 4,
                        Target::Heal => unreachable!(),
                    };
                    masks[i][target] = lerp(
                        masks[i][target],
                        if stroke.erase { 0.0 } else { 1.0 },
                        weight,
                    );
                }
            }
        }
    }
    (masks, healed)
}

pub fn render(image: &RgbaImage, edit: &Edit, seg: Option<&Segmentation>) -> RgbaImage {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return image.clone();
    }
    let s = &edit.settings;
    let linear: Vec<[f32; 4]> = image
        .as_raw()
        .par_chunks_exact(4)
        .map(|p| {
            [
                srgb_to_linear(p[0] as f32 / 255.0),
                srgb_to_linear(p[1] as f32 / 255.0),
                srgb_to_linear(p[2] as f32 / 255.0),
                p[3] as f32 / 255.0,
            ]
        })
        .collect();
    let radius = (w.min(h) as f32 * 0.003).round().max(1.0) as usize;
    let fine = blur(&linear, w as usize, h as usize, radius);
    let low = blur(&linear, w as usize, h as usize, radius * 4);
    let background = if s.background == Background::Blur {
        Some(blur(
            &linear,
            w as usize,
            h as usize,
            (w.min(h) as f32 * s.background_blur / 3000.0)
                .round()
                .max(1.0) as usize,
        ))
    } else {
        None
    };
    let (local_masks, healed) = local_layers(image, edit, seg, &linear, &fine);
    let mut output = vec![0u8; linear.len() * 4];
    output.par_chunks_mut(4).enumerate().for_each(|(i, out)| {
        let x = i as u32 % w;
        let y = i as u32 / w;
        let uv = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
        let masks = local_masks[i];
        let mut p = healed.as_ref().map_or(linear[i], |pixels| pixels[i]);
        let skin = masks[0];
        let edge = ((lum(&linear[i]) - lum(&low[i])).abs() * 12.0).clamp(0.0, 1.0);
        for c in 0..3 {
            // Only soften the medium-frequency layer. Fine pores remain intact.
            p[c] -= (fine[i][c] - low[i][c]) * skin * s.smoothing / 100.0 * 0.4 * (1.0 - edge);
            p[c] = lerp(
                p[c],
                low[i][c] + linear[i][c] - fine[i][c],
                skin * s.tone_evenness / 100.0 * 0.18 * (1.0 - edge),
            );
        }
        let l = lum(&p);
        p[0] = lerp(p[0], l + (p[0] - l) * 0.7, skin * s.redness / 100.0 * 0.5);
        for c in 0..3 {
            p[c] = lerp(
                p[c],
                lerp(l, p[c], 0.6) + 0.06,
                masks[1] * s.teeth / 100.0 * 0.4,
            );
            p[c] += masks[2] * s.eyes / 100.0 * (0.05 + (linear[i][c] - fine[i][c]) * 0.7);
            p[c] += masks[3] * s.under_eyes / 100.0 * 0.05;
            p[c] *= 2.0f32.powf(s.exposure);
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
            p[c] += (linear[i][c] - fine[i][c]) * s.sharpening / 100.0 * 0.6;
            let v = ((uv[0] - 0.5).powi(2) + (uv[1] - 0.5).powi(2)) * 1.5;
            p[c] *= 1.0 - v * s.vignette / 100.0 * 0.65;
        }
        if s.background != Background::Original {
            let b = masks[4];
            match s.background {
                Background::Blur => {
                    if let Some(bg) = &background {
                        for c in 0..3 {
                            p[c] = lerp(p[c], bg[i][c], b);
                        }
                    }
                }
                Background::Solid => {
                    for (c, value) in p.iter_mut().enumerate().take(3) {
                        *value = lerp(
                            *value,
                            srgb_to_linear(s.background_color[c] as f32 / 255.0),
                            b,
                        );
                    }
                }
                Background::Transparent => p[3] *= 1.0 - b,
                Background::Original => {}
            }
        }
        for c in 0..3 {
            out[c] = (linear_to_srgb(p[c]).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        out[3] = (p[3].clamp(0.0, 1.0) * 255.0).round() as u8;
    });
    RgbaImage::from_raw(w, h, output).expect("render dimensions are unchanged")
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

pub fn resize_export(image: RgbaImage, size: ExportSize) -> RgbaImage {
    let image = DynamicImage::ImageRgba8(image);
    match size {
        ExportSize::Original => image.into_rgba8(),
        ExportSize::Web => {
            if image.width().max(image.height()) > 2048 {
                image
                    .resize(2048, 2048, image::imageops::FilterType::Lanczos3)
                    .into_rgba8()
            } else {
                image.into_rgba8()
            }
        }
        _ => {
            let (w, h) = match size {
                ExportSize::Instagram => (1080, 1350),
                ExportSize::Story => (1080, 1920),
                _ => (1200, 1200),
            };
            image
                .resize_to_fill(w, h, image::imageops::FilterType::Lanczos3)
                .into_rgba8()
        }
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
        let image = resize_export(render(&photo.original, edit, seg), size);
        let mut writer = BufWriter::new(file);
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
            let rgb: Vec<u8> = image
                .as_raw()
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| {
                    let alpha = p[3] as f32 / 255.0;
                    (0..3).map(move |c| {
                        (linear_to_srgb(srgb_to_linear(p[c] as f32 / 255.0) * alpha + 1.0 - alpha)
                            * 255.0)
                            .round() as u8
                    })
                })
                .collect();
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
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

#[derive(Default)]
pub struct History {
    past: Vec<Edit>,
    future: Vec<Edit>,
}

pub fn changelog(edit: &Edit) -> String {
    let s = &edit.settings;
    format!(
        "Skin {} · tone {} · redness {} · spot strength {} ({} spots) · under-eyes {} · teeth {} · eyes {} · exposure {:+.2} EV · contrast {} · highlights {} · shadows {} · warmth {} · tint {} · saturation {} · sharpening {} · vignette {} · background {:?}",
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
    )
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
    pub fn record(&mut self, edit: Edit) {
        self.past.push(edit);
        if self.past.len() > 100 {
            self.past.remove(0);
        }
        self.future.clear();
    }
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
    pub fn undo(&mut self, current: &mut Edit) -> bool {
        if let Some(previous) = self.past.pop() {
            self.future.push(std::mem::replace(current, previous));
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self, current: &mut Edit) -> bool {
        if let Some(next) = self.future.pop() {
            self.past.push(std::mem::replace(current, next));
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            skin: vec![0.0, 1.0, 0.0, 1.0],
            background: vec![1.0, 0.0, 1.0, 0.0],
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
}
