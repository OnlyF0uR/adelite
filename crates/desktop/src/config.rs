//! The shell's settings: `~/.adelite/config.json`. A few small values, read
//! once and written when one changes.

use std::path::{Path, PathBuf};

use adelite_ui::platform;
use serde_json::{Value, json};

/// The settings file's name in the data folder.
const FILE: &str = "config.json";
/// Interface zoom limits, in percent.
pub const ZOOM: (u16, u16) = (50, 200);

/// What the shell remembers between runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The colour scheme's key (`black`, `white`, `amber`).
    pub theme: String,
    /// Interface zoom in percent, on top of the display's scale.
    pub zoom: u16,
    /// The window's size and state: `1200x800`, `1200x800 maximized`.
    pub window: String,
    /// Whether what arrives in the background is said outside the window.
    pub notify: bool,
    /// Whether a message that arrives or leaves is heard.
    pub sounds: bool,
    /// What closing the window does ([`crate::close::CloseAction::key`]); empty for the platform's usual way.
    pub close: String,
    /// Where it is kept; `None` without a home directory (nothing is saved then).
    path: Option<PathBuf>,
}

impl Config {
    /// Reads the settings in `dir`; what is missing or unreadable is the default.
    pub fn load(dir: Option<&Path>) -> Self {
        let path = dir.map(|dir| dir.join(FILE));
        let saved = path.as_ref().and_then(|path| std::fs::read(path).ok());
        let saved: Value = saved.and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or(Value::Null);
        let text = |key: &str| saved[key].as_str().unwrap_or_default().to_owned();
        let zoom = saved["zoom"].as_u64().and_then(|zoom| u16::try_from(zoom).ok()).unwrap_or(100);
        let switch = |key: &str| saved[key].as_bool().unwrap_or(true);
        Self {
            theme: text("theme"),
            zoom: zoom.clamp(ZOOM.0, ZOOM.1),
            window: text("window"),
            notify: switch("notify"),
            sounds: switch("sounds"),
            close: text("close"),
            path,
        }
    }

    /// Writes the settings. A failure is logged: a setting not kept is no reason to stop.
    pub fn save(&self) {
        let Some(path) = &self.path else { return };
        let text = json!({
            "theme": self.theme, "zoom": self.zoom, "window": self.window, "notify": self.notify,
            "sounds": self.sounds, "close": self.close,
        })
        .to_string();
        // ponytail: written on the interface's thread; it is a hundred bytes. A writer thread if it ever grows.
        if let Err(e) = platform::write_private(path, text.as_bytes()) {
            adelite_ui::log::error(format!("could not save the settings: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_survive_nonsense() {
        let dir = std::env::temp_dir().join(format!("adelite-config-test-{}", std::process::id()));
        let fresh = Config::load(Some(&dir));
        assert_eq!((fresh.theme.as_str(), fresh.zoom, fresh.window.as_str(), fresh.notify), ("", 100, "", true));
        assert_eq!((fresh.sounds, fresh.close.as_str()), (true, ""));
        let saved = Config {
            theme: "amber".into(),
            zoom: 120,
            window: "1200x800 maximized".into(),
            notify: false,
            sounds: false,
            close: "quit".into(),
            ..fresh
        };
        saved.save();
        assert_eq!(Config::load(Some(&dir)), saved);
        std::fs::write(dir.join(FILE), br#"{"zoom": 9000, "theme": 7, "window": null}"#).expect("written");
        let odd = Config::load(Some(&dir));
        assert_eq!(
            (odd.theme.as_str(), odd.zoom, odd.window.as_str()),
            ("", ZOOM.1, ""),
            "a zoom out of range is the nearest allowed"
        );
        std::fs::write(dir.join(FILE), b"not json").expect("written");
        assert_eq!(Config::load(Some(&dir)).zoom, 100);
        assert_eq!(Config::load(None).zoom, 100);
        Config::load(None).save();
        let _ = std::fs::remove_dir_all(dir);
    }
}
