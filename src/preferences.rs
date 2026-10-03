//! Small, version-tolerant app preferences, separate from photograph edits and recovery.
use crate::engine::ExportSize;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    pub tooltips: bool,
    pub shortcuts: std::collections::BTreeMap<String, Option<crate::shortcuts::Shortcut>>,
    pub ui_scale: f32,
    pub reduced_motion: bool,
    pub show_filmstrip: bool,
    pub wheel_zoom: bool,
    pub zoom_speed: f32,
    pub invert_scroll: bool,
    pub brush_outline: bool,
    pub mask_overlay: bool,
    pub brush_radius_percent: f32,
    pub brush_softness: f32,
    pub brush_strength: f32,
    pub autosave: bool,
    pub autosave_seconds: u64,
    pub restore_workspace: bool,
    pub export_concurrency: u8,
    pub export_png: bool,
    pub export_size: ExportSize,
    pub jpeg_quality: u8,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            tooltips: true,
            shortcuts: Default::default(),
            ui_scale: 1.1,
            reduced_motion: false,
            show_filmstrip: true,
            wheel_zoom: true,
            zoom_speed: 1.0,
            invert_scroll: false,
            brush_outline: true,
            mask_overlay: true,
            brush_radius_percent: 1.8,
            brush_softness: 75.,
            brush_strength: 100.,
            autosave: true,
            autosave_seconds: 2,
            restore_workspace: true,
            export_concurrency: 1,
            export_png: false,
            export_size: ExportSize::Original,
            jpeg_quality: 95,
        }
    }
}
impl Preferences {
    pub fn normalized(mut self) -> Self {
        let defaults = Self::default();
        fn bounded(value: f32, default: f32, min: f32, max: f32) -> f32 {
            if value.is_finite() {
                value.clamp(min, max)
            } else {
                default
            }
        }
        self.ui_scale = bounded(self.ui_scale, defaults.ui_scale, 0.85, 1.35);
        self.zoom_speed = bounded(self.zoom_speed, defaults.zoom_speed, 0.25, 2.0);
        self.brush_radius_percent = bounded(
            self.brush_radius_percent,
            defaults.brush_radius_percent,
            0.2,
            9.0,
        );
        self.brush_softness = bounded(self.brush_softness, defaults.brush_softness, 0., 100.);
        self.brush_strength = bounded(self.brush_strength, defaults.brush_strength, 1., 100.);
        self.autosave_seconds = self.autosave_seconds.clamp(1, 10);
        self.jpeg_quality = self.jpeg_quality.clamp(50, 100);
        self.export_concurrency = self.export_concurrency.clamp(1, 2);
        self
    }
}
pub(crate) fn path() -> PathBuf {
    if let Some(path) = std::env::var_os("HASTUR_PREFERENCES_PATH") {
        return path.into();
    }
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("HasturRetouch")
        .join("preferences.ron")
}
pub(crate) fn load(path: &Path) -> Result<Preferences> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(ron::from_str::<Preferences>(&text)
            .context("Invalid preferences file")?
            .normalized()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(error) => Err(error).context("Cannot read preferences file"),
    }
}
pub(crate) fn save(path: &Path, preferences: &Preferences) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let (temp, mut file) = (0..1000)
        .find_map(|n| {
            let temp = path.with_extension(format!("ron.{}.{n}.tmp", std::process::id()));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
            {
                Ok(file) => Some(Ok((temp, file))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .context("Cannot reserve preferences temporary file")??;
    let result = (|| -> Result<()> {
        let text =
            ron::ser::to_string_pretty(&preferences.clone().normalized(), Default::default())?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_roundtrip_replaces_atomically_and_preserves_defaults_and_invalid_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app/preferences.ron");
        assert_eq!(load(&path).unwrap(), Preferences::default());
        let mut prefs = Preferences {
            ui_scale: 1.25,
            autosave_seconds: 5,
            export_png: true,
            export_size: ExportSize::Web,
            ..Default::default()
        };
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path).unwrap(), prefs);
        prefs.reduced_motion = true;
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path).unwrap(), prefs);
        fs::write(&path, "(ui_scale:1.2)").unwrap();
        assert_eq!(
            load(&path).unwrap(),
            Preferences {
                ui_scale: 1.2,
                ..Default::default()
            }
        );
        fs::write(&path, "broken preferences").unwrap();
        assert!(load(&path).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "broken preferences");
    }
    #[test]
    fn invalid_numeric_preferences_cannot_break_layout_or_disable_brushes() {
        let prefs = Preferences {
            ui_scale: f32::NAN,
            zoom_speed: f32::INFINITY,
            brush_radius_percent: -20.,
            brush_strength: 0.,
            brush_softness: 200.,
            autosave_seconds: 0,
            jpeg_quality: 0,
            ..Default::default()
        }
        .normalized();
        assert_eq!(prefs.ui_scale, 1.1);
        assert_eq!(prefs.zoom_speed, 1.);
        assert_eq!(prefs.brush_radius_percent, 0.2);
        assert_eq!(prefs.brush_strength, 1.);
        assert_eq!(prefs.brush_softness, 100.);
        assert_eq!(prefs.autosave_seconds, 1);
        assert_eq!(prefs.jpeg_quality, 50);
    }
}
