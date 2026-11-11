//! Operating-system integration through the tools each platform ships with:
//! opening URLs and folders, the native file pickers (open and save),
//! notifications, the tray icon ([`tray`]) and the app's sounds ([`sound`]).
//! No shell is involved: every argument is passed to the program directly,
//! and text that is not ours never becomes part of a script.
//!
//! The pickers block until the user answers, so call them on a worker thread.
//!
//! ponytail: the picker runs a helper process (PowerShell / `osascript` /
//! `zenity`/`kdialog`) instead of linking the platform dialog APIs; it takes
//! a moment to appear. Swap in direct Win32/Cocoa/portal calls if that grates.

#![expect(
    unsafe_code,
    reason = "the few system calls std has no safe form of: a message box, the tray icon, a sound, the local time"
)]

pub mod sound;
pub mod tray;

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Configures `command` to open no console window (Windows only; a GUI app
/// spawning console programs would otherwise flash one).
pub fn no_window(command: &mut Command) -> &mut Command {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Starts `command` detached from our stdio and reaps it in the background.
fn launch(mut command: Command) -> io::Result<()> {
    let mut child = no_window(&mut command).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Opens `url` in the default browser.
///
/// # Errors
/// The launcher could not be started.
pub fn open_url(url: &str) -> io::Result<()> {
    let command = if cfg!(target_os = "windows") {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    launch(command)
}

/// Shows `path` in the file manager.
///
/// # Errors
/// The file manager could not be started.
pub fn open_folder(path: &Path) -> io::Result<()> {
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut command = Command::new(program);
    command.arg(path);
    launch(command)
}

/// Shows a native error dialog with `title` and `message` and waits until
/// the user closes it. For failures the window cannot show (it failed to
/// start, or the app is crashing); safe from any thread. Does nothing where
/// no dialog helper is installed.
pub fn alert(title: &str, message: &str) {
    #[cfg(target_os = "windows")]
    {
        // `MessageBoxW` from user32, which every Windows GUI process loads.
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBoxW(owner: *mut std::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
        }
        /// `MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST`.
        const KIND: u32 = 0x10 | 0x1_0000 | 0x4_0000;
        let (text, caption) = (wide(message), wide(title));
        // SAFETY: both strings are NUL-terminated UTF-16 that outlive the
        // call, and a null owner is allowed.
        unsafe {
            MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), KIND);
        }
    }
    #[cfg(target_os = "macos")]
    {
        // The text goes in as arguments, never into the script itself.
        let mut command = Command::new("osascript");
        command.args([
            "-e",
            "on run argv",
            "-e",
            "display alert (item 1 of argv) message (item 2 of argv) as critical",
            "-e",
            "end run",
            title,
            message,
        ]);
        let _ = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut zenity = Command::new("zenity");
        zenity.args(["--error", "--no-markup", "--title", title, "--text", message]);
        let shown = zenity.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok();
        if !shown {
            let mut kdialog = Command::new("kdialog");
            kdialog.args(["--title", title, "--error", message]);
            let _ = kdialog.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
    }
}

/// `s` as NUL-terminated UTF-16 for Win32, without inner NULs.
#[cfg(target_os = "windows")]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().filter(|&c| c != 0).chain([0]).collect()
}

/// Tells the user what arrived while they were elsewhere, where the system
/// shows what apps have to say: the notification centre on macOS, the
/// desktop's notifications on Linux. On Windows a notification comes from
/// the app's icon in the tray ([`tray::Tray::notify`]), which also hears
/// the click on it.
#[derive(Debug, Default)]
pub struct Notifier;

impl Notifier {
    /// Shows `body` under `title`.
    pub fn show(&self, title: &str, body: &str) {
        #[cfg(target_os = "macos")]
        {
            // The words go in as arguments, never into the script itself.
            let mut command = Command::new("osascript");
            command.args([
                "-e",
                "on run argv",
                "-e",
                "display notification (item 2 of argv) with title (item 1 of argv)",
                "-e",
                "end run",
                title,
                body,
            ]);
            let _ = launch(command);
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let mut command = Command::new("notify-send");
            command.args(["--app-name=Adelite", "--icon=adelite", "--", title, body]);
            let _ = launch(command);
        }
        #[cfg(not(unix))]
        let _ = (title, body);
    }
}

/// Runs a picker helper and returns the non-empty lines it printed.
fn run_picker(mut command: Command) -> io::Result<Vec<String>> {
    let output = no_window(&mut command).stdin(Stdio::null()).stderr(Stdio::null()).output()?;
    // A cancelled dialog exits non-zero or prints nothing; both mean "none".
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

/// PowerShell prelude: the forms library and UTF-8 output.
const PS_PRELUDE: &str = "Add-Type -AssemblyName System.Windows.Forms; \
    [Console]::OutputEncoding = [Text.Encoding]::UTF8;";
/// What a dialog needs on top of the prelude: an invisible topmost owner,
/// so it appears in front of our window.
const PS_OWNER: &str = "$owner = New-Object System.Windows.Forms.Form -Property @{TopMost = $true; ShowInTaskbar = $false; Opacity = 0}; \
    $owner.Show(); $owner.Activate();";

/// Runs `script` in PowerShell with `vars` in its environment. Text that is
/// not ours (a story's title) goes in as a variable, never into the script.
///
/// `dialog`: the script shows one, which needs an owner in front of our
/// window. A script that shows nothing must not have one: it would take
/// the keyboard from the app for a moment.
fn powershell(script: &str, vars: &[(&str, &str)], dialog: bool) -> io::Result<Vec<String>> {
    let owner = if dialog { PS_OWNER } else { "" };
    // Windows PowerShell 5 is always present as a fallback for PowerShell 7.
    let run = |program: &str| {
        let mut command = Command::new(program);
        command.args(["-NoProfile", "-NonInteractive", "-STA", "-Command", &format!("{PS_PRELUDE} {owner} {script}")]);
        command.envs(vars.iter().copied());
        run_picker(command)
    };
    run("pwsh").or_else(|_| run("powershell"))
}

/// Kinds of file a picker offers: a name and the extensions, without dots.
pub type Filter<'a> = (&'a str, &'a [&'a str]);

/// The portrait picker's file types.
const IMAGES: Filter<'static> = ("Images", &["png", "jpg", "jpeg", "webp"]);

/// Asks the user for a PNG, JPEG or WebP image. `None` when cancelled.
///
/// # Errors
/// No dialog helper could be started.
pub fn pick_image() -> io::Result<Option<PathBuf>> {
    Ok(pick_files("Choose a picture", IMAGES, false)?.into_iter().next())
}

/// The files on the clipboard, for a paste that is not text: files copied
/// in the file manager, or else a copied picture, which is saved as a PNG
/// at `picture`. None when the clipboard holds neither.
///
/// # Errors
/// No helper could be started.
// ponytail: copied files are only read on Windows; elsewhere only a copied picture is.
pub fn clipboard_files(picture: &Path) -> io::Result<Vec<PathBuf>> {
    let out = picture.display().to_string();
    let lines = if cfg!(target_os = "windows") {
        let script = "Add-Type -AssemblyName System.Drawing; $c = [System.Windows.Forms.Clipboard]; \
            if ($c::ContainsFileDropList()) { $c::GetFileDropList() } \
            elseif ($c::ContainsImage()) { $c::GetImage().Save($env:ADELITE_OUT, [System.Drawing.Imaging.ImageFormat]::Png); \
            $env:ADELITE_OUT }";
        powershell(script, &[("ADELITE_OUT", &out)], false)?
    } else if cfg!(target_os = "macos") {
        let script = "try\n\
            set png to (the clipboard as «class PNGf»)\n\
            set f to open for access (POSIX file (item 1 of argv)) with write permission\n\
            write png to f\n\
            close access f\n\
            return item 1 of argv\n\
            end try\n\
            return \"\"";
        let mut command = Command::new("osascript");
        command.args(["-e", "on run argv", "-e", script, "-e", "end run", &out]);
        run_picker(command)?
    } else {
        // Wayland's clipboard first, then X11's.
        let bytes = |program: &str, args: &[&str]| {
            let output = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
            Some(output.stdout).filter(|bytes| output.status.success() && !bytes.is_empty())
        };
        let png = bytes("wl-paste", &["--type", "image/png"])
            .or_else(|| bytes("xclip", &["-selection", "clipboard", "-t", "image/png", "-o"]));
        match png {
            Some(png) => {
                std::fs::write(picture, png)?;
                vec![out]
            }
            None => Vec::new(),
        }
    };
    Ok(lines.into_iter().map(PathBuf::from).collect())
}

/// Asks the user for files of any kind; none when cancelled.
///
/// # Errors
/// No dialog helper could be started.
pub fn pick_any(title: &str) -> io::Result<Vec<PathBuf>> {
    pick_files(title, ("All files", &["*"]), true)
}

/// Asks the user for files of the kinds `filter` names (several when
/// `multiple`); none when cancelled.
///
/// # Errors
/// No dialog helper could be started.
pub fn pick_files(title: &str, (kind, extensions): Filter<'_>, multiple: bool) -> io::Result<Vec<PathBuf>> {
    let lines = if cfg!(target_os = "windows") {
        let filter = format!("{kind}|{}", extensions.iter().map(|e| format!("*.{e}")).collect::<Vec<_>>().join(";"));
        let multiple = if multiple { "$true" } else { "$false" };
        powershell(
            &format!(
                "$d = New-Object System.Windows.Forms.OpenFileDialog -Property @{{Title = $env:ADELITE_TITLE; Filter = $env:ADELITE_FILTER; Multiselect = {multiple}}}; \
                 if ($d.ShowDialog($owner) -eq 'OK') {{ $d.FileNames }}"
            ),
            &[("ADELITE_TITLE", title), ("ADELITE_FILTER", &filter)],
            true,
        )?
    } else if cfg!(target_os = "macos") {
        // Type identifiers, from our own list only: never text from elsewhere.
        let types: Vec<String> = extensions.iter().map(|e| format!("\"{}\"", uti(e))).collect();
        let several = if multiple { " with multiple selections allowed" } else { "" };
        let script = format!(
            "set picked to choose file with prompt (item 1 of argv) of type {{{}}}{several}\n\
             if class of picked is not list then set picked to {{picked}}\n\
             set out to \"\"\n\
             repeat with f in picked\n\
             set out to out & POSIX path of f & linefeed\n\
             end repeat\n\
             return out",
            types.join(", ")
        );
        let mut command = Command::new("osascript");
        command.args(["-e", "on run argv", "-e", &script, "-e", "end run", title]);
        run_picker(command)?
    } else {
        // Any file at all includes those without an extension.
        let pattern = |e: &&str| if *e == "*" { "*".to_owned() } else { format!("*.{e}") };
        let patterns: Vec<String> = extensions.iter().map(pattern).collect();
        let mut command = Command::new("zenity");
        command.args([
            "--file-selection",
            &format!("--title={title}"),
            &format!("--file-filter={kind} | {}", patterns.join(" ")),
        ]);
        if multiple {
            command.args(["--multiple", "--separator=\n"]);
        }
        run_picker(command).or_else(|_| {
            let mut command = Command::new("kdialog");
            command.args(["--getopenfilename", ".", &format!("{}|{kind}", patterns.join(" ")), "--title", title]);
            if multiple {
                command.args(["--multiple", "--separate-output"]);
            }
            run_picker(command)
        })?
    };
    Ok(lines.into_iter().map(PathBuf::from).collect())
}

/// macOS's type identifier for a file extension.
fn uti(extension: &str) -> &'static str {
    match extension {
        "png" => "public.png",
        "jpg" | "jpeg" => "public.jpeg",
        "webp" => "org.webmproject.webp",
        "json" => "public.json",
        _ => "public.data",
    }
}

