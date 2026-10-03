//! Portable persisted key bindings. Canvas navigation keys remain reserved.
use eframe::egui::{self, Key, Modifiers};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Shortcut {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}
impl Shortcut {
    pub fn parse(text: &str) -> Option<Self> {
        if text.is_empty() {
            return None;
        }
        let pieces: Vec<_> = text.split('+').collect();
        let name = *pieces.last()?;
        let key = match name {
            "Left" => Key::ArrowLeft,
            "Right" => Key::ArrowRight,
            "," => Key::Comma,
            "0" => Key::Num0,
            _ => Key::from_name(name)?,
        };
        Some(Self {
            key: key.name().into(),
            ctrl: pieces.contains(&"Ctrl"),
            shift: pieces.contains(&"Shift"),
            alt: pieces.contains(&"Alt"),
        })
    }
    pub fn capture(key: Key, modifiers: Modifiers) -> Self {
        Self {
            key: key.name().into(),
            ctrl: modifiers.ctrl || modifiers.command,
            shift: modifiers.shift,
            alt: modifiers.alt,
        }
    }
    pub fn valid(&self) -> bool {
        Key::from_name(&self.key)
            .is_some_and(|key| !matches!(key, Key::Space | Key::Backslash | Key::Tab | Key::Escape))
    }
    pub fn label(&self) -> String {
        format!(
            "{}{}{}{}",
            if self.ctrl { "Ctrl+" } else { "" },
            if self.alt { "Alt+" } else { "" },
            if self.shift { "Shift+" } else { "" },
            self.key
        )
    }
    pub fn consume(&self, ctx: &egui::Context) -> bool {
        let Some(key) = Key::from_name(&self.key) else {
            return false;
        };
        let modifiers = Modifiers {
            ctrl: false,
            command: self.ctrl,
            shift: self.shift,
            alt: self.alt,
            ..Modifiers::NONE
        };
        ctx.input_mut(|i| i.modifiers.matches_exact(modifiers) && i.consume_key(modifiers, key))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bindings_preserve_exact_modifiers_and_reserve_pan_and_cancel_keys() {
        assert_eq!(
            Shortcut::parse("Ctrl+Shift+Z").unwrap().label(),
            "Ctrl+Shift+Z"
        );
        for key in [Key::Space, Key::Backslash, Key::Escape, Key::Tab] {
            assert!(!Shortcut::capture(key, Modifiers::CTRL).valid());
        }
        let ctx = egui::Context::default();
        let binding = Shortcut::parse("Alt+Q").unwrap();
        let _ = ctx.run(
            egui::RawInput {
                modifiers: Modifiers::ALT,
                events: vec![egui::Event::Key {
                    key: Key::Q,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::ALT,
                }],
                ..Default::default()
            },
            |ctx| {
                assert!(!Shortcut::parse("Q").unwrap().consume(ctx));
                assert!(binding.consume(ctx));
                assert!(!binding.consume(ctx));
            },
        );
    }
}
