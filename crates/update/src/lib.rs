//! Adelite's updates, when the user asks for them in the settings.
//!
//! [`Updater::check`] asks GitHub for the newest release ([`feed`]) and
//! compares it with this build. [`Updater::install`] downloads this
//! system's file and its signature, checks the signature ([`signature`]) and
//! readies the file to go in. The app then quits, and [`Staged::apply`] puts
//! it in place and starts the new version with [`AFTER_UPDATE`]. Nothing
//! happens unless asked: the updater never checks by itself.
//!
//! Each step runs on a thread of its own, which wakes the interface when it
//! has news; [`Updater::poll`] takes the news in. No interface code here: the
//! shell draws [`State`].

pub mod feed;
mod fetch;
mod install;
pub mod signature;
mod version;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

pub use feed::{Asset, Release};
pub use install::{Staged, clean_up};
pub use version::Version;

/// The argument a freshly updated app is started with: it waits for the old
/// one to let go of the instance lock, and says that it was updated.
pub const AFTER_UPDATE: &str = "--after-update";

/// Wakes the interface from the updater's threads.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Something about updating went wrong: a sentence for the user, and for the
/// log what lay behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    message: String,
    detail: String,
}

impl Error {
    /// A failure the sentence says all about.
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), detail: String::new() }
    }

    /// A failure told as `message`, with `detail` for the log.
    pub(crate) fn logged(message: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { message: message.into(), detail: detail.into() }
    }

    /// What lay behind it, for the log; empty when the sentence says it all.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// Where updating stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Nothing asked yet.
    Idle,
    /// Asking GitHub.
    Checking,
    /// This is the newest version.
    Current,
    /// A newer version is out.
    Available(Release),
    /// Downloading `version`: `done` bytes of `size` have arrived.
    Downloading {
        /// The version coming.
        version: Version,
        /// Bytes arrived.
        done: u64,
        /// Bytes in all; 0 when GitHub did not say.
        size: u64,
    },
    /// Downloaded, checked and ready: the app quits so it can go in.
    Ready(Version),
    /// It went wrong, as the error says.
    Failed(Error),
}

/// What a thread found.
enum News {
    Checked(Result<Option<Release>, Error>),
    Progress(u64),
    Staged(Result<Staged, Error>),
}

/// Checks for updates and installs them, on threads of its own.
pub struct Updater {
    state: State,
    /// The update that is ready to go in once the app quits.
    staged: Option<Staged>,
    /// Where downloads are kept until the next start clears them.
    work: PathBuf,
    wake: Wake,
    sender: Sender<News>,
    news: Receiver<News>,
}

impl Updater {
    /// An updater that keeps its downloads in `work` and calls `wake` when it
    /// has news.
    #[must_use]
    pub fn new(work: PathBuf, wake: Wake) -> Self {
        let (sender, news) = channel();
        Self { state: State::Idle, staged: None, work, wake, sender, news }
    }

    /// An updater that only shows `state`, for pictures of the interface.
    #[must_use]
    pub fn showing(state: State) -> Self {
        Self { state, ..Self::new(std::env::temp_dir(), Arc::new(|| {})) }
    }

    /// Where updating stands.
    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Whether a step is under way, so nothing else may start.
    #[must_use]
    pub fn busy(&self) -> bool {
        matches!(self.state, State::Checking | State::Downloading { .. } | State::Ready(_))
    }

    /// Asks GitHub whether a newer version is out.
    pub fn check(&mut self) {
        if self.busy() {
            return;
        }
        self.state = State::Checking;
        let (sender, wake) = (self.sender.clone(), Arc::clone(&self.wake));
        self.spawn(move || {
            let _ = sender.send(News::Checked(feed::latest()));
            wake();
        });
    }

    /// Downloads the newer version, checks it and readies it to go in.
    pub fn install(&mut self) {
        let State::Available(release) = &self.state else { return };
        let Some((file, signature)) = release.download.clone() else { return };
        self.state = State::Downloading { version: release.version, done: 0, size: file.size };
        let (sender, wake, work) = (self.sender.clone(), Arc::clone(&self.wake), self.work.clone());
        self.spawn(move || {
            let progress = |done| {
                let _ = sender.send(News::Progress(done));
                wake();
            };
            let staged = download(&work, &file, &signature, progress);
            let _ = sender.send(News::Staged(staged));
            wake();
        });
    }