/// Asks the user for a folder. `None` when cancelled.
///
/// # Errors
/// No dialog helper could be started.
pub fn pick_folder(title: &str) -> io::Result<Option<PathBuf>> {
    let lines = if cfg!(target_os = "windows") {
        powershell(
            "$d = New-Object System.Windows.Forms.FolderBrowserDialog -Property @{Description = $env:ADELITE_TITLE; UseDescriptionForTitle = $true}; if ($d.ShowDialog($owner) -eq 'OK') { $d.SelectedPath }",
            &[("ADELITE_TITLE", title)],
            true,
        )?
    } else if cfg!(target_os = "macos") {
        // The title goes in as an argument, never into the script itself.
        let script = "return POSIX path of (choose folder with prompt (item 1 of argv))";
        let mut command = Command::new("osascript");
        command.args(["-e", "on run argv", "-e", script, "-e", "end run", title]);
        run_picker(command)?
    } else {
        let mut command = Command::new("zenity");
        command.args(["--file-selection", "--directory", &format!("--title={title}")]);
        run_picker(command).or_else(|_| {
            let mut command = Command::new("kdialog");
            command.args(["--getexistingdirectory", ".", "--title", title]);
            run_picker(command)
        })?
    };
    Ok(lines.into_iter().next().map(PathBuf::from))
}

