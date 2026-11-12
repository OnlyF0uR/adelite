//! TDLib's JSON interface (`tdjson`), loaded at run time: the only unsafe
//! code of the Telegram backend.
//!
//! The library is opened when the app starts rather than linked when it is
//! built, so Adelite builds and runs without TDLib and says what is missing
//! instead of failing to start. It is never closed: TDLib's own threads
//! live in it for as long as the process does.
//!
//! The interface is four C functions. Requests and answers are JSON text;
//! `td_send` may be called from any thread, `td_receive` from one at a
//! time, which [`Inbox`] guarantees by being the only way to call it.

#![expect(unsafe_code, reason = "the C interface of TDLib, and loading the library that has it")]

use std::ffi::{CStr, CString, c_char, c_double, c_int, c_void};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Why TDLib could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// No library could be opened at any of these places.
    NotFound(Vec<PathBuf>),
    /// The library at this path lacks a function of the JSON interface.
    Incomplete(PathBuf, &'static str),
    /// Something in this process is already receiving from TDLib, which only one may.
    InUse,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(tried) => {
                write!(f, "TDLib ({FILE_NAME}) was not found; looked at")?;
                tried.iter().try_for_each(|path| write!(f, " {};", path.display()))
            }
            Self::Incomplete(path, symbol) => write!(f, "{} has no `{symbol}`: not TDLib's tdjson", path.display()),
            Self::InUse => f.write_str("TDLib is already in use in this process"),
        }
    }
}

impl std::error::Error for LoadError {}

/// The library's file name on this platform.
pub const FILE_NAME: &str = if cfg!(windows) {
    "tdjson.dll"
} else if cfg!(target_os = "macos") {
    "libtdjson.dylib"
} else {
    "libtdjson.so"
};

/// The loaded library: the functions any thread may call.
#[derive(Debug)]
pub struct Library {
    create_client_id: unsafe extern "C" fn() -> c_int,
    send: unsafe extern "C" fn(c_int, *const c_char),
    receive: unsafe extern "C" fn(c_double) -> *const c_char,
    execute: unsafe extern "C" fn(*const c_char) -> *const c_char,
}

/// The one receiver of what TDLib sends back, for every client. There is
/// at most one in the process at a time: TDLib aborts when two threads
/// receive at once.
#[derive(Debug)]
pub struct Inbox {
    library: Arc<Library>,
}

/// Whether an [`Inbox`] exists.
static RECEIVING: AtomicBool = AtomicBool::new(false);

impl Drop for Inbox {
    fn drop(&mut self) {
        RECEIVING.store(false, Ordering::Release);
    }
}

impl Library {
    /// Opens the first library found at `places` and hands back its
    /// functions and the receiving end.
    ///
    /// # Errors
    /// See [`LoadError`].
    pub fn open(places: &[PathBuf]) -> Result<(Arc<Self>, Inbox), LoadError> {
        if RECEIVING.swap(true, Ordering::AcqRel) {
            return Err(LoadError::InUse);
        }
        // From here on, leaving without an `Inbox` gives the place back.
        let opened = Self::load(places);
        if opened.is_err() {
            RECEIVING.store(false, Ordering::Release);
        }
        let library = Arc::new(opened?);
        Ok((Arc::clone(&library), Inbox { library }))
    }

