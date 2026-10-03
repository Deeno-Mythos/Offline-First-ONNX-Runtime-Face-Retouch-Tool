//! Shared geometry for native-pixel views and input gestures.
use eframe::egui::{Pos2, Rect, Vec2, vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    pub scale: f32,
    pub fit: bool,
    pub pan: [f32; 2],
}
impl Default for View {
    fn default() -> Self {
        Self {
            scale: 1.0,
            fit: true,
            pan: [0.0; 2],
        }
    }
}
impl View {
    pub const MIN: f32 = 0.01;
    pub const MAX: f32 = 8.0;
    pub fn layout(&mut self, viewport: Rect, dimensions: Vec2, dpi: f32) -> Rect {
        if self.fit {
            self.scale = (viewport.width() * dpi / dimensions.x)
                .min(viewport.height() * dpi / dimensions.y)
                * 0.96;
            self.pan = [0.0; 2];
        }
        let size = dimensions * self.scale / dpi;
        for (axis, limit) in [
            ((size.x - viewport.width()) * 0.5).max(0.0),
            ((size.y - viewport.height()) * 0.5).max(0.0),
        ]
        .into_iter()
        .enumerate()
        {
            self.pan[axis] = self.pan[axis].clamp(-limit, limit);
        }
        let rect = Rect::from_center_size(viewport.center() + vec2(self.pan[0], self.pan[1]), size);
        if (self.scale - self.scale.round()).abs() < 0.0001 {
            // A native-pixel view must also start on a physical pixel boundary.
            Rect::from_min_size((rect.min.to_vec2() * dpi).round().to_pos2() / dpi, size)
        } else {
            rect
        }
    }
    pub fn zoom_at(&mut self, scale: f32, anchor: Pos2, viewport: Rect) {
        let next = scale.clamp(Self::MIN, Self::MAX);
        let relative = anchor - viewport.center();
        self.pan = [
            (self.pan[0] - relative.x) * next / self.scale + relative.x,
            (self.pan[1] - relative.y) * next / self.scale + relative.y,
        ];
        self.scale = next;
        self.fit = false;
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Gesture {
    #[default]
    None,
    Pan,
    Divider,
    Brush,
    Source,
}
pub fn gesture_at(pos: Pos2, image: Rect, split: Option<f32>, brush: bool, space: bool) -> Gesture {
    if split.is_some_and(|s| {
        (pos.x - (image.left() + image.width() * s)).abs() <= 14.0
            && pos.y >= image.top()
            && pos.y <= image.bottom()
    }) {
        Gesture::Divider
    } else if brush && !space && image.contains(pos) {
        Gesture::Brush
    } else {
        Gesture::Pan
    }
}

/// Stamp along every segment, including fast pointer movements, in source-pixel space.
pub fn stamps(
    previous: Option<[f32; 2]>,
    next: [f32; 2],
    dimensions: [u32; 2],
    radius: f32,
) -> Vec<[f32; 2]> {
    let Some(start) = previous else {
        return vec![next];
    };
    let short = dimensions[0].min(dimensions[1]) as f32;
    let dx = (next[0] - start[0]) * dimensions[0] as f32 / short;
    let dy = (next[1] - start[1]) * dimensions[1] as f32 / short;
    let distance = dx.hypot(dy);
    if distance < radius * 0.15 {
        return vec![];
    }
    let count = (distance / (radius * 0.2).max(0.0001)).ceil() as usize;
    (1..=count)
        .map(|i| {
            let t = i as f32 / count as f32;
            [
                start[0] + (next[0] - start[0]) * t,
                start[1] + (next[1] - start[1]) * t,
            ]
        })
        .collect()
}

pub fn select(
    selected: &mut [bool],
    active: &mut usize,
    anchor: &mut usize,
    clicked: usize,
    shift: bool,
    control: bool,
) {
    if shift {
        let range = (*anchor).min(clicked)..=(*anchor).max(clicked);
        for (i, value) in selected.iter_mut().enumerate() {
            *value = range.contains(&i) || (control && *value);
        }
        *active = clicked;
    } else if control {
        selected[clicked] = !selected[clicked];
        *anchor = clicked;
    } else {
        selected.fill(false);
        selected[clicked] = true;
        *active = clicked;
        *anchor = clicked;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_pixels_and_anchor_and_bounds() {
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let dimensions = vec2(4000.0, 3000.0);
        let mut v = View::default();
        v.layout(viewport, dimensions, 1.25);
        assert!((v.scale - 0.24).abs() < 0.001);
        let anchor = viewport.center() + vec2(80.0, 60.0);
        let before = v.layout(viewport, dimensions, 1.25);
        let uv = (anchor - before.min) / before.size();
        v.zoom_at(1.0, anchor, viewport);
        let after = v.layout(viewport, dimensions, 1.25);
        assert_eq!(after.width(), 3200.0);
        // Native display snaps to physical pixels, with at most half a pixel
        // of anchor movement on either axis.
        let error = (anchor - (after.min + after.size() * uv)) * 1.25;
        assert!(error.x.abs() <= 0.501 && error.y.abs() <= 0.501);
        assert!((after.min.x * 1.25).fract().abs() < 0.001);
        v.pan = [100_000.0; 2];
        v.layout(viewport, dimensions, 1.25);
        assert_eq!(v.pan, [1200.0, 900.0]);
        v.zoom_at(999.0, anchor, viewport);
        assert_eq!(v.scale, View::MAX);
    }
    #[test]
    fn divider_wins_even_with_brush_or_space() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 600.0));
        assert_eq!(
            gesture_at(rect.center(), rect, Some(0.5), true, true),
            Gesture::Divider
        );
    }
    #[test]
    fn space_temporarily_overrides_the_active_brush_with_pan() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 600.0));
        let point = rect.center();
        assert_eq!(gesture_at(point, rect, None, true, false), Gesture::Brush);
        assert_eq!(gesture_at(point, rect, None, true, true), Gesture::Pan);
        assert_eq!(gesture_at(point, rect, None, false, true), Gesture::Pan);
    }
    #[test]
    fn fast_strokes_have_no_gaps_on_tall_photos() {
        let points = stamps(Some([0.1, 0.1]), [0.9, 0.9], [1000, 4000], 0.02);
        let mut last = [0.1, 0.1];
        for p in points {
            assert!((p[0] - last[0]).hypot((p[1] - last[1]) * 4.0) <= 0.0041);
            last = p;
        }
        assert_eq!(last, [0.9, 0.9]);
    }
    #[test]
    fn shift_and_control_selection() {
        let mut selected = [false; 5];
        let mut active = 0;
        let mut anchor = 0;
        select(&mut selected, &mut active, &mut anchor, 1, false, false);
        select(&mut selected, &mut active, &mut anchor, 3, true, false);
        assert_eq!(selected, [false, true, true, true, false]);
        assert_eq!(active, 3);
        select(&mut selected, &mut active, &mut anchor, 4, false, true);
        assert_eq!(active, 3);
        assert!(selected[4]);
    }
}