/// `~/.adelite`, the folder holding everything the app keeps.
///
/// # Errors
/// The platform reports no home directory.
pub fn data_dir() -> io::Result<PathBuf> {
    std::env::home_dir()
        .filter(|home| !home.as_os_str().is_empty())
        .map(|home| home.join(".adelite"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home directory"))
}

/// Seconds since the Unix epoch.
#[must_use]
pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Writes `bytes` to `path` atomically (a temporary file, then a rename),
/// readable only by the user on Unix: for settings and anything secret.
///
/// # Errors
/// The folder or file could not be written.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    let mut dirs = std::fs::DirBuilder::new();
    dirs.recursive(true);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        std::os::unix::fs::DirBuilderExt::mode(&mut dirs, 0o700);
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    }
    if let Some(dir) = path.parent() {
        dirs.create(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let written = options.open(&tmp).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    });
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// A moment on the user's clock and calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalTime {
    /// The year.
    pub year: i32,
    /// The month, 1 to 12.
    pub month: u8,
    /// The day of the month, 1 to 31.
    pub day: u8,
    /// The hour, 0 to 23.
    pub hour: u8,
    /// The minute.
    pub minute: u8,
    /// The day of the week: 0 is Sunday.
    pub weekday: u8,
}

impl LocalTime {
    /// The calendar day as one number that sorts and compares: `20261110`.
    #[must_use]
    pub fn date(self) -> i32 {
        self.year * 10_000 + i32::from(self.month) * 100 + i32::from(self.day)
    }
}

