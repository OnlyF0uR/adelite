//! The icon in the Windows notification area (the tray): it keeps the app
//! within reach while its window is hidden, and it is where notifications
//! come from.
//!
//! Windows: the icon is there while the window is hidden and while a
//! notification waits to be seen. A click on it, or on the notification,
//! asks for the window ([`TrayEvent::Open`]); its menu opens the window or
//! quits. A second start of the app asks the running one to show itself
//! ([`wake_running`]). Elsewhere there is no tray: macOS hides the app
//! behind its Dock icon, and Linux has none without a D-Bus client
//! (`StatusNotifierItem`), so [`Tray::start`] says so and the app minimises
//! instead.

/// What the user did with the tray icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// Open the window: a click on the icon or on a notification, the
    /// icon's menu, or a second start of the app.
    Open,
    /// Quit, from the icon's menu.
    Quit,
}

#[cfg(all(windows, target_pointer_width = "64"))]
pub use windows::{Tray, wake_running};

/// The tray where there is none: nothing to show.
#[cfg(not(all(windows, target_pointer_width = "64")))]
#[derive(Debug)]
pub struct Tray;

#[cfg(not(all(windows, target_pointer_width = "64")))]
impl Tray {
    /// There is no tray here.
    ///
    /// # Errors
    /// Always: this platform has no tray.
    pub fn start(_: impl Fn(TrayEvent) + Send + 'static) -> std::io::Result<Self> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no tray on this platform"))
    }

    /// Does nothing.
    pub fn show(&self) {}

    /// Does nothing.
    pub fn hide(&self) {}

    /// Does nothing.
    pub fn notify(&self, _: &str, _: &str) {}

    /// Does nothing.
    pub fn clear(&self) {}
}

/// There is no running copy to ask where there is no tray.
#[cfg(not(all(windows, target_pointer_width = "64")))]
#[must_use]
pub fn wake_running() -> bool {
    false
}

#[cfg(all(windows, target_pointer_width = "64"))]
mod windows {
    //! `Shell_NotifyIconW` and a hidden window of our own to receive its
    //! clicks, on a thread with its own message loop. The window is a
    //! top-level one that is never shown, not a message-only one: only
    //! top-level windows hear Explorer's `TaskbarCreated` (sent when it
    //! restarts, which drops every icon), and only they can take the
    //! foreground that a popup menu needs to close properly.

    use std::cell::{Cell, OnceCell};
    use std::ffi::c_void;
    use std::io;
    use std::sync::{Arc, Mutex, PoisonError, mpsc};
    use std::thread::JoinHandle;

    use super::TrayEvent;

    type Handle = *mut c_void;
    type Procedure = unsafe extern "system" fn(Handle, u32, usize, isize) -> isize;
    /// A notification's title and words, on their way to the tray's thread.
    type Words = Arc<Mutex<Option<(String, String)>>>;

    /// The hidden window's class; another start of the app finds it by it.
    const CLASS: &str = if cfg!(test) { "AdeliteTrayTest" } else { "AdeliteTray" };
    const WM_NULL: u32 = 0;
    const WM_DESTROY: u32 = 0x0002;
    const WM_CLOSE: u32 = 0x0010;
    const WM_CONTEXTMENU: u32 = 0x007B;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_RBUTTONUP: u32 = 0x0205;
    /// `NIN_BALLOONUSERCLICK`: the notification was clicked.
    const NOTIFICATION_CLICKED: u32 = 0x0405;
    const WM_APP: u32 = 0x8000;
    /// What the icon sends: its mouse message in `lparam`.
    const CALLBACK: u32 = WM_APP + 1;
    /// The window is hidden: the icon stays in the tray.
    const SHOW: u32 = WM_APP + 2;
    /// The window is back: the icon is not needed for it.
    const HIDE: u32 = WM_APP + 3;
    /// Another start of the app asks for the window.
    const WAKE: u32 = WM_APP + 4;
    /// A notification waits in the shared words.
    const NOTIFY: u32 = WM_APP + 5;
    /// The notification was seen.
    const CLEAR: u32 = WM_APP + 6;
    const NIM_ADD: u32 = 0;
    const NIM_MODIFY: u32 = 1;
    const NIM_DELETE: u32 = 2;
    /// `NIF_MESSAGE | NIF_ICON | NIF_TIP`, and `NIF_INFO` for a notification.
    const ICON_FLAGS: u32 = 0x1 | 0x2 | 0x4;
    const NIF_INFO: u32 = 0x10;
    /// `NIIF_USER | NIIF_LARGE_ICON | NIIF_RESPECT_QUIET_TIME`: the app's own icon, and quiet during focus time.
    const INFO_FLAGS: u32 = 0x4 | 0x20 | 0x80;
    const MF_STRING: u32 = 0;
    const MF_SEPARATOR: u32 = 0x0800;
    /// `TPM_RIGHTBUTTON | TPM_NONOTIFY | TPM_RETURNCMD`.
    const MENU_FLAGS: u32 = 0x0002 | 0x0080 | 0x0100;
    const IMAGE_ICON: u32 = 1;
    const SM_CXSMICON: i32 = 49;
    const SM_CYSMICON: i32 = 50;
    /// `IDI_APPLICATION`, the stock icon when ours is not embedded.
    const IDI_APPLICATION: usize = 32512;
    /// `ASFW_ANY`.
    const ANY_PROCESS: u32 = u32::MAX;
    /// The menu's choices.
    const OPEN: usize = 1;
    const QUIT: usize = 2;

