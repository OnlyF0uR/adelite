//! Telegram for Adelite's chat, through TDLib.
//!
//! [`Telegram`] is a [`Backend`]: the chat view sends it [`Command`]s and
//! reads [`Event`]s from the channel it was started with. Underneath,
//! TDLib (Telegram's own client library) does the networking, encryption
//! and storage on its threads; this crate loads it ([`tdjson`]), speaks its
//! JSON (`request`), and turns what it says into the chat model (`incoming`,
//! `map`).
//!
//! Threading: commands become JSON on the caller's thread and are handed to
//! TDLib, which never blocks. One thread of ours (`adelite-telegram`) sits in
//! `td_receive`, maps what arrives and wakes the interface. What it remembers
//! of users and chats sits behind a lock, which a command takes for the moment
//! its request depends on it. No async runtime.
//!
//! Everything Telegram keeps is in the folder given to [`Telegram::start`]:
//! TDLib's database and downloaded files.
//!
//! Telegram knows the app by an API id and hash (from
//! <https://my.telegram.org>), which belong to the app, not to its user:
//! the build takes them from `ADELITE_TELEGRAM_API_ID` and
//! `ADELITE_TELEGRAM_API_HASH` (the `.env` file at the workspace's root, or
//! the environment) and seals them into the binary (`build.rs`, `seal`). A
//! build without them asks for a pair and keeps it in `app.json`, so a
//! developer can still try it.

mod client;
mod incoming;
mod map;
mod request;
#[cfg_attr(not(test), expect(dead_code, reason = "sealing and reading the env file are the build script's part"))]
mod seal;
pub mod tdjson;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;

use adelite_chat::{Backend, Command, Event, Session};

use client::Shared;

/// The Telegram backend. Dropping it closes TDLib, which writes what it
/// still holds to disk first.
pub struct Telegram {
    shared: Option<Arc<Shared>>,
    thread: Option<JoinHandle<()>>,
}

impl Telegram {
    /// Starts Telegram with its data in `dir`. Events arrive on the
    /// returned channel; `wake` is called (from another thread) whenever
    /// some were sent. Without TDLib on this machine the backend still
    /// starts, and its first event says what is missing.
    pub fn start(dir: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> (Self, Receiver<Event>) {
        let (events, inbox) = mpsc::channel();
        let (library, receiver) = match tdjson::Library::open(&tdjson::places(&dir)) {
            Ok(opened) => opened,
            Err(e) => {
                let _ = events.send(Event::Session(Session::Unavailable {
                    title: "Telegram's library is missing".to_owned(),
                    detail: format!(
                        "Adelite talks to Telegram through TDLib. Put {} beside the app, or build it with \
                         packaging/tdlib. ({e})",
                        tdjson::FILE_NAME
                    ),
                }));
                wake();
                return (Self { shared: None, thread: None }, inbox);
            }
        };
        let shared = Arc::new(Shared::new(library, dir, events, Box::new(wake)));
        let theirs = Arc::clone(&shared);
        let thread = std::thread::Builder::new().name("adelite-telegram".into()).spawn(move || theirs.run(receiver));
        match thread {
            Ok(thread) => {
                shared.begin();
                (Self { shared: Some(shared), thread: Some(thread) }, inbox)
            }
            Err(e) => {
                shared.report(Event::Session(Session::Unavailable {
                    title: "Telegram could not be started".to_owned(),
                    detail: format!("No thread could be started for it: {e}"),
                }));
                (Self { shared: None, thread: None }, inbox)
            }
        }
    }
}

impl Backend for Telegram {
    fn name(&self) -> &'static str {
        "Telegram"
    }

    fn send(&self, command: Command) {
        if let Some(shared) = &self.shared {
            shared.command(command);
        }
    }
}

impl Drop for Telegram {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            shared.close();
        }
        // The thread ends when TDLib says it has closed, or stays silent too long.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Held by each test that uses the real library: only one may receive from it at a time.
#[cfg(test)]
static TDLIB: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    /// Starts the real backend in an empty folder: it must ask for the phone
    /// number when the build has an API id and for an API id when it has
    /// none, wake the interface, and close cleanly when dropped.
    /// `cargo test -p adelite-telegram backend -- --ignored`
    #[test]
    #[ignore = "needs TDLib; run by hand after building it"]
    fn backend_asks_for_the_api_id_first() {
        let _alone = TDLIB.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("adelite-backend-test-{}", std::process::id()));
        let woken = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&woken);
        let (telegram, events) = Telegram::start(dir.clone(), move || {
            count.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(telegram.name(), "Telegram");
        // Other events (the connection's state) may come before the prompt.
        let prompt = std::iter::from_fn(|| events.recv_timeout(Duration::from_secs(20)).ok())
            .find_map(|event| match event {
                Event::Session(Session::Asking(prompt)) => Some(prompt),
                _ => None,
            })
            .expect("a prompt");
        assert!(woken.load(Ordering::Relaxed) > 0, "the interface was woken");
        if client::has_built_in() {
            // Built with its own API id: the first thing asked is the user's phone number.
            assert_eq!((prompt.title.as_str(), prompt.fields.len()), ("Sign in to Telegram", 1));
        } else {
            assert_eq!((prompt.title.as_str(), prompt.fields.len()), ("Connect Telegram", 2));
            // An answer that cannot be an API id is refused here, without asking Telegram.
            telegram.send(Command::Answer(vec!["abc".into(), "short".into()]));
            let refused = events.recv_timeout(Duration::from_secs(5)).expect("an event");
            assert!(matches!(refused, Event::Session(Session::Asking(prompt)) if !prompt.problem.is_empty()));
        }
        drop(telegram);
        assert!(!dir.join("app.json").exists(), "nothing was saved");
        let _ = std::fs::remove_dir_all(dir);
    }
}