/// `unix` (seconds since the epoch) on the calendar of the user's time
/// zone, with the daylight saving of that day; in UTC where the system
/// will not say.
#[must_use]
pub fn local_time(unix: i64) -> LocalTime {
    zoned(unix).unwrap_or_else(|| civil(unix))
}

/// `unix` on the UTC calendar (the days-from-civil arithmetic, inverted).
#[must_use]
pub fn civil(unix: i64) -> LocalTime {
    let (days, seconds) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    // Days since 0000-03-01, in 400-year eras of 146097 days.
    let shifted = days + 719_468;
    let (era, day_of_era) = (shifted.div_euclid(146_097), shifted.rem_euclid(146_097));
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let month = if month_from_march < 10 { month_from_march + 3 } else { month_from_march - 9 };
    LocalTime {
        year: (year_of_era + era * 400 + i64::from(month <= 2)) as i32,
        month: month as u8,
        day: (day_of_year - (153 * month_from_march + 2) / 5 + 1) as u8,
        hour: (seconds / 3_600) as u8,
        minute: (seconds % 3_600 / 60) as u8,
        // 1970-01-01 was a Thursday.
        weekday: (days + 4).rem_euclid(7) as u8,
    }
}

#[cfg(windows)]
fn zoned(unix: i64) -> Option<LocalTime> {
    /// `SYSTEMTIME`.
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    unsafe extern "system" {
        fn FileTimeToSystemTime(file_time: *const u64, system_time: *mut SystemTime) -> i32;
        fn SystemTimeToTzSpecificLocalTime(
            zone: *const std::ffi::c_void,
            universal: *const SystemTime,
            local: *mut SystemTime,
        ) -> i32;
    }
    // A FILETIME counts 100 ns ticks from 1601; it is laid out as a `u64` on every Windows target.
    let ticks = u64::try_from(unix.checked_add(11_644_473_600)?.checked_mul(10_000_000)?).ok()?;
    let (mut universal, mut local) = (SystemTime::default(), SystemTime::default());
    // SAFETY: `ticks` and `universal` are valid for the read and the write of the call.
    if unsafe { FileTimeToSystemTime(&raw const ticks, &raw mut universal) } == 0 {
        return None;
    }
    // SAFETY: a null zone means the current one; both structs are valid for the call.
    if unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &raw const universal, &raw mut local) } == 0 {
        return None;
    }
    Some(LocalTime {
        year: i32::from(local.year),
        month: local.month as u8,
        day: local.day as u8,
        hour: local.hour as u8,
        minute: local.minute as u8,
        weekday: local.weekday as u8,
    })
}