    #[repr(C)]
    struct WindowClass {
        style: u32,
        procedure: Procedure,
        class_extra: i32,
        window_extra: i32,
        instance: Handle,
        icon: Handle,
        cursor: Handle,
        background: Handle,
        menu_name: *const u16,
        class_name: *const u16,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct Msg {
        window: Handle,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        point: Point,
        private: u32,
    }

    /// `NOTIFYICONDATAW`, as the 64-bit headers lay it out (976 bytes).
    #[repr(C)]
    struct NotifyIconData {
        size: u32,
        window: Handle,
        id: u32,
        flags: u32,
        callback: u32,
        icon: Handle,
        tip: [u16; 128],
        state: u32,
        state_mask: u32,
        info: [u16; 256],
        version: u32,
        info_title: [u16; 64],
        info_flags: u32,
        guid: [u32; 4],
        balloon_icon: Handle,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterClassW(class: *const WindowClass) -> u16;
        fn CreateWindowExW(
            ex_style: u32,
            class: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: Handle,
            menu: Handle,
            instance: Handle,
            param: *mut c_void,
        ) -> Handle;
        fn DefWindowProcW(window: Handle, message: u32, wparam: usize, lparam: isize) -> isize;
        fn GetMessageW(message: *mut Msg, window: Handle, min: u32, max: u32) -> i32;
        fn TranslateMessage(message: *const Msg) -> i32;
        fn DispatchMessageW(message: *const Msg) -> isize;
        fn PostMessageW(window: Handle, message: u32, wparam: usize, lparam: isize) -> i32;
        fn PostQuitMessage(code: i32);
        fn DestroyWindow(window: Handle) -> i32;
        fn FindWindowW(class: *const u16, name: *const u16) -> Handle;
        fn RegisterWindowMessageW(name: *const u16) -> u32;
        fn CreatePopupMenu() -> Handle;
        fn AppendMenuW(menu: Handle, flags: u32, id: usize, text: *const u16) -> i32;
        fn TrackPopupMenu(
            menu: Handle,
            flags: u32,
            x: i32,
            y: i32,
            reserved: i32,
            window: Handle,
            rect: *const c_void,
        ) -> i32;
        fn DestroyMenu(menu: Handle) -> i32;
        fn SetForegroundWindow(window: Handle) -> i32;
        fn AllowSetForegroundWindow(process: u32) -> i32;
        fn GetCursorPos(point: *mut Point) -> i32;
        fn LoadImageW(instance: Handle, name: *const u16, kind: u32, width: i32, height: i32, flags: u32) -> Handle;
        fn LoadIconW(instance: Handle, name: *const u16) -> Handle;
        fn DestroyIcon(icon: Handle) -> i32;
        fn GetSystemMetrics(index: i32) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn Shell_NotifyIconW(message: u32, data: *const NotifyIconData) -> i32;
    }

    thread_local! {
        /// Where the tray thread's events go.
        static SINK: OnceCell<Box<dyn Fn(TrayEvent)>> = const { OnceCell::new() };
        /// Where a notification's words wait for the tray's thread.
        static WORDS: OnceCell<Words> = const { OnceCell::new() };
        /// The window is hidden: the icon stays in the tray.
        static KEPT: Cell<bool> = const { Cell::new(false) };
        /// A notification waits to be seen: the icon stays in the tray.
        static NOTED: Cell<bool> = const { Cell::new(false) };
        /// The icon is in the tray.
        static ADDED: Cell<bool> = const { Cell::new(false) };
        /// The icon, and whether it is ours to destroy.
        static ICON: Cell<(usize, bool)> = const { Cell::new((0, false)) };
        /// The message Explorer sends when it restarts.
        static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    }

