//! The apps on the rail. This is the one place that names them: adding an
//! app to Adelite (mail, documents) is a crate that implements
//! [`Module`], and a line here; taking one out is removing its line.

use std::path::Path;
use std::sync::Arc;

use adelite_chat_view::ChatModule;
use adelite_telegram::Telegram;
use adelite_ui::{Module, Waker};

/// Starts every app, in the order the rail shows them. Each keeps what it
/// stores in a folder of its own under `dir`; `waker` wakes the interface
/// from their threads.
pub fn start(dir: &Path, waker: &Waker) -> Vec<Box<dyn Module>> {
    vec![chat(dir, waker)]
}

/// Chat, over Telegram. Another network is another backend handed to the
/// same [`ChatModule`].
fn chat(dir: &Path, waker: &Waker) -> Box<dyn Module> {
    let wake = Arc::clone(waker);
    let (telegram, events) = Telegram::start(dir.join("telegram"), move || wake());
    Box::new(ChatModule::new(Box::new(telegram), events, Arc::clone(waker)))
}
