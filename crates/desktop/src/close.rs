//! What closing the window does ([`CloseAction`]): the app keeps running
//! out of sight, the window is minimised, or the app quits. The settings
//! offer what the platform can do, its usual way first.
//!
//! Windows: the window goes and an icon in the tray brings it back.
//! macOS: the app is hidden, as Cmd+H does; its Dock icon brings it back.
//! Linux has no tray without a D-Bus client, so there closing minimises.

use adelite_ui::ui::icon;

/// What the window's close button (and Alt+F4) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// The window goes away and the app keeps running: in the tray on
    /// Windows, hidden behind its Dock icon on macOS.
    Background,
    /// The window is minimised to the taskbar or Dock.
    Minimize,
    /// The app quits.
    Quit,
}

/// Whether this platform can keep running without a window ([`CloseAction::Background`]).
const BACKGROUND: bool = cfg!(any(windows, target_os = "macos"));

impl CloseAction {
    /// The choices this platform offers, its default first.
    pub fn offered() -> &'static [Self] {
        if BACKGROUND { &[Self::Background, Self::Minimize, Self::Quit] } else { &[Self::Minimize, Self::Quit] }
    }

    /// What the settings file calls it.
    pub fn key(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Minimize => "minimize",
            Self::Quit => "quit",
        }
    }

    /// The one saved as `key`; the platform's default if none is, or the
    /// one saved is not offered here.
    pub fn from_key(key: &str) -> Self {
        let offered = Self::offered();
        offered.iter().copied().find(|action| action.key() == key).unwrap_or(offered[0])
    }

    /// Its name in the settings.
    pub fn label(self) -> &'static str {
        match self {
            Self::Background if cfg!(windows) => "Keep running in the tray",
            Self::Background => "Keep running in the Dock",
            Self::Minimize if cfg!(windows) => "Minimise to the taskbar",
            Self::Minimize if cfg!(target_os = "macos") => "Minimise to the Dock",
            Self::Minimize => "Minimise",
            Self::Quit => "Quit",
        }
    }

    /// A line on what it means.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Background if cfg!(windows) => "Messages keep arriving; the tray icon brings the window back.",
            Self::Background => "Messages keep arriving; the Dock icon brings the app back.",
            Self::Minimize => "Messages keep arriving in the minimised window.",
            Self::Quit => "Nothing arrives until Adelite is started again.",
        }
    }

    /// Its icon in the settings' list.
    pub fn glyph(self) -> char {
        match self {
            Self::Background => icon::DOWNLOAD,
            Self::Minimize => icon::MINUS,
            Self::Quit => icon::X,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_close_action_is_read_back_or_the_default_taken() {
        let default = CloseAction::offered()[0];
        assert_eq!(CloseAction::from_key(""), default);
        assert_eq!(CloseAction::from_key("nonsense"), default);
        for &action in CloseAction::offered() {
            assert_eq!(CloseAction::from_key(action.key()), action);
            assert!(!action.label().is_empty() && !action.detail().is_empty());
        }
        // Linux cannot keep running without a window: it minimises instead.
        let background = CloseAction::from_key("background");
        assert_eq!(background == CloseAction::Background, cfg!(any(windows, target_os = "macos")));
    }
}