    /// `s` as a NUL-terminated UTF-16 string, without inner NULs.
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().filter(|&unit| unit != 0).chain(Some(0)).collect()
    }

    /// Copies `text` into `to`, cut to fit with its terminator.
    fn fill(to: &mut [u16], text: &str) {
        let room = to.len().saturating_sub(1);
        for (slot, unit) in to.iter_mut().zip(text.encode_utf16().filter(|&unit| unit != 0).take(room)) {
            *slot = unit;
        }
    }

    /// The tray icon's thread and hidden window. Dropping it takes the icon
    /// away and ends the thread.
    #[derive(Debug)]
    pub struct Tray {
        /// The hidden window, as a number so the handle can cross threads.
        window: usize,
        words: Words,
        thread: Option<JoinHandle<()>>,
    }

    impl Tray {
        /// Starts the tray's thread; `on_event` runs on it for each click.
        /// The icon is not in the tray until [`Tray::show`] or [`Tray::notify`].
        ///
        /// # Errors
        /// The thread or its window could not be made.
        pub fn start(on_event: impl Fn(TrayEvent) + Send + 'static) -> io::Result<Self> {
            let (made, window) = mpsc::channel();
            let words = Words::default();
            let theirs = Arc::clone(&words);
            let thread = std::thread::Builder::new().name("adelite-tray".into()).spawn(move || {
                SINK.with(|sink| drop(sink.set(Box::new(on_event))));
                WORDS.with(|words| drop(words.set(theirs)));
                let window = create();
                let failed = window.is_null();
                let _ = made.send(if failed { Err(io::Error::last_os_error()) } else { Ok(window as usize) });
                if !failed {
                    run();
                }
                let (icon, owned) = ICON.get();
                if owned {
                    // SAFETY: the icon was loaded by `create` on this thread and is in no tray any more.
                    unsafe { DestroyIcon(icon as Handle) };
                }
            })?;
            let window = window.recv().map_err(|_| io::Error::other("the tray thread ended"))??;
            Ok(Self { window, words, thread: Some(thread) })
        }

        /// The window is hidden: the icon stays in the tray until [`Tray::hide`].
        pub fn show(&self) {
            self.post(SHOW);
        }

        /// The window is back: the icon goes, unless a notification still waits.
        pub fn hide(&self) {
            self.post(HIDE);
        }

        /// Shows `body` under `title` as a notification from the icon. A second one replaces the first.
        pub fn notify(&self, title: &str, body: &str) {
            *self.words.lock().unwrap_or_else(PoisonError::into_inner) = Some((title.to_owned(), body.to_owned()));
            self.post(NOTIFY);
        }

        /// The notification was seen: it is taken back, and the icon with it unless the window is hidden.
        pub fn clear(&self) {
            self.post(CLEAR);
        }