    /// Opens the first library found at `places` and finds its functions.
    fn load(places: &[PathBuf]) -> Result<Self, LoadError> {
        let Some((path, handle)) = places.iter().find_map(|path| Some((path, sys::open(path)?))) else {
            return Err(LoadError::NotFound(places.to_vec()));
        };
        let symbol = |name: &'static CStr, shown: &'static str| {
            sys::symbol(handle, name).ok_or_else(|| LoadError::Incomplete(path.clone(), shown))
        };
        let create_client_id = symbol(c"td_create_client_id", "td_create_client_id")?;
        let send = symbol(c"td_send", "td_send")?;
        let receive = symbol(c"td_receive", "td_receive")?;
        let execute = symbol(c"td_execute", "td_execute")?;
        // SAFETY: each pointer is the non-null address of the function of
        // that name in TDLib's tdjson, whose C signature (td_json_client.h)
        // is the one it is turned into; the library is never unloaded, so
        // the addresses stay valid for the life of the process.
        let library = unsafe {
            Self {
                create_client_id: std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> c_int>(create_client_id),
                send: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(c_int, *const c_char)>(send),
                receive: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(c_double) -> *const c_char>(receive),
                execute: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_char) -> *const c_char>(
                    execute,
                ),
            }
        };
        Ok(library)
    }

    /// A new client's id. It starts when the first request is sent to it.
    #[must_use]
    pub fn client(&self) -> i32 {
        // SAFETY: `td_create_client_id` takes nothing and may be called from any thread.
        unsafe { (self.create_client_id)() }
    }

    /// Sends `request` (a JSON object) to `client`. Never blocks; the answer
    /// comes through the [`Inbox`]. A request holding a NUL byte is not JSON
    /// and is dropped.
    pub fn send(&self, client: i32, request: &str) {
        let Ok(request) = CString::new(request) else { return };
        // SAFETY: `request` is a NUL-terminated string that outlives the
        // call; `td_send` copies it and may be called from any thread.
        unsafe { (self.send)(client, request.as_ptr()) }
    }

    /// Runs one of the few requests TDLib answers at once, without a client
    /// (parsing text, setting the log level), and returns its JSON answer.
    #[must_use]
    pub fn execute(&self, request: &str) -> Option<String> {
        let request = CString::new(request).ok()?;
        // SAFETY: `request` is a NUL-terminated string that outlives the call.
        let answer = unsafe { (self.execute)(request.as_ptr()) };
        if answer.is_null() {
            return None;
        }
        // SAFETY: a non-null answer is a NUL-terminated string that stays
        // valid until this thread calls `td_execute` or `td_receive` again,
        // which it cannot before the copy below is made.
        let answer = unsafe { CStr::from_ptr(answer) };
        Some(answer.to_string_lossy().into_owned())
    }
}

impl Inbox {
    /// Waits up to `timeout` seconds for the next update or answer of any
    /// client and hands its JSON to `read`. `None` when nothing came.
    pub fn receive<R>(&mut self, timeout: f64, read: impl FnOnce(&str) -> R) -> Option<R> {
        // SAFETY: `&mut self` on the only `Inbox` means no other thread is
        // inside `td_receive`, which is all it requires.
        let answer = unsafe { (self.library.receive)(timeout) };
        if answer.is_null() {
            return None;
        }
        // SAFETY: a non-null answer is a NUL-terminated string that stays
        // valid until the next `td_receive` on this thread, which `&mut
        // self` keeps from happening while `read` borrows it.
        let answer = unsafe { CStr::from_ptr(answer) };
        Some(read(&answer.to_string_lossy()))
    }
}

/// Opening a shared library and finding a function in it, per platform.
#[cfg(windows)]
mod sys {
    use std::ffi::{CStr, c_char, c_void};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    /// Look for the DLLs a library needs beside it, not beside the app.
    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x8;

    unsafe extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    /// Opens the library at `path`; `None` if there is none that loads.
    pub fn open(path: &Path) -> Option<*mut c_void> {
        // The altered search path needs an absolute path; a bare name is left to the system's search.
        let flags = if path.is_absolute() { LOAD_WITH_ALTERED_SEARCH_PATH } else { 0 };
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the
        // call, the file handle must be null, and the flags are valid.
        let handle = unsafe { LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), flags) };
        (!handle.is_null()).then_some(handle)
    }

    /// The address of `name` in the library `handle`.
    pub fn symbol(handle: *mut c_void, name: &CStr) -> Option<*mut c_void> {
        // SAFETY: `handle` came from `open` and is never freed; `name` is NUL-terminated.
        let address = unsafe { GetProcAddress(handle, name.as_ptr()) };
        (!address.is_null()).then_some(address)
    }
}

/// Opening a shared library and finding a function in it, per platform.
#[cfg(unix)]
mod sys {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    /// Resolve every symbol when the library is opened.
    const RTLD_NOW: c_int = 2;

