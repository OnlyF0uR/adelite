//! HTTPS through the system's `curl`. Windows (10 and later), macOS and
//! practically every Linux desktop come with it, so the app carries no TLS
//! stack of its own for the two or three requests an update makes. Blocking:
//! called on the updater's thread.

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::Error;
use crate::version::Version;

/// How often a download's progress is looked at.
const PROGRESS: Duration = Duration::from_millis(200);

/// `curl`, wherever the system keeps it, with what every request shares:
/// HTTPS only (redirects too), quiet but for errors, and the app's name.
fn curl() -> Command {
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        /// Keeps a console window from flashing up: curl is a console program.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let mut command = Command::new(Path::new(&root).join("System32").join("curl.exe"));
        command.creation_flags(CREATE_NO_WINDOW);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = Command::new("/usr/bin/curl");
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut command = Command::new("curl");
    command
        .args(["--silent", "--show-error", "--location", "--proto", "=https", "--proto-redir", "=https"])
        .args(["--connect-timeout", "20", "--user-agent", &format!("Adelite/{}", Version::current())])
        .stdin(Stdio::null());
    command
}

/// Asks for `url` and returns the status and the body.
///
/// # Errors
/// No answer came: curl is missing, or the network or GitHub failed.
pub fn get(url: &str, accept: &str) -> Result<(u16, Vec<u8>), Error> {
    let output = curl()
        .args(["--max-time", "30", "--header", &format!("Accept: {accept}"), "--write-out", "\n%{http_code}", url])
        .output()
        .map_err(|e| started(&e))?;
    if !output.status.success() {
        return Err(failed(output.status.code(), &output.stderr));
    }
    // The status comes last, on a line of its own.
    let mut body = output.stdout;
    let at = body.iter().rposition(|&b| b == b'\n').unwrap_or(0);
    let status = std::str::from_utf8(&body[at..]).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    body.truncate(at);
    Ok((status, body))
}

/// Saves `url` as `to`, telling `progress` how many bytes have arrived.
///
/// # Errors
/// curl is missing, or the download failed or was refused.
pub fn download(url: &str, to: &Path, mut progress: impl FnMut(u64)) -> Result<(), Error> {
    let mut child = curl()
        .args(["--fail", "--max-time", "3600", "--output"])
        .arg(to)
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| started(&e))?;
    let mut told = 0;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                return Err(Error::new(format!("The download stopped: {e}")));
            }
        }
        std::thread::sleep(PROGRESS);
        let arrived = std::fs::metadata(to).map_or(0, |m| m.len());
        if arrived != told {
            told = arrived;
            progress(arrived);
        }
    };
    if !status.success() {
        return Err(failed(status.code(), &stderr(&mut child)));
    }
    progress(std::fs::metadata(to).map_or(told, |m| m.len()));
    Ok(())
}

/// What curl said about why it failed.
fn stderr(child: &mut Child) -> Vec<u8> {
    let mut said = Vec::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_end(&mut said);
    }
    said
}

/// curl could not be started.
fn started(e: &std::io::Error) -> Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        Error::new("Adelite needs curl to download updates, and it was not found.")
    } else {
        Error::new(format!("curl could not be started: {e}"))
    }
}

/// What curl's exit `code` means for the user; `said` goes to the log.
fn failed(code: Option<i32>, said: &[u8]) -> Error {
    let said = String::from_utf8_lossy(said);
    let said = said.trim();
    let message = match code {
        // Name not resolved, no connection, timed out, TLS failed.
        Some(5..=7 | 28 | 35 | 60) => "Adelite could not reach GitHub. Check the connection and try again.".to_owned(),
        // `--fail`: the server answered with an error.
        Some(22) => "GitHub refused the download. Try again in a while.".to_owned(),
        Some(23) => "The download could not be saved.".to_owned(),
        _ if said.is_empty() => format!("The download failed (curl {}).", code.unwrap_or(-1)),
        _ => format!("The download failed: {said}"),
    };
    Error::logged(message, said)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Downloads a file for real: `cargo test -p adelite-update github -- --ignored`
    #[test]
    #[ignore = "downloads from GitHub over the network; run by hand"]
    fn github_downloads_arrive_whole() {
        let to = std::env::temp_dir().join(format!("adelite-download-test-{}", std::process::id()));
        let mut told = Vec::new();
        let url = format!("https://github.com/{}/raw/HEAD/LICENSE-MIT", crate::feed::REPOSITORY);
        download(&url, &to, |done| told.push(done)).unwrap_or_else(|e| panic!("{e} {}", e.detail()));
        let text = std::fs::read_to_string(&to).expect("saved");
        let _ = std::fs::remove_file(&to);
        assert!(text.contains("MIT"), "{text}");
        assert_eq!(told.last().copied(), Some(text.len() as u64));
        // A file that is not there fails as a refusal, not as a saved error page.
        let missing = format!("https://github.com/{}/raw/HEAD/no-such-file", crate::feed::REPOSITORY);
        assert!(download(&missing, &to, |_| {}).is_err());
        let _ = std::fs::remove_file(&to);
    }

    #[test]
    fn curl_failures_read_as_sentences() {
        assert!(failed(Some(6), b"curl: (6) Could not resolve host").to_string().contains("could not reach GitHub"));
        assert!(failed(Some(22), b"").to_string().contains("refused"));
        assert_eq!(failed(Some(99), b"").to_string(), "The download failed (curl 99).");
        assert_eq!(failed(Some(99), b" odd \n").to_string(), "The download failed: odd");
    }
}