#[cfg(unix)]
fn zoned(unix: i64) -> Option<LocalTime> {
    use std::ffi::{c_char, c_int, c_long};
    /// `struct tm`, with the two fields glibc, musl and the BSDs (macOS) add.
    #[repr(C)]
    struct Tm {
        second: c_int,
        minute: c_int,
        hour: c_int,
        day: c_int,
        month: c_int,
        year: c_int,
        weekday: c_int,
        day_of_year: c_int,
        daylight: c_int,
        offset: c_long,
        zone: *const c_char,
    }
    unsafe extern "C" {
        fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
    }
    let mut tm = std::mem::MaybeUninit::<Tm>::uninit();
    // SAFETY: `unix` is a valid `time_t` (64 bits on the 64-bit targets we
    // build for) and `tm` is valid for the write.
    let filled = unsafe { localtime_r(&raw const unix, tm.as_mut_ptr()) };
    if filled.is_null() {
        return None;
    }
    // SAFETY: a non-null result means `localtime_r` filled `tm`.
    let tm = unsafe { tm.assume_init() };
    Some(LocalTime {
        year: tm.year + 1900,
        month: (tm.month + 1) as u8,
        day: tm.day as u8,
        hour: tm.hour as u8,
        minute: tm.minute as u8,
        weekday: tm.weekday as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_seconds_land_on_the_calendar() {
        let at = |unix| {
            let t = civil(unix);
            (t.year, t.month, t.day, t.hour, t.minute, t.weekday)
        };
        assert_eq!(at(0), (1970, 1, 1, 0, 0, 4));
        assert_eq!(at(951_782_400), (2000, 2, 29, 0, 0, 2), "a leap day in a century year");
        assert_eq!(at(1_794_268_800 + 19 * 3_600 + 5 * 60), (2026, 11, 10, 19, 5, 2));
        assert_eq!(at(-1), (1969, 12, 31, 23, 59, 3), "before the epoch");
        assert_eq!(civil(1_794_268_800).date(), 20_261_110);
        // The zoned calendar is a day from UTC's at most, and the ends of time are no reason to panic.
        let (local, utc) = (local_time(1_794_268_800 + 43_200), civil(1_794_268_800 + 43_200));
        assert_eq!((local.year, local.month), (utc.year, utc.month));
        assert!((i32::from(local.day) - i32::from(utc.day)).abs() <= 1);
        let _ = (local_time(i64::MAX), local_time(i64::MIN));
    }
}
