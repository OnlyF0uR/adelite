//! Putting a checked download in place, and clearing up after it.
//!
//! Windows: the download is the installer (`packaging/windows/adelite.iss`).
//! It runs silently once the app has quit, waits until the old
//! `adelite.exe` is free, replaces the files and starts the app again with
//! [`AFTER_UPDATE`].
//!
//! macOS and Linux: the download is a `.tar.gz`. While the app still runs it
//! is unpacked beside the installed app ("staged"). Once the app has quit,
//! each unpacked entry takes the place of the old one by renaming, which a
//! running program does not prevent, and the new app is started. What it
//! replaced is removed at the next start ([`clean_up`]).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(not(windows))]
use crate::AFTER_UPDATE;
use crate::Error;

/// The folder an update is unpacked into, beside the installed app.
#[cfg(not(windows))]
const STAGING: &str = ".adelite-update";
/// The end of the name an entry that was replaced keeps until the next start.
#[cfg(not(windows))]
const OLD: &str = ".adelite-old";

/// Where an update goes: found (and on macOS and Linux tried for writing)
/// before anything is downloaded, so a copy that cannot update itself says
/// so at once.
#[derive(Debug)]
pub struct Place {
    /// The folder holding the installed app.
    #[cfg(not(windows))]
    root: PathBuf,
    /// The installed app: the bundle on macOS, the program on Linux.
    #[cfg(not(windows))]
    app: PathBuf,
}

/// A checked download, ready to go in once the app has quit.
#[derive(Debug)]
pub struct Staged {
    /// The installer to run.
    #[cfg(windows)]
    installer: PathBuf,
    /// The folder it was unpacked into.
    #[cfg(not(windows))]
    staging: PathBuf,
    /// What moves where: an unpacked entry and the installed one it replaces.
    #[cfg(not(windows))]
    moves: Vec<(PathBuf, PathBuf)>,
    /// What is started afterwards.
    #[cfg(not(windows))]
    app: PathBuf,
}

/// Finds where the update goes.
///
/// # Errors
/// This copy cannot replace itself: not an app bundle, run from a read-only
/// place, or in a folder it may not write to.
#[cfg(windows)]
#[expect(clippy::unnecessary_wraps, reason = "finding the place can fail on the other systems")]
pub fn prepare() -> Result<Place, Error> {
    Ok(Place {})
}

/// Finds where the update goes.
///
/// # Errors
/// This copy cannot replace itself: not an app bundle, run from a read-only
/// place, or in a folder it may not write to.
#[cfg(not(windows))]
pub fn prepare() -> Result<Place, Error> {
    let place = place()?;
    let staging = place.root.join(STAGING);
    remove(&staging);
    fs::create_dir(&staging).map_err(|e| {
        Error::logged(
            format!("Adelite may not write in {}, so it cannot update itself there.", place.root.display()),
            e.to_string(),
        )
    })?;
    Ok(place)
}

/// Readies `download`, already checked, to go in.
///
/// # Errors
/// The download could not be unpacked, or is not an Adelite release.
#[cfg(windows)]
#[expect(clippy::unnecessary_wraps, reason = "unpacking can fail on the other systems")]
pub fn stage(_: Place, download: PathBuf) -> Result<Staged, Error> {
    Ok(Staged { installer: download })
}

/// Readies `download`, already checked, to go in: unpacks it beside the app.
///
/// # Errors
/// The download could not be unpacked, or is not an Adelite release.
#[cfg(not(windows))]
pub fn stage(place: Place, download: PathBuf) -> Result<Staged, Error> {
    let staging = place.root.join(STAGING);
    let mut tar = Command::new(if cfg!(target_os = "macos") { "/usr/bin/tar" } else { "tar" });
    tar.arg("-xzf").arg(&download).arg("-C").arg(&staging);
    // The Linux archive holds a folder `adelite/`; its contents are what is installed.
    if cfg!(target_os = "linux") {
        tar.arg("--strip-components=1");
    }
    let output = tar
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error::logged("The download could not be unpacked.", e.to_string()))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(Error::logged("The download could not be unpacked.", said));
    }
    let moves = moves(&place, &staging)?;
    let _ = fs::remove_file(download);
    Ok(Staged { staging, moves, app: place.app })
}

impl Staged {
    /// Puts the update in place and starts the new version. Called as the
    /// app quits, once it has let go of everything.
    ///
    /// # Errors
    /// The installer could not be started (Windows), or the files could not
    /// be moved, in which case what was moved is put back.
    #[cfg(windows)]
    pub fn apply(self) -> Result<(), Error> {
        Command::new(&self.installer)
            .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"])
            .stdin(Stdio::null())
            .spawn()
            .map(drop)
            .map_err(|e| Error::logged("The installer could not be started.", e.to_string()))
    }

