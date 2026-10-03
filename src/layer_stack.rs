//! Ordered, independently editable retouch and adjustment layers over an immutable photograph.
use crate::{engine::Edit, layer_mask::LayerMask, shared::SharedVec};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Difference,
}
impl BlendMode {
    pub const ALL: [Self; 4] = [Self::Normal, Self::Multiply, Self::Screen, Self::Difference];
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Multiply => "Multiply",
            Self::Screen => "Screen",
            Self::Difference => "Difference",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerType {
    #[default]
    Retouch,
    Adjustment,
    OriginalCopy,
}

impl LayerType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Retouch => "Retouch",
            Self::Adjustment => "Adjustment",
            Self::OriginalCopy => "Image copy",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerColor {
    #[default]
    None,
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Violet,
}
impl LayerColor {
    pub const ALL: [Self; 7] = [
        Self::None,
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Violet,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "No color",
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Violet => "Violet",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    pub kind: LayerType,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub color: LayerColor,
    pub mask: Option<LayerMask>,
    pub edit: Box<Edit>,
}
impl Default for Layer {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Retouch".into(),
            kind: LayerType::Retouch,
            visible: true,
            locked: false,
            opacity: 100.0,
            blend: BlendMode::Normal,
            color: LayerColor::None,
            mask: None,
            edit: Box::default(),
        }
    }
}
impl Layer {
    pub fn factor(&self) -> f32 {
        if self.visible && self.opacity.is_finite() {
            self.opacity.clamp(0.0, 100.0) / 100.0
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerStack {
    /// Bottom to top. The original background is implicit and permanently locked.
    pub layers: SharedVec<Layer>,
    pub active: u64,
    pub next_id: u64,
    pub solo: Option<SoloState>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoloState {
    pub layer: u64,
    pub visibility: Vec<(u64, bool)>,
}

impl Edit {
    pub fn flat_edit(&self) -> Self {
        let mut flat = self.clone();
        flat.stack = LayerStack::default();
        flat
    }
    pub fn ensure_stack(&mut self) {
        if self.stack.next_id == 0 {
            let layer = Layer {
                id: 1,
                edit: Box::new(self.flat_edit()),
                ..Default::default()
            };
            self.stack = LayerStack {
                layers: vec![layer].into(),
                active: 1,
                next_id: 2,
                solo: None,
            };
        }
    }
    pub fn active_layer(&self) -> Option<&Layer> {
        self.stack
            .layers
            .iter()
            .find(|layer| layer.id == self.stack.active)
    }
    pub fn active_layer_mut(&mut self) -> Option<&mut Layer> {
        self.stack
            .layers
            .iter_mut()
            .find(|layer| layer.id == self.stack.active)
    }
    pub fn layer_locked(&self) -> bool {
        if self.stack.next_id > 0 {
            self.active_layer().is_none_or(|layer| layer.locked)
        } else {
            false
        }
    }
    pub fn sync_active_layer(&mut self) {
        if self.stack.layers.is_empty() {
            return;
        }
        let flat = self.flat_edit();
        if let Some(layer) = self.active_layer_mut() {
            *layer.edit = flat;
        }
    }
    pub fn select_layer(&mut self, id: u64) -> bool {
        if id == self.stack.active
            || (id != 0 && !self.stack.layers.iter().any(|layer| layer.id == id))
        {
            return false;
        }
        self.sync_active_layer();
        let flat = self
            .stack
            .layers
            .iter()
            .find(|layer| layer.id == id)
            .map_or_else(Edit::default, |layer| layer.edit.flat_edit());
        let mut stack = self.stack.clone();
        stack.active = id;
        *self = flat;
        self.stack = stack;
        true
    }
    pub fn add_layer(&mut self, kind: LayerType) -> u64 {
        self.ensure_stack();
        self.restore_solo();
        self.sync_active_layer();
        let id = self.stack.next_id.max(2);
        self.stack.next_id = id + 1;
        let name = match kind {
            LayerType::Retouch => format!("Retouch {id}"),
            LayerType::Adjustment => format!("Adjustment {id}"),
            LayerType::OriginalCopy => "Background copy".into(),
        };
        let index = self
            .stack
            .layers
            .iter()
            .position(|layer| layer.id == self.stack.active)
            .map_or(self.stack.layers.len(), |i| i + 1);
        self.stack.layers.insert(
            index,
            Layer {
                id,
                name,
                kind,
                ..Default::default()
            },
        );
        self.select_layer(id);
        id
    }
    pub fn duplicate_layer(&mut self) -> Option<u64> {
        self.restore_solo();
        self.sync_active_layer();
        let index = self
            .stack
            .layers
            .iter()
            .position(|layer| layer.id == self.stack.active)?;
        let mut layer = self.stack.layers[index].clone();
        let id = self.stack.next_id;
        self.stack.next_id += 1;
        layer.id = id;
        layer.name = format!("{} copy", layer.name);
        layer.locked = false;
        self.stack.layers.insert(index + 1, layer);
        self.select_layer(id);
        Some(id)
    }
    pub fn delete_layer(&mut self) -> bool {
        if self.layer_locked() {
            return false;
        }
        self.restore_solo();
        let Some(index) = self
            .stack
            .layers
            .iter()
            .position(|layer| layer.id == self.stack.active)
        else {
            return false;
        };
        if self.stack.layers.len() == 1 {
            let flat = Edit::default();
            let mut stack = self.stack.clone();
            stack.layers.clear();
            stack.active = 0;
            *self = flat;
            self.stack = stack;
        } else {
            self.stack.layers.remove(index);
            // Do not save the deleted layer into its replacement.
            self.stack.active = 0;
            self.select_layer(self.stack.layers[index.min(self.stack.layers.len() - 1)].id);
        }
        true
    }
    pub fn move_layer(&mut self, upward: bool) -> bool {
        if self.layer_locked() {
            return false;
        }
        let Some(index) = self
            .stack
            .layers
            .iter()
            .position(|layer| layer.id == self.stack.active)
        else {
            return false;
        };
        let next = if upward {
            index + 1
        } else {
            index.saturating_sub(1)
        };
        if next >= self.stack.layers.len() || next == index {
            return false;
        }
        self.stack.layers.swap(index, next);
        true
    }
    pub fn reorder_layer(&mut self, from: u64, to: u64) -> bool {
        let Some(a) = self
            .stack
            .layers
            .iter()
            .position(|l| l.id == from && !l.locked)
        else {
            return false;
        };
        let Some(b) = self.stack.layers.iter().position(|l| l.id == to) else {
            return false;
        };
        if a == b {
            return false;
        }
        let layer = self.stack.layers.remove(a);
        self.stack.layers.insert(b, layer);
        true
    }
    pub fn resolved_layers(&self) -> Vec<Layer> {
        self.stack
            .layers
            .iter()
            .filter(|layer| layer.factor() > 0.0)
            .map(|layer| {
                let mut resolved = layer.clone();
                resolved.edit = Box::new(if layer.id == self.stack.active {
                    self.flat_edit()
                } else {
                    layer.edit.flat_edit()
                });
                resolved
            })
            .collect()
    }

    pub fn restore_solo(&mut self) -> bool {
        let Some(solo) = self.stack.solo.take() else {
            return false;
        };
        for layer in self.stack.layers.iter_mut() {
            if let Some((_, visible)) = solo.visibility.iter().find(|(id, _)| *id == layer.id) {
                layer.visible = *visible;
            }
        }
        true
    }

    /// Alt-clicking an eye previews one layer and restores the exact previous visibility set.
    pub fn solo_layer(&mut self, id: u64) -> bool {
        if !self.stack.layers.iter().any(|layer| layer.id == id) {
            return false;
        }
        if self
            .stack
            .solo
            .as_ref()
            .is_some_and(|solo| solo.layer == id)
        {
            return self.restore_solo();
        }
        let visibility = self.stack.solo.as_ref().map_or_else(
            || {
                self.stack
                    .layers
                    .iter()
                    .map(|layer| (layer.id, layer.visible))
                    .collect()
            },
            |solo| solo.visibility.clone(),
        );
        for layer in self.stack.layers.iter_mut() {
            layer.visible = layer.id == id;
        }
        self.stack.solo = Some(SoloState {
            layer: id,
            visibility,
        });
        true
    }

    pub fn toggle_layer_visibility(&mut self, id: u64) -> bool {
        if !self.stack.layers.iter().any(|layer| layer.id == id) {
            return false;
        }
        self.restore_solo();
        let layer = self
            .stack
            .layers
            .iter_mut()
            .find(|layer| layer.id == id)
            .unwrap();
        layer.visible = !layer.visible;
        true
    }
    pub fn portrait_demand(&self) -> crate::nullstate::PortraitDemand {
        if self.stack.layers.is_empty() {
            return crate::nullstate::PortraitDemand::from_settings(&self.effective_settings());
        }
        self.resolved_layers().iter().fold(
            crate::nullstate::PortraitDemand::NONE,
            |demand, layer| {
                demand.union(crate::nullstate::PortraitDemand::from_settings(
                    &layer.edit.effective_settings(),
                ))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        engine::{self, Crop, Renderer},
        geometry::WarpStroke,
        layer_mask::MaskStroke,
    };
    use image::RgbaImage;
    use std::sync::Arc;

    #[test]
    fn painted_mask_restores_original_pixels_in_full_and_native_layer_renders() {
        let source = Arc::new(RgbaImage::from_fn(128, 160, |x, y| {
            image::Rgba([80 + (x % 21) as u8, 65 + (y % 17) as u8, 70, 255])
        }));
        let mut edit = Edit::default();
        edit.settings.exposure = 0.7;
        edit.ensure_stack();
        let unmasked = Renderer::default()
            .render(&source, &edit, None, None)
            .unwrap();
        let mask = LayerMask {
            strokes: vec![MaskStroke {
                center: [0.5, 0.5],
                radius: 0.2,
                softness: 0.5,
                strength: 100.0,
                reveal: false,
            }]
            .into(),
            ..Default::default()
        };
        edit.active_layer_mut().unwrap().mask = Some(mask);
        let mut renderer = Renderer::default();
        let result = renderer.render(&source, &edit, None, None).unwrap();
        assert_eq!(result.get_pixel(64, 80), source.get_pixel(64, 80));
        assert_eq!(result.get_pixel(2, 2), unmasked.get_pixel(2, 2));
        assert_ne!(result.get_pixel(64, 80), unmasked.get_pixel(64, 80));
        let region = Crop {
            x: 31,
            y: 39,
            width: 71,
            height: 82,
        };
        assert_eq!(
            renderer
                .render_region(&source, &edit, None, region, None)
                .unwrap(),
            image::imageops::crop_imm(&result, region.x, region.y, region.width, region.height)
                .to_image()
        );
        let saved = ron::ser::to_string(&edit).unwrap();
        assert_eq!(ron::from_str::<Edit>(&saved).unwrap(), edit);
    }

    #[test]
    fn solo_mutations_restore_visibility_and_labels_preserve_pixels_and_legacy_defaults() {
        let image = RgbaImage::from_pixel(32, 40, image::Rgba([100, 90, 80, 255]));
        let mut edit = Edit::default();
        edit.settings.exposure = 0.25;
        edit.ensure_stack();
        edit.add_layer(LayerType::Adjustment);
        edit.add_layer(LayerType::Retouch);
        edit.stack.layers[0].visible = false;
        let before = engine::render(&image, &edit, None);
        edit.active_layer_mut().unwrap().color = LayerColor::Blue;
        edit.active_layer_mut().unwrap().name = "Polish".into();
        assert_eq!(engine::render(&image, &edit, None), before);
        assert!(edit.solo_layer(2));
        assert!(!edit.solo_layer(999));
        edit.add_layer(LayerType::Retouch);
        assert!(edit.stack.solo.is_none());
        assert!(!edit.stack.layers[0].visible);
        assert!(edit.stack.layers[1].visible && edit.stack.layers[2].visible);
        edit.solo_layer(2);
        edit.toggle_layer_visibility(3);
        assert!(edit.stack.solo.is_none());
        assert!(!edit.stack.layers[0].visible && !edit.stack.layers[2].visible);
        edit.solo_layer(2);
        edit.select_layer(2);
        edit.delete_layer();
        assert!(edit.stack.solo.is_none());
        assert!(!edit.stack.layers[0].visible);
        let old: Layer=ron::from_str("(id:1,name:\"Legacy\",kind:Retouch,visible:true,locked:false,opacity:100.0,blend:Normal,edit:())").unwrap();
        assert_eq!(old.color, LayerColor::None);
        let old_stack: LayerStack = ron::from_str("(layers:[],active:0,next_id:2)").unwrap();
        assert!(old_stack.solo.is_none());
    }

    #[test]
    fn stack_migration_and_layer_selection_preserve_pixels_and_independent_settings() {
        let image = RgbaImage::from_pixel(64, 80, image::Rgba([90, 80, 75, 255]));
        let mut edit = Edit::default();
        edit.settings.exposure = 0.5;
        let legacy = engine::render(&image, &edit, None);
        edit.ensure_stack();
        assert_eq!(engine::render(&image, &edit, None), legacy);
        let id = edit.add_layer(LayerType::Adjustment);
        assert_eq!(edit.settings.exposure, 0.0);
        edit.settings.contrast = 30.0;
        let composite = engine::render(&image, &edit, None);
        edit.select_layer(1);
        assert_eq!(edit.settings.exposure, 0.5);
        assert_eq!(edit.settings.contrast, 0.0);
        assert_eq!(engine::render(&image, &edit, None), composite);
        edit.select_layer(id);
        assert_eq!(edit.settings.contrast, 30.0);
        edit.select_layer(0);
        assert!(edit.layer_locked());
        assert_eq!(engine::render(&image, &edit, None), composite);
    }
    #[test]
    fn stack_duplicate_reorder_visibility_opacity_and_delete_change_the_composite() {
        let image = RgbaImage::from_pixel(64, 80, image::Rgba([100, 90, 80, 255]));
        let mut edit = Edit::default();
        edit.settings.exposure = 0.5;
        edit.ensure_stack();
        let single = engine::render(&image, &edit, None);
        let copy = edit.duplicate_layer().unwrap();
        assert_ne!(engine::render(&image, &edit, None), single);
        edit.active_layer_mut().unwrap().visible = false;
        assert_eq!(engine::render(&image, &edit, None), single);
        edit.active_layer_mut().unwrap().visible = true;
        edit.active_layer_mut().unwrap().opacity = 0.0;
        assert_eq!(engine::render(&image, &edit, None), single);
        assert!(edit.delete_layer());
        assert_eq!(edit.stack.active, 1);
        let original = edit.add_layer(LayerType::OriginalCopy);
        assert_eq!(engine::render(&image, &edit, None), image);
        assert!(edit.reorder_layer(original, 1));
        assert_eq!(engine::render(&image, &edit, None), single);
        assert!(!edit.reorder_layer(copy, 1));
        edit.select_layer(1);
        edit.active_layer_mut().unwrap().locked = true;
        assert!(!edit.delete_layer());
        assert!(!edit.move_layer(true));
        edit.active_layer_mut().unwrap().locked = false;
        edit.delete_layer();
        edit.delete_layer();
        assert!(edit.stack.layers.is_empty());
        edit.ensure_stack();
        assert!(edit.stack.layers.is_empty());
        assert_eq!(engine::render(&image, &edit, None), image);
        assert!(edit.add_layer(LayerType::Retouch) > original);
    }
    #[test]
    fn stack_native_regions_cached_previews_and_png_export_match_the_ordered_full_render() {
        let image = Arc::new(RgbaImage::from_fn(96, 112, |x, y| {
            image::Rgba([(x * 2) as u8, (y * 2) as u8, 80, 240])
        }));
        let mut edit = Edit::default();
        edit.settings.exposure = 0.3;
        edit.ensure_stack();
        edit.add_layer(LayerType::Retouch);
        edit.clones.push(crate::cleanup::CloneStamp {
            center: [0.48, 0.5],
            source: [0.7, 0.4],
            radius: 0.08,
            softness: 0.7,
            strength: 90.0,
        });
        edit.warps.push(WarpStroke {
            center: [0.5, 0.5],
            delta: [0.03, -0.01],
            radius: 0.12,
            softness: 0.7,
            strength: 80.0,
        });
        edit.active_layer_mut().unwrap().opacity = 70.0;
        edit.add_layer(LayerType::Adjustment);
        edit.settings.contrast = 15.0;
        let mut renderer = Renderer::default();
        let crop = Crop {
            x: 20,
            y: 22,
            width: 50,
            height: 60,
        };
        for mode in BlendMode::ALL {
            edit.active_layer_mut().unwrap().blend = mode;
            let full = engine::render(&image, &edit, None);
            assert_eq!(renderer.render(&image, &edit, None, None).unwrap(), full);
            assert_eq!(
                renderer
                    .render_region(&image, &edit, None, crop, None)
                    .unwrap(),
                image::imageops::crop_imm(&full, crop.x, crop.y, crop.width, crop.height)
                    .to_image()
            );
        }
        let photo = engine::photo_from_image("layers.png".into(), None, (*image).clone());
        let directory = tempfile::tempdir().unwrap();
        let exported = engine::export(
            &photo,
            &edit,
            None,
            directory.path(),
            engine::ExportSize::Original,
            true,
            95,
        )
        .unwrap();
        assert_eq!(
            image::open(exported).unwrap().into_rgba8(),
            engine::render(&image, &edit, None)
        );
    }
    #[test]
    fn dormant_ai_reads_every_visible_layer_and_brush_mapping_uses_selected_layer_input() {
        let mut edit = Edit::default();
        edit.settings.smoothing = 60.0;
        edit.ensure_stack();
        edit.warps.push(WarpStroke {
            center: [0.5, 0.5],
            delta: [0.05, 0.0],
            radius: 0.2,
            softness: 0.7,
            strength: 100.0,
        });
        edit.add_layer(LayerType::Retouch);
        assert!(edit.portrait_demand().skin);
        assert_eq!(
            crate::geometry::source_uv([0.5, 0.5], &edit, None, (100, 100)),
            [0.5, 0.5]
        );
        edit.stack.layers[0].visible = false;
        assert!(edit.portrait_demand().is_empty());
        edit.stack.layers[0].visible = true;
        edit.select_layer(1);
        assert!(
            (crate::geometry::source_uv([0.5, 0.5], &edit, None, (100, 100))[0] - 0.45).abs()
                < 0.00001
        );
    }

    #[test]
    fn large_layer_stack_native_crop_matches_tiled_original_resolution_output() {
        let image = Arc::new(RgbaImage::from_fn(1600, 1500, |x, y| {
            image::Rgba([
                ((x + y) % 180 + 30) as u8,
                ((x * 3 + y) % 150 + 40) as u8,
                95,
                255,
            ])
        }));
        let mut edit = Edit::default();
        edit.settings.exposure = 0.25;
        edit.ensure_stack();
        edit.add_layer(LayerType::Retouch);
        edit.clones.push(crate::cleanup::CloneStamp {
            center: [0.5, 0.5],
            source: [0.6, 0.45],
            radius: 0.02,
            softness: 0.7,
            strength: 85.0,
        });
        edit.active_layer_mut().unwrap().opacity = 65.0;
        let full = engine::render(&image, &edit, None);
        let crop = Crop {
            x: 721,
            y: 672,
            width: 210,
            height: 200,
        };
        let mut renderer = Renderer::default();
        let native = renderer
            .render_region(&image, &edit, None, crop, None)
            .unwrap();
        assert_eq!(
            native,
            image::imageops::crop_imm(&full, crop.x, crop.y, crop.width, crop.height).to_image()
        );
        assert_eq!(
            renderer
                .render_region(&image, &edit, None, crop, None)
                .unwrap(),
            native
        );
    }
}
