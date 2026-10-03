//! Resolution-independent, editable masks attached to a layer rather than its pixels.
use crate::{
    engine::{self, Crop},
    shared::SharedVec,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerMask {
    /// White reveals the layer; black hides it. An absent mask is fully white.
    pub base: u8,
    pub strokes: SharedVec<MaskStroke>,
}

impl Default for LayerMask {
    fn default() -> Self {
        Self {
            base: 255,
            strokes: SharedVec::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskStroke {
    pub center: [f32; 2],
    pub radius: f32,
    pub softness: f32,
    pub strength: f32,
    pub reveal: bool,
}

impl LayerMask {
    pub fn sample_uv(&self, uv: [f32; 2], dimensions: (u32, u32)) -> u8 {
        let (w, h) = dimensions;
        let mut value = self.base as f32;
        let short = w.min(h) as f32;
        for stroke in &self.strokes {
            let radius = stroke.radius * short;
            if radius <= 0.0 || !radius.is_finite() {
                continue;
            }
            let distance = ((uv[0] - stroke.center[0]) * w as f32)
                .hypot((uv[1] - stroke.center[1]) * h as f32)
                / radius;
            let weight = engine::brush_weight(distance, stroke.softness)
                * (stroke.strength / 100.0).clamp(0.0, 1.0);
            let goal = if stroke.reveal { 255.0 } else { 0.0 };
            value += (goal - value) * weight;
        }
        value.round().clamp(0.0, 255.0) as u8
    }
    pub fn invert(&mut self) {
        self.base = 255 - self.base;
        for stroke in self.strokes.iter_mut() {
            stroke.reveal = !stroke.reveal;
        }
    }

    pub fn coverage(&self, dimensions: (u32, u32), crop: Crop) -> Vec<u8> {
        let (w, h) = dimensions;
        let mut pixels = vec![self.base; crop.width as usize * crop.height as usize];
        if w == 0 || h == 0 || crop.width == 0 || crop.height == 0 {
            return pixels;
        }
        let short = w.min(h) as f32;
        for stroke in &self.strokes {
            let cx = stroke.center[0] * w as f32;
            let cy = stroke.center[1] * h as f32;
            let radius = stroke.radius * short;
            if !cx.is_finite() || !cy.is_finite() || !radius.is_finite() || radius <= 0.0 {
                continue;
            }
            let x0 = ((cx - radius).floor().max(crop.x as f32) as u32).min(crop.x + crop.width);
            let x1 = ((cx + radius).ceil().min((crop.x + crop.width) as f32) as u32).max(x0);
            let y0 = ((cy - radius).floor().max(crop.y as f32) as u32).min(crop.y + crop.height);
            let y1 = ((cy + radius).ceil().min((crop.y + crop.height) as f32) as u32).max(y0);
            for y in y0..y1 {
                for x in x0..x1 {
                    let distance = ((x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy)) / radius;
                    let weight = engine::brush_weight(distance, stroke.softness)
                        * (stroke.strength / 100.0).clamp(0.0, 1.0);
                    if weight <= 0.0 {
                        continue;
                    }
                    let index = ((y - crop.y) * crop.width + x - crop.x) as usize;
                    let before = pixels[index] as f32;
                    let goal = if stroke.reveal { 255.0 } else { 0.0 };
                    pixels[index] = (before + (goal - before) * weight).round() as u8;
                }
            }
        }
        pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_hide_invert_and_native_crop_match_full_mask() {
        let mut mask = LayerMask::default();
        mask.strokes.push(MaskStroke {
            center: [0.5, 0.5],
            radius: 0.16,
            softness: 0.5,
            strength: 100.0,
            reveal: false,
        });
        mask.strokes.push(MaskStroke {
            center: [0.52, 0.48],
            radius: 0.05,
            softness: 0.7,
            strength: 60.0,
            reveal: true,
        });
        let full = mask.coverage(
            (200, 300),
            Crop {
                x: 0,
                y: 0,
                width: 200,
                height: 300,
            },
        );
        let crop = Crop {
            x: 60,
            y: 90,
            width: 100,
            height: 120,
        };
        let local = mask.coverage((200, 300), crop);
        for y in 0..crop.height {
            for x in 0..crop.width {
                assert_eq!(
                    local[(y * crop.width + x) as usize],
                    full[((crop.y + y) * 200 + crop.x + x) as usize]
                );
            }
        }
        assert_eq!(full[0], 255);
        assert!(full[150 * 200 + 100] < 255);
        mask.invert();
        let inverse = mask.coverage(
            (200, 300),
            Crop {
                x: 0,
                y: 0,
                width: 200,
                height: 300,
            },
        );
        for (a, b) in full.into_iter().zip(inverse) {
            assert!(a.abs_diff(255 - b) <= 1);
        }
    }
}