        fn post(&self, message: u32) {
            // SAFETY: a post to our own window by its handle; it fails harmlessly once the window is gone.
            unsafe { PostMessageW(self.window as Handle, message, 0, 0) };
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            self.post(WM_CLOSE);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// Asks a running copy of the app to show its window. Returns whether
    /// there was one to ask.
    #[must_use]
    pub fn wake_running() -> bool {
        let class = wide(CLASS);
        // SAFETY: a lookup by a NUL-terminated class name that outlives the call; a null title matches any.
        let window = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
        if window.is_null() {
            return false;
        }
        // We were just started by the user, so we may let it come to the front.
        // SAFETY: takes a process id or this constant, and no pointers.
        unsafe { AllowSetForegroundWindow(ANY_PROCESS) };
        // SAFETY: a post to the window just found; it fails harmlessly if it went meanwhile.
        unsafe { PostMessageW(window, WAKE, 0, 0) != 0 }
    }

    /// Registers the class and makes the hidden window, and loads the icon.
    /// Null when the window could not be made.
    fn create() -> Handle {
        let (class, title, taskbar) = (wide(CLASS), wide("Adelite"), wide("TaskbarCreated"));
        // SAFETY: a null name asks for the handle of this executable, which is always loaded.
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let description = WindowClass {
            style: 0,
            procedure,
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: std::ptr::null_mut(),
            cursor: std::ptr::null_mut(),
            background: std::ptr::null_mut(),
            menu_name: std::ptr::null(),
            class_name: class.as_ptr(),
        };
        // Fails only if it is already registered (a second tray in the tests).
        // SAFETY: `description` is a whole WNDCLASSW whose class name outlives the call.
        unsafe { RegisterClassW(&raw const description) };
        // SAFETY: a NUL-terminated name that outlives the call.
        TASKBAR_CREATED.set(unsafe { RegisterWindowMessageW(taskbar.as_ptr()) });
        // SAFETY: takes an index and no pointers.
        let width = unsafe { GetSystemMetrics(SM_CXSMICON) };
        // SAFETY: as above.
        let height = unsafe { GetSystemMetrics(SM_CYSMICON) };
        // The build embeds the app's icon as resource 1.
        // SAFETY: the name is a resource number (as `MAKEINTRESOURCE` makes it), never read as a string.
        let icon = unsafe { LoadImageW(instance, std::ptr::without_provenance(1), IMAGE_ICON, width, height, 0) };
        if icon.is_null() {
            // SAFETY: a null module with a resource number asks for a stock icon, which is never freed.
            let stock = unsafe { LoadIconW(std::ptr::null_mut(), std::ptr::without_provenance(IDI_APPLICATION)) };
            ICON.set((stock as usize, false));
        } else {
            ICON.set((icon as usize, true));
        }
        // SAFETY: the class was registered above; both strings are NUL-terminated and outlive the
        // call; no parent, menu or creation data.
        unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null_mut(),
            )
        }
    }

    /// The thread's message loop, until the window is destroyed.
    fn run() {
        let mut message = Msg {
            window: std::ptr::null_mut(),
            message: 0,
            wparam: 0,
            lparam: 0,
            time: 0,
            point: Point::default(),
            private: 0,
        };
        loop {
            // SAFETY: `message` is a whole MSG, valid for the write; a null window takes every message of this thread.
            let got = unsafe { GetMessageW(&raw mut message, std::ptr::null_mut(), 0, 0) };
            if got <= 0 {
                return;
            }
            // SAFETY: the MSG was just filled by `GetMessageW`.
            unsafe { TranslateMessage(&raw const message) };
            // SAFETY: as above; it calls `procedure`, on this thread.
            unsafe { DispatchMessageW(&raw const message) };
        }
    }

    fn emit(event: TrayEvent) {
        SINK.with(|sink| {
            if let Some(sink) = sink.get() {
                sink(event);
            }
        });
    }

    /// Adds, changes or removes the icon of `window`; with `words`, it shows them as a notification.
    fn shell(message: u32, window: Handle, words: Option<&(String, String)>) {
        let mut data = NotifyIconData {
            size: size_of::<NotifyIconData>() as u32,
            window,
            id: 1,
            flags: ICON_FLAGS,
            callback: CALLBACK,
            icon: ICON.get().0 as Handle,
            tip: [0; 128],
            state: 0,
            state_mask: 0,
            info: [0; 256],
            version: 0,
            info_title: [0; 64],
            info_flags: 0,
            guid: [0; 4],
            balloon_icon: std::ptr::null_mut(),
        };
        fill(&mut data.tip, "Adelite");
        if let Some((title, body)) = words {
            data.flags |= NIF_INFO;
            data.info_flags = INFO_FLAGS;
            fill(&mut data.info_title, title);
            // A notification without words is not shown at all.
            fill(&mut data.info, if body.is_empty() { " " } else { body });
        }
        // SAFETY: `data` is a whole NOTIFYICONDATAW of the size it says, alive for the call; its
        // strings are terminated and its handles are the system's.
        unsafe { Shell_NotifyIconW(message, &raw const data) };
    }

    /// Puts the icon where it should be: in the tray while the window is
    /// hidden or a notification waits, out of it otherwise.
    fn settle(window: Handle) {
        let wanted = KEPT.get() || NOTED.get();
        if wanted != ADDED.replace(wanted) {
            shell(if wanted { NIM_ADD } else { NIM_DELETE }, window, None);
        }
    }

    /// Shows the notification that waits in the shared words.
    fn note(window: Handle) {
        let waiting = WORDS.with(|words| words.get().and_then(|words| words.lock().ok()?.take()));
        let Some(words) = waiting else { return };
        NOTED.set(true);
        let message = if ADDED.replace(true) { NIM_MODIFY } else { NIM_ADD };
        shell(message, window, Some(&words));
    }

