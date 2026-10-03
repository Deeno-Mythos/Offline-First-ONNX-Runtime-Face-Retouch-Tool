//! NullState: each portrait component stays absent until an enabled edit needs it.
//! Components share geometry, retain completed maps, and can release model weights
//! independently of the photo's edits and history.
use crate::engine::Settings;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PortraitDemand {
    pub geometry: bool,
    pub skin: bool,
    pub blemishes: bool,
}

impl PortraitDemand {
    pub const NONE: Self = Self {
        geometry: false,
        skin: false,
        blemishes: false,
    };
    pub const GEOMETRY: Self = Self {
        geometry: true,
        ..Self::NONE
    };
    pub const ALL: Self = Self {
        geometry: true,
        skin: true,
        blemishes: true,
    };

    pub fn normalized(self) -> Self {
        Self {
            geometry: self.geometry || self.skin || self.blemishes,
            ..self
        }
    }
    pub fn is_empty(self) -> bool {
        !self.geometry && !self.skin && !self.blemishes
    }
    pub fn contains(self, other: Self) -> bool {
        (!other.geometry || self.geometry)
            && (!other.skin || self.skin)
            && (!other.blemishes || self.blemishes)
    }
    pub fn union(self, other: Self) -> Self {
        Self {
            geometry: self.geometry || other.geometry,
            skin: self.skin || other.skin,
            blemishes: self.blemishes || other.blemishes,
        }
    }
    /// Raw missing components; normalization is applied to requests, not readiness.
    pub fn without(self, other: Self) -> Self {
        Self {
            geometry: self.geometry && !other.geometry,
            skin: self.skin && !other.skin,
            blemishes: self.blemishes && !other.blemishes,
        }
    }
    pub fn from_settings(settings: &Settings) -> Self {
        let s = settings.effective();
        let active = |v: f32| v.is_finite() && v != 0.0;
        let skin = [s.smoothing, s.under_eyes, s.forehead, s.laugh_lines]
            .into_iter()
            .any(active);
        let blemishes = active(s.blemishes);
        let geometry = skin
            || blemishes
            || [
                s.tone_evenness,
                s.redness,
                s.teeth,
                s.eyes,
                s.contour,
                s.face_highlight,
                s.eye_size,
                s.nose_width,
                s.lip_plumpness,
                s.jawline,
                s.flyaway_hairs,
            ]
            .into_iter()
            .any(active);
        Self {
            geometry,
            skin,
            blemishes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Adjustment, Background};

    #[test]
    fn neutral_manual_and_color_edits_leave_all_components_dormant() {
        let mut settings = Settings::default();
        assert_eq!(
            PortraitDemand::from_settings(&settings),
            PortraitDemand::NONE
        );
        settings.exposure = 1.0;
        settings.contrast = 50.0;
        settings.healing = 100.0;
        settings.background = Background::Solid;
        assert_eq!(
            PortraitDemand::from_settings(&settings),
            PortraitDemand::NONE
        );
    }
    #[test]
    fn face_adjustments_request_only_enabled_dependencies() {
        let mut settings = Settings {
            under_eyes: 50.0,
            ..Default::default()
        };
        assert_eq!(
            PortraitDemand::from_settings(&settings),
            PortraitDemand {
                geometry: true,
                skin: true,
                blemishes: false,
            }
        );
        settings.disabled.push(Adjustment::UnderEyes);
        assert!(PortraitDemand::from_settings(&settings).is_empty());
        settings.blemishes = 55.0;
        assert_eq!(
            PortraitDemand::from_settings(&settings),
            PortraitDemand {
                geometry: true,
                skin: false,
                blemishes: true,
            }
        );
        settings.blemishes = 0.0;
        settings.nose_width = -25.0;
        assert_eq!(
            PortraitDemand::from_settings(&settings),
            PortraitDemand::GEOMETRY
        );
        let missing = PortraitDemand::ALL.without(PortraitDemand::GEOMETRY);
        assert!(!missing.geometry && missing.skin && missing.blemishes);
    }
}
