//! What an app on the rail is to the shell: an icon, a place to draw, and
//! the keyboard while it is shown. Adding an app to Adelite is implementing
//! [`Module`] in a crate of its own and naming it in the shell's list.

use std::path::PathBuf;
use std::sync::Arc;

use arboard::Clipboard;
use winit::event::KeyEvent;
use winit::keyboard::ModifiersState;

use crate::paint::{Painter, Rect};
pub use crate::platform::sound::Sound;
use crate::ui::Ui;

/// Wakes the interface from any thread: the shell then calls every
/// module's [`Module::poll`] and draws a frame. A module hands it to the
/// threads it starts, which keep their results in a channel of the module's.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Something a module asks of the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Something arrived: flash the window if it is in the background.
    Attention,
    /// Tell the user this went wrong, in a notice over the page.
    Notice(String),
    /// Tell the user this, in a notice over the page: nothing is wrong.
    Info(String),
    /// Something arrived while the user was elsewhere: tell them outside
    /// the window, where the system shows what apps have to say.
    Notify {
        /// Who it is from.
        title: String,
        /// What it says.
        body: String,
    },
    /// Something happened that has a sound.
    Sound {
        /// Which sound.
        sound: Sound,
        /// It happened where the user is looking: it is heard only when
        /// the window is not in front.
        in_view: bool,
    },
}

/// What the shell lends a module while it handles input or draws.
pub struct Context<'a> {
    /// The system clipboard, opened on first use.
    pub clipboard: &'a mut Option<Clipboard>,
    /// What the module asks of the shell, carried out after the call.
    pub requests: &'a mut Vec<Request>,
}

/// An app on the rail.
pub trait Module {
    /// Its name, shown beside its icon while the mouse is over it.
    fn name(&self) -> &'static str;

    /// Its icon on the rail ([`crate::ui::icon`]).
    fn icon(&self) -> char;

    /// The count on its icon: what is waiting for the user there.
    fn badge(&self) -> u32 {
        0
    }

    /// Takes in what its threads left for it. Called on the interface's
    /// thread after a [`Waker`] fired, shown or not.
    fn poll(&mut self, _cx: &mut Context<'_>) {}

    /// Draws it in `area`: the window under the title bar, beside the rail.
    fn draw(&mut self, p: &mut Painter<'_>, ui: &mut Ui, area: Rect, cx: &mut Context<'_>);

    /// A key was pressed while it is shown.
    fn key(&mut self, _event: &KeyEvent, _mods: ModifiersState, _cx: &mut Context<'_>) {}

    /// An input method committed `text` while it is shown.
    fn ime_commit(&mut self, _text: &str) {}

    /// Where its caret is, for the input method's window.
    fn ime_area(&self) -> Option<Rect> {
        None
    }

    /// Files were dropped on the window while it is shown.
    fn files_dropped(&mut self, _files: Vec<PathBuf>) {}

    /// Whether it moves by itself and wants a frame after this one.
    fn is_busy(&self) -> bool {
        false
    }
}