    /// The icon's menu: open or quit.
    fn menu(window: Handle) {
        // SAFETY: takes nothing; the menu is ours until destroyed below.
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return;
        }
        let (open, quit) = (wide("Open Adelite"), wide("Quit Adelite"));
        // SAFETY: our menu, and a NUL-terminated string that outlives the call.
        unsafe { AppendMenuW(menu, MF_STRING, OPEN, open.as_ptr()) };
        // SAFETY: our menu; a separator has no text.
        unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null()) };
        // SAFETY: our menu, and a NUL-terminated string that outlives the call.
        unsafe { AppendMenuW(menu, MF_STRING, QUIT, quit.as_ptr()) };
        let mut point = Point::default();
        // SAFETY: `point` is valid for the write.
        unsafe { GetCursorPos(&raw mut point) };
        // Without this and the empty message below, the menu would not close when clicked beside.
        // SAFETY: our own window.
        unsafe { SetForegroundWindow(window) };
        // SAFETY: our menu and window; no rectangle to keep clear of.
        let chosen = unsafe { TrackPopupMenu(menu, MENU_FLAGS, point.x, point.y, 0, window, std::ptr::null()) };
        // SAFETY: a post to our own window.
        unsafe { PostMessageW(window, WM_NULL, 0, 0) };
        // SAFETY: the menu made above, shown no longer.
        unsafe { DestroyMenu(menu) };
        match chosen as usize {
            OPEN => emit(TrayEvent::Open),
            QUIT => emit(TrayEvent::Quit),
            _ => {}
        }
    }

    /// The hidden window's procedure: Windows calls it on the tray's thread.
    unsafe extern "system" fn procedure(window: Handle, message: u32, wparam: usize, lparam: isize) -> isize {
        match message {
            CALLBACK => match lparam as u32 & 0xFFFF {
                WM_LBUTTONUP | NOTIFICATION_CLICKED => emit(TrayEvent::Open),
                WM_RBUTTONUP | WM_CONTEXTMENU => menu(window),
                _ => {}
            },
            SHOW | HIDE => {
                KEPT.set(message == SHOW);
                settle(window);
            }
            NOTIFY => note(window),
            CLEAR => {
                NOTED.set(false);
                // Taking the icon out takes its notification with it; an icon that stays is added afresh.
                if ADDED.replace(false) {
                    shell(NIM_DELETE, window, None);
                }
                settle(window);
            }
            WAKE => emit(TrayEvent::Open),
            WM_CLOSE => {
                if ADDED.replace(false) {
                    shell(NIM_DELETE, window, None);
                }
                // SAFETY: our own window, on the thread that made it.
                unsafe { DestroyWindow(window) };
            }
            // SAFETY: takes an exit code and no pointers; it ends this thread's loop.
            WM_DESTROY => unsafe { PostQuitMessage(0) },
            // Explorer restarted and forgot every icon: ours goes back.
            restarted if restarted != 0 && restarted == TASKBAR_CREATED.get() => {
                if ADDED.get() {
                    shell(NIM_ADD, window, None);
                }
            }
            // SAFETY: the arguments are the ones Windows called this procedure with.
            _ => return unsafe { DefWindowProcW(window, message, wparam, lparam) },
        }
        0
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_icon_data_is_laid_out_as_windows_has_it() {
            assert_eq!(size_of::<NotifyIconData>(), 976);
            assert_eq!(std::mem::offset_of!(NotifyIconData, info), 304);
            assert_eq!(std::mem::offset_of!(NotifyIconData, balloon_icon), 968);
            let mut short = [0_u16; 4];
            fill(&mut short, "a\0bcdef");
            assert_eq!(short, [u16::from(b'a'), u16::from(b'b'), u16::from(b'c'), 0], "cut, with its terminator");
        }

        #[test]
        fn a_second_start_wakes_the_running_tray() {
            let (sent, events) = mpsc::channel();
            let tray = Tray::start(move |event| {
                let _ = sent.send(event);
            })
            .expect("the tray starts");
            assert!(wake_running(), "the hidden window is found");
            let event = events.recv_timeout(std::time::Duration::from_secs(5));
            assert_eq!(event, Ok(TrayEvent::Open));
            // Nothing was shown: dropping it leaves no icon behind and ends its thread.
            drop(tray);
            assert!(!wake_running(), "its window went with it");
        }
    }
}