    /// Takes in what the threads found. Returns whether the state changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(news) = self.news.try_recv() {
            changed = true;
            match news {
                News::Checked(Ok(Some(release))) if release.version > Version::current() => {
                    self.state = State::Available(release);
                }
                News::Checked(Ok(_)) => self.state = State::Current,
                News::Checked(Err(e)) | News::Staged(Err(e)) => self.state = State::Failed(e),
                News::Progress(arrived) => {
                    if let State::Downloading { done, .. } = &mut self.state {
                        *done = arrived;
                    }
                }
                News::Staged(Ok(staged)) => {
                    if let State::Downloading { version, .. } = self.state {
                        self.staged = Some(staged);
                        self.state = State::Ready(version);
                    }
                }
            }
        }
        changed
    }

    /// The update that is ready, for the app to apply as it quits.
    pub fn take_staged(&mut self) -> Option<Staged> {
        self.staged.take()
    }

    /// Runs `work` on a thread of its own; says so if no thread can be had.
    fn spawn(&mut self, work: impl FnOnce() + Send + 'static) {
        if let Err(e) = std::thread::Builder::new().name("adelite-update".into()).spawn(work) {
            self.state = State::Failed(Error::logged("Adelite could not start updating.", e.to_string()));
        }
    }
}

/// Downloads `file` into `work`, checks it against `signature` and readies it.
fn download(
    work: &std::path::Path,
    file: &Asset,
    signature: &Asset,
    progress: impl FnMut(u64),
) -> Result<Staged, Error> {
    // Before the download: a copy that cannot replace itself says so at once.
    let place = install::prepare()?;
    std::fs::create_dir_all(work)
        .map_err(|e| Error::logged("Adelite could not make room for the download.", e.to_string()))?;
    let path = work.join(&file.name);
    let partial = work.join(format!("{}.part", file.name));
    fetch::download(&file.url, &partial, progress)?;
    std::fs::rename(&partial, &path).map_err(|e| Error::logged("The download could not be saved.", e.to_string()))?;
    let (status, body) = fetch::get(&signature.url, "application/octet-stream")?;
    if status != 200 {
        return Err(Error::new(format!("The download's signature could not be fetched (GitHub answered {status}).")));
    }
    signature::verify(&signature::app_key(), &file.name, &path, &String::from_utf8_lossy(&body))?;
    install::stage(place, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: Version) -> Release {
        Release { version, page: feed::releases_page(), download: None }
    }

    #[test]
    fn a_check_answers_with_a_newer_version_or_none() {
        let mut updater = Updater::showing(State::Checking);
        let newer = Version(u32::MAX, 0, 0);
        updater.sender.send(News::Checked(Ok(Some(release(newer))))).expect("sent");
        assert!(updater.poll());
        assert_eq!(updater.state(), &State::Available(release(newer)));
        // The same version, or none published: this one is the newest.
        updater.sender.send(News::Checked(Ok(Some(release(Version::current()))))).expect("sent");
        updater.poll();
        assert_eq!(updater.state(), &State::Current);
        updater.sender.send(News::Checked(Ok(None))).expect("sent");
        updater.poll();
        assert_eq!(updater.state(), &State::Current);
        updater.sender.send(News::Checked(Err(Error::new("offline")))).expect("sent");
        updater.poll();
        assert_eq!(updater.state(), &State::Failed(Error::new("offline")));
        assert!(!updater.poll());
    }

    #[test]
    fn a_download_counts_up_and_nothing_starts_meanwhile() {
        let version = Version(9, 0, 0);
        let mut updater = Updater::showing(State::Downloading { version, done: 0, size: 100 });
        assert!(updater.busy());
        updater.check();
        assert!(matches!(updater.state(), State::Downloading { .. }));
        updater.sender.send(News::Progress(40)).expect("sent");
        updater.poll();
        assert_eq!(updater.state(), &State::Downloading { version, done: 40, size: 100 });
        // A release without a download for this system cannot be installed.
        let mut updater = Updater::showing(State::Available(release(version)));
        updater.install();
        assert_eq!(updater.state(), &State::Available(release(version)));
        assert!(updater.take_staged().is_none());
    }
}