    #[cfg_attr(target_os = "linux", link(name = "dl"))]
    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    /// Opens the library at `path`; `None` if there is none that loads.
    pub fn open(path: &Path) -> Option<*mut c_void> {
        let name = CString::new(path.as_os_str().as_bytes()).ok()?;
        // SAFETY: `name` is a NUL-terminated string that outlives the call.
        let handle = unsafe { dlopen(name.as_ptr(), RTLD_NOW) };
        (!handle.is_null()).then_some(handle)
    }

    /// The address of `name` in the library `handle`.
    pub fn symbol(handle: *mut c_void, name: &CStr) -> Option<*mut c_void> {
        // SAFETY: `handle` came from `open` and is never closed; `name` is NUL-terminated.
        let address = unsafe { dlsym(handle, name.as_ptr()) };
        (!address.is_null()).then_some(address)
    }
}

/// Where to look for the library, in order: where `ADELITE_TDJSON` points,
/// beside the app (and in its bundle's `Contents/Frameworks` on macOS), in a
/// source checkout's `vendor/tdlib` (a few folders up from a build in
/// `target/`), in the app's data folder (`dir`), and wherever the system
/// looks for libraries.
#[must_use]
pub fn places(dir: &Path) -> Vec<PathBuf> {
    let mut places = Vec::new();
    places.extend(std::env::var_os("ADELITE_TDJSON").map(PathBuf::from));
    if let Some(beside) = std::env::current_exe().ok().as_deref().and_then(Path::parent) {
        places.push(beside.join(FILE_NAME));
        places.push(beside.join("tdlib").join(FILE_NAME));
        if cfg!(target_os = "macos")
            && let Some(contents) = beside.parent()
        {
            places.push(contents.join("Frameworks").join(FILE_NAME));
        }
        // `target/release` is two folders under the checkout, a test's `target/debug/deps` three.
        places.extend(beside.ancestors().skip(2).take(2).map(|up| up.join("vendor").join("tdlib").join(FILE_NAME)));
    }
    places.push(dir.join(FILE_NAME));
    places.push(PathBuf::from(FILE_NAME));
    places
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_library_says_where_it_looked() {
        let nowhere = [PathBuf::from("/nowhere/at/all").join(FILE_NAME)];
        let error = Library::open(&nowhere).expect_err("nothing is there");
        assert!(matches!(&error, LoadError::NotFound(tried) if tried == &nowhere));
        assert!(error.to_string().contains("nowhere"), "{error}");
    }

    /// Loads the real library and asks it something that needs no account:
    /// `ADELITE_TDJSON=<path to tdjson> cargo test -p adelite-telegram tdlib -- --ignored`
    #[test]
    #[ignore = "needs TDLib; run by hand after building it"]
    fn tdlib_loads_and_answers() {
        let _alone = crate::TDLIB.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (library, mut inbox) = Library::open(&places(Path::new("."))).expect("TDLib");
        let answer = library.execute(r#"{"@type":"setLogVerbosityLevel","new_verbosity_level":0}"#).expect("an answer");
        assert!(answer.contains(r#""@type":"ok""#), "{answer}");
        let text = library
            .execute(r#"{"@type":"parseMarkdown","text":{"@type":"formattedText","text":"**bold** move"}}"#)
            .expect("an answer");
        assert!(text.contains("textEntityTypeBold") && text.contains(r#""text":"bold move""#), "{text}");
        // A client starts with its first request, and first says it wants its parameters.
        let client = library.client();
        library.send(client, r#"{"@type":"getOption","name":"version","@extra":"v"}"#);
        let mut seen = (false, false);
        for _ in 0..50 {
            let Some(update) = inbox.receive(1.0, str::to_owned) else { continue };
            seen.0 |= update.contains("authorizationStateWaitTdlibParameters");
            seen.1 |= update.contains(r#""@extra":"v""#) && update.contains("optionValueString");
            if seen == (true, true) {
                break;
            }
        }
        assert_eq!(seen, (true, true), "the state and the version both arrive");
        // One receiver at a time: a second is refused while the first lives.
        assert!(matches!(Library::open(&places(Path::new("."))), Err(LoadError::InUse)));
        drop(inbox);
        assert!(Library::open(&places(Path::new("."))).is_ok());
    }
}