    /// Puts the update in place and starts the new version. Called as the
    /// app quits, once it has let go of everything.
    ///
    /// # Errors
    /// The installer could not be started (Windows), or the files could not
    /// be moved, in which case what was moved is put back.
    #[cfg(not(windows))]
    pub fn apply(self) -> Result<(), Error> {
        // What has moved: the installed entry, where the old one went, and whether there was one.
        let mut done: Vec<(&Path, PathBuf, bool)> = Vec::new();
        for (from, to) in &self.moves {
            let old = old_name(to);
            remove(&old);
            let had = to.symlink_metadata().is_ok();
            let moved = (if had { fs::rename(to, &old) } else { Ok(()) }).and_then(|()| {
                fs::rename(from, to).inspect_err(|_| {
                    if had {
                        let _ = fs::rename(&old, to);
                    }
                })
            });
            if let Err(e) = moved {
                for (to, old, had) in done.into_iter().rev() {
                    remove(to);
                    if had {
                        let _ = fs::rename(&old, to);
                    }
                }
                return Err(Error::logged("The new version could not be put in place.", e.to_string()));
            }
            done.push((to.as_path(), old, had));
        }
        remove(&self.staging);
        let mut start = if cfg!(target_os = "macos") {
            let mut open = Command::new("/usr/bin/open");
            open.arg("-n").arg(&self.app).arg("--args");
            open
        } else {
            Command::new(&self.app)
        };
        start.arg(AFTER_UPDATE).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        start.spawn().map(drop).map_err(|e| Error::logged("The new version could not be started.", e.to_string()))
    }
}

/// Removes what updates left behind: the downloads in `work`, and on macOS
/// and Linux an unpacked update that never went in and what the last one
/// replaced. Quick, and quiet about what it cannot remove (an installer
/// still finishing, say): the next start tries again.
pub fn clean_up(work: &Path) {
    remove(work);
    #[cfg(not(windows))]
    if let Ok(place) = place() {
        remove(&place.root.join(STAGING));
        for entry in fs::read_dir(&place.root).into_iter().flatten().flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') && name.ends_with(OLD) {
                remove(&entry.path());
            }
        }
    }
}

/// The app's bundle and the folder holding it.
#[cfg(target_os = "macos")]
fn place() -> Result<Place, Error> {
    let exe = std::env::current_exe().map_err(|e| Error::logged("Adelite could not find itself.", e.to_string()))?;
    let in_bundle = exe.parent().and_then(Path::file_name).is_some_and(|name| name == "MacOS");
    let bundle = exe.ancestors().nth(3).filter(|b| in_bundle && b.extension().is_some_and(|e| e == "app"));
    let Some(bundle) = bundle else {
        return Err(Error::new("This copy of Adelite is not an app bundle, so it cannot update itself."));
    };
    // macOS runs a downloaded app it has not seen moved from a read-only copy.
    if bundle.to_string_lossy().contains("/AppTranslocation/") {
        return Err(Error::new("Move Adelite into Applications, start it from there, and update again."));
    }
    let root = bundle.parent().ok_or_else(|| Error::new("Adelite could not find its folder."))?;
    Ok(Place { root: root.to_path_buf(), app: bundle.to_path_buf() })
}

/// The program and the folder holding it.
#[cfg(not(any(windows, target_os = "macos")))]
fn place() -> Result<Place, Error> {
    let exe = std::env::current_exe().map_err(|e| Error::logged("Adelite could not find itself.", e.to_string()))?;
    let root = exe.parent().ok_or_else(|| Error::new("Adelite could not find its folder."))?;
    Ok(Place { root: root.to_path_buf(), app: root.join("adelite") })
}

/// What of the unpacked update replaces what: on macOS the bundle (under
/// whatever name it was given), on Linux each file beside the program.
#[cfg(not(windows))]
fn moves(place: &Place, staging: &Path) -> Result<Vec<(PathBuf, PathBuf)>, Error> {
    let not_ours = || Error::new("The download is not an Adelite release for this system.");
    if cfg!(target_os = "macos") {
        let bundle = staging.join("Adelite.app");
        if !bundle.join("Contents").join("MacOS").join("adelite").is_file() {
            return Err(not_ours());
        }
        return Ok(vec![(bundle, place.app.clone())]);
    }
    let moves: Vec<_> = fs::read_dir(staging)
        .map_err(|e| Error::logged("The download could not be unpacked.", e.to_string()))?
        .flatten()
        .map(|entry| (entry.path(), place.root.join(entry.file_name())))
        .collect();
    if !staging.join("adelite").is_file() {
        return Err(not_ours());
    }
    Ok(moves)
}

/// Where `path` waits, replaced, until the next start: `.name.adelite-old` beside it.
#[cfg(not(windows))]
fn old_name(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}{OLD}"))
}

/// Removes a file or a whole folder, if it is there. What cannot be removed
/// now (an installer still finishing, say) is left for the next start.
fn remove(path: &Path) {
    let _ = match path.symlink_metadata() {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(_) => Ok(()),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_takes_files_and_folders_and_minds_nothing_missing() {
        let dir = std::env::temp_dir().join(format!("adelite-remove-test-{}", std::process::id()));
        fs::create_dir_all(dir.join("inner")).expect("temp folder");
        fs::write(dir.join("inner").join("file"), b"x").expect("temp file");
        fs::write(dir.join("loose"), b"x").expect("temp file");
        remove(&dir.join("loose"));
        assert!(!dir.join("loose").exists());
        remove(&dir);
        assert!(!dir.exists());
        remove(&dir);
    }

    #[cfg(not(windows))]
    #[test]
    fn a_replaced_entry_waits_under_a_hidden_name() {
        assert_eq!(old_name(Path::new("/opt/adelite/adelite")), Path::new("/opt/adelite/.adelite.adelite-old"));
        assert_eq!(
            old_name(Path::new("/Applications/Adelite.app")),
            Path::new("/Applications/.Adelite.app.adelite-old")
        );
    }
}
