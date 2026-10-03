//! Reversible controls for the editor's fixed tool groups. The original stays immutable.
use crate::engine::{Background, Edit, Settings, Target};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(usize)]
pub enum LayerKind {
    Portrait,
    Color,
    Background,
    Liquify,
    SpotHeal,
    CloneStamp,
    Patch,
}

impl LayerKind {
    pub const ALL: [Self; 7] = [
        Self::Portrait,
        Self::Color,
        Self::Background,
        Self::Liquify,
        Self::SpotHeal,
        Self::CloneStamp,
        Self::Patch,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Portrait => "Portrait",
            Self::Color => "Color",
            Self::Background => "Background",
            Self::Liquify => "Liquify",
            Self::SpotHeal => "Spot heal",
            Self::CloneStamp => "Clone stamp",
            Self::Patch => "Patch",
        }
    }

    /// Inspect saved edits, independently of the layer's current visibility.
    pub fn has_content(self, edit: &Edit) -> bool {
        let settings = edit.settings.effective();
        match self {
            Self::Portrait => [
                settings.smoothing,
                settings.blemishes,
                settings.tone_evenness,
                settings.redness,
                settings.teeth,
                settings.eyes,
                settings.under_eyes,
                settings.forehead,
                settings.laugh_lines,
                settings.contour,
                settings.face_highlight,
                settings.eye_size,
                settings.nose_width,
                settings.lip_plumpness,
                settings.jawline,
                settings.flyaway_hairs,
            ]
            .iter()
            .any(|v| *v != 0.0),
            Self::Color => {
                [
                    settings.exposure,
                    settings.contrast,
                    settings.shadows,
                    settings.highlights,
                    settings.warmth,
                    settings.tint,
                    settings.saturation,
                    settings.sharpening,
                    settings.vignette,
                ]
                .iter()
                .any(|v| *v != 0.0)
                    || settings.color.active()
            }
            Self::Background => {
                settings.background != Background::Original && settings.background_opacity != 0.0
            }
            Self::Liquify => edit.warps.iter().any(|stroke| stroke.strength != 0.0),
            Self::SpotHeal => {
                settings.healing != 0.0
                    && edit
                        .strokes
                        .iter()
                        .any(|stroke| stroke.target == Target::Heal && !stroke.erase)
            }
            Self::CloneStamp => edit.clones.iter().any(|stroke| stroke.strength != 0.0),
            Self::Patch => edit.patches.iter().any(|stroke| stroke.strength != 0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerControl {
    pub visible: bool,
    pub opacity: f32,
}

impl Default for LayerControl {
    fn default() -> Self {
        Self {
            visible: true,
            opacity: 100.0,
        }
    }
}

impl LayerControl {
    pub fn factor(self) -> f32 {
        if self.visible && self.opacity.is_finite() {
            self.opacity.clamp(0.0, 100.0) / 100.0
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerControls {
    controls: [LayerControl; 7],
}

impl LayerControls {
    pub fn get(&self, kind: LayerKind) -> &LayerControl {
        &self.controls[kind as usize]
    }

    pub fn get_mut(&mut self, kind: LayerKind) -> &mut LayerControl {
        &mut self.controls[kind as usize]
    }

    pub fn is_default(&self) -> bool {
        self.controls
            .iter()
            .all(|control| *control == LayerControl::default())
    }
}

impl Edit {
    /// Opacity attenuates each tool's effect without modifying the saved sliders or strokes.
    pub fn effective_settings(&self) -> Settings {
        let mut settings = self.settings.effective();
        let portrait = self.layers.get(LayerKind::Portrait).factor();
        for value in [
            &mut settings.smoothing,
            &mut settings.blemishes,
            &mut settings.tone_evenness,
            &mut settings.redness,
            &mut settings.teeth,
            &mut settings.eyes,
            &mut settings.under_eyes,
            &mut settings.forehead,
            &mut settings.laugh_lines,
            &mut settings.contour,
            &mut settings.face_highlight,
            &mut settings.eye_size,
            &mut settings.nose_width,
            &mut settings.lip_plumpness,
            &mut settings.jawline,
            &mut settings.flyaway_hairs,
        ] {
            *value *= portrait;
        }
        let color = self.layers.get(LayerKind::Color).factor();
        for value in [
            &mut settings.exposure,
            &mut settings.contrast,
            &mut settings.shadows,
            &mut settings.highlights,
            &mut settings.warmth,
            &mut settings.tint,
            &mut settings.saturation,
            &mut settings.sharpening,
            &mut settings.vignette,
            &mut settings.color.hsl_strength,
            &mut settings.color.curves_strength,
            &mut settings.color.reference_strength,
        ] {
            *value *= color;
        }
        let background = self.layers.get(LayerKind::Background).factor();
        settings.background_opacity *= background;
        if settings.background_opacity <= 0.0 {
            settings.background = Background::Original;
        }
        settings.healing *= self.layers.get(LayerKind::SpotHeal).factor();
        settings
    }

    /// Materialize controls once at a render boundary. Default edits retain shared storage.
    pub fn effective_layers(&self) -> Cow<'_, Self> {
        if self.layers.is_default() {
            return Cow::Borrowed(self);
        }
        let mut result = self.clone();
        result.settings = self.effective_settings();
        let liquify = self.layers.get(LayerKind::Liquify).factor();
        if liquify == 0.0 {
            result.warps = Default::default();
        } else if liquify != 1.0 {
            for warp in &mut *result.warps {
                warp.strength *= liquify;
            }
        }
        if self.layers.get(LayerKind::SpotHeal).factor() == 0.0 {
            result.strokes = self
                .strokes
                .iter()
                .filter(|stroke| stroke.target != Target::Heal)
                .cloned()
                .collect();
        }
        let clone = self.layers.get(LayerKind::CloneStamp).factor();
        if clone == 0.0 {
            result.clones = Default::default();
        } else if clone != 1.0 {
            for stamp in &mut *result.clones {
                stamp.strength *= clone;
            }
        }
        let patch = self.layers.get(LayerKind::Patch).factor();
        if patch == 0.0 {
            result.patches = Default::default();
        } else if patch != 1.0 {
            for stroke in &mut *result.patches {
                stroke.strength *= patch;
            }
        }
        result.layers = LayerControls::default();
        Cow::Owned(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cleanup::{CloneStamp, PatchStroke},
        engine::{Adjustment, Stroke},
        geometry::WarpStroke,
    };

    #[test]
    fn disabled_layers_neutralize_only_their_group_and_preserve_saved_values() {
        let mut edit = Edit::default();
        edit.settings.smoothing = 70.0;
        edit.settings.eye_size = 50.0;
        edit.settings.contrast = 40.0;
        edit.settings.color.hsl_strength = 60.0;
        edit.settings.background = Background::Solid;
        edit.layers.get_mut(LayerKind::Portrait).visible = false;
        edit.layers.get_mut(LayerKind::Color).opacity = 25.0;
        edit.layers.get_mut(LayerKind::Background).opacity = 50.0;
        let settings = edit.effective_settings();
        assert_eq!(settings.smoothing, 0.0);
        assert_eq!(settings.eye_size, 0.0);
        assert_eq!(settings.contrast, 10.0);
        assert_eq!(settings.color.hsl_strength, 15.0);
        assert_eq!(settings.background, Background::Solid);
        assert_eq!(settings.background_opacity, 50.0);
        assert_eq!(edit.settings.smoothing, 70.0);
        edit.settings.disabled.push(Adjustment::Contrast);
        assert_eq!(edit.effective_settings().contrast, 0.0);
    }

    #[test]
    fn manual_controls_keep_instructions_reversible_and_materialization_idempotent() {
        let mut edit = Edit::default();
        assert!(matches!(edit.effective_layers(), Cow::Borrowed(_)));
        edit.strokes = vec![
            Stroke {
                target: Target::Heal,
                center: [0.5; 2],
                radius: 0.1,
                erase: false,
                strength: 100.0,
                softness: 0.6,
            },
            Stroke {
                target: Target::Skin,
                center: [0.4; 2],
                radius: 0.2,
                erase: false,
                strength: 100.0,
                softness: 0.5,
            },
        ]
        .into();
        edit.warps = vec![WarpStroke {
            center: [0.5; 2],
            delta: [0.1, 0.0],
            radius: 0.2,
            softness: 0.5,
            strength: 80.0,
        }]
        .into();
        edit.clones = vec![CloneStamp {
            center: [0.5; 2],
            source: [0.2; 2],
            radius: 0.1,
            softness: 0.5,
            strength: 60.0,
        }]
        .into();
        edit.patches = vec![PatchStroke {
            boundary: vec![[0.2; 2], [0.3, 0.2], [0.3; 2]],
            offset: [0.1; 2],
            softness: 0.5,
            strength: 70.0,
        }]
        .into();
        edit.layers.get_mut(LayerKind::Liquify).opacity = 50.0;
        edit.layers.get_mut(LayerKind::SpotHeal).visible = false;
        edit.layers.get_mut(LayerKind::CloneStamp).opacity = 25.0;
        edit.layers.get_mut(LayerKind::Patch).visible = false;
        let effective = edit.effective_layers();
        assert_eq!(effective.warps[0].strength, 40.0);
        assert_eq!(effective.clones[0].strength, 15.0);
        assert!(effective.patches.is_empty());
        assert_eq!(effective.strokes.len(), 1);
        assert_eq!(effective.strokes[0].target, Target::Skin);
        assert!(matches!(effective.effective_layers(), Cow::Borrowed(_)));
        assert_eq!(edit.strokes.len(), 2);
        assert_eq!(edit.patches.len(), 1);
        assert_eq!(edit.warps[0].strength, 80.0);
    }
}
