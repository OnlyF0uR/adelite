//! The window, its events and the frame.
//!
//! Threading model: the winit thread owns all state and draws. Apps do
//! their work on threads of their own and wake this one ([`UserEvent::Wake`]);
//! thumbnails are decoded on a worker. The event loop sleeps until
//! something happens: an idle window draws nothing.
//!
//! Closing the window does what the settings say ([`CloseAction`]): by
//! default the app keeps running out of sight, where the platform has a
//! way back to it (the tray icon on Windows, the Dock icon on macOS).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use adelite_ui::gpu::{Frame, GpuError, Instance, Renderer, block_on};
use adelite_ui::image::{self, ImageAtlas, ImageKey};
use adelite_ui::paint::{Painter, Rect};
use adelite_ui::platform::sound::Sounds;
use adelite_ui::platform::tray::{Tray, TrayEvent};
use adelite_ui::text::{Fonts, GlyphAtlas};
use adelite_ui::theme::{Palette, Scheme};
use adelite_ui::titlebar::{self, Command, Grab};
use adelite_ui::ui::Ui;
use adelite_ui::{Context, Request, Waker, log, platform};
use arboard::Clipboard;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::Key;
use winit::window::{CursorIcon, Theme, UserAttentionType, Window};

use crate::close::CloseAction;
use crate::config::{Config, ZOOM};
use crate::modules;
use crate::shell::Shell;

/// Length of the cross-fade when the colour scheme changes.
const THEME_FADE_SECS: f32 = 0.25;
/// Window size on first start, in logical pixels.
const WINDOW_SIZE: (u32, u32) = (1440, 900);
/// Smallest window size, in logical pixels.
const MIN_WINDOW_SIZE: (u32, u32) = (860, 560);
/// How far one press of Ctrl/Cmd + or - zooms, in percent.
const ZOOM_STEP: u16 = 10;
/// Wait between attempts to bring the GPU back after it was lost.
const GPU_RETRY: Duration = Duration::from_secs(2);

/// What other threads tell the event loop.
pub enum UserEvent {
    /// An app has something for the interface.
    Wake,
    /// A thumbnail was decoded (or could not be).
    Thumbnail(ImageKey, Result<Vec<u8>, String>),
    /// The tray icon was clicked, or another start of the app asks for the window.
    Tray(TrayEvent),
}

/// Failure to start the app.
#[derive(Debug)]
pub enum StartupError {
    /// The OS refused to create a window.
    Window(winit::error::OsError),
    /// GPU initialisation failed.
    Gpu(GpuError),
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Window(e) => write!(f, "cannot create the window: {e}"),
            Self::Gpu(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for StartupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Window(e) => Some(e),
            Self::Gpu(e) => Some(e),
        }
    }
}

/// The running application. Its fields drop in this order: the window
/// goes first, the apps last, so closing is instant to the eye while an
/// app (Telegram's library) still writes what it holds.
pub struct App {
    window: Arc<Window>,
    /// For creating the renderer again after the GPU was lost.
    display: winit::event_loop::OwnedDisplayHandle,
    renderer: Renderer,
    /// When to try again to bring back a lost GPU.
    gpu_retry: Option<Instant>,
    fonts: Fonts,
    atlas: GlyphAtlas,
    images: ImageAtlas,
    instances: Vec<Instance>,
    proxy: EventLoopProxy<UserEvent>,
    clipboard: Option<Clipboard>,
    ui: Ui,
    start: Instant,
    last_frame: Instant,
    config: Config,
    scheme: Scheme,
    /// Colours drawn this frame; differs from `scheme` during a cross-fade.
    palette: Palette,
    /// Palette a cross-fade started from, and its progress in `0..=1`.
    fade: Option<(Palette, f32)>,
    /// Whether the window has been made visible (after the first frame).
    shown: bool,
    /// Cursor currently set on the window.
    cursor: CursorIcon,
    /// Input-method candidate area last sent to the window.
    ime_area: Option<Rect>,
    /// The title bar's close button was clicked: the window closes as the
    /// settings say once the frame is drawn.
    closing: bool,
    /// When the title bar strip was last pressed, to maximize on a second press.
    title_press: Option<f32>,
    /// A wake is on its way to the event loop: no need for another.
    waking: Arc<AtomicBool>,
    /// Says what arrived while the window was in the background, where the
    /// tray does not.
    notifier: platform::Notifier,
    /// The tray icon that brings a hidden window back and says what arrived (Windows only).
    tray: Option<Tray>,
    /// The window is hidden, its icon in the tray.
    hidden: bool,
    /// The app's sounds.
    sounds: Sounds,
    shell: Shell,
}

impl App {
    /// Creates the window and GPU state, reads the settings and starts the apps.
    ///
    /// # Errors
    /// See [`StartupError`].
    pub fn new(event_loop: &ActiveEventLoop, proxy: EventLoopProxy<UserEvent>) -> Result<Self, StartupError> {
        let dir = platform::data_dir().inspect_err(|e| log::error(format!("nothing will be saved: {e}"))).ok();
        let config = Config::load(dir.as_deref());
        let scheme = Scheme::from_key(Some(config.theme.as_str()));
        let ((width, height), maximized) = window_state(&config.window);
        let attributes = Window::default_attributes()
            .with_title("Adelite")
            .with_inner_size(LogicalSize::new(width, height))
            .with_min_inner_size(LogicalSize::new(MIN_WINDOW_SIZE.0, MIN_WINDOW_SIZE.1))
            // Windows shows a window as it maximizes it, so there it waits
            // for the first frame (see `frame`).
            .with_maximized(maximized && !cfg!(windows))
            .with_theme(Some(window_theme(scheme)))
            // Shown after the first frame so the user never sees a blank window.
            .with_visible(false)
            // The app draws its own title bar (`titlebar`), in the scheme's colours.
            .with_decorations(!titlebar::CUSTOM);
        // Undecorated, a window still casts the system's shadow.
        #[cfg(windows)]
        let attributes =
            winit::platform::windows::WindowAttributesExtWindows::with_undecorated_shadow(attributes, true);
        // Let the page run under a transparent title bar on macOS.
        #[cfg(target_os = "macos")]
        let attributes = {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attributes.with_titlebar_transparent(true).with_fullsize_content_view(true).with_title_hidden(true)
        };
        // Wayland's app id and X11's WM_CLASS: desktops match it to the
        // installed `adelite.desktop` for the name and icon.
        #[cfg(target_os = "linux")]
        let attributes =
            winit::platform::wayland::WindowAttributesExtWayland::with_name(attributes, "adelite", "adelite");
        let window = Arc::new(event_loop.create_window(attributes).map_err(StartupError::Window)?);
        set_icon(&window);
        // Chinese, Japanese and Korean input methods deliver text through IME events.
        window.set_ime_allowed(true);
        let display = event_loop.owned_display_handle();
        let renderer = block_on(Renderer::new(Arc::clone(&window), display.clone())).map_err(StartupError::Gpu)?;

        // Apps wake the loop from their threads; many wakes before it runs are one.
        let waking = Arc::new(AtomicBool::new(false));
        let (flag, sender) = (Arc::clone(&waking), proxy.clone());
        let waker: Waker = Arc::new(move || {
            if !flag.swap(true, Ordering::AcqRel) {
                // Fails only if the event loop is gone, i.e. the app is exiting.
                let _ = sender.send_event(UserEvent::Wake);
            }
        });
        // Without a home directory the apps keep their data where it is at least out of the way.
        let data = dir.unwrap_or_else(|| std::env::temp_dir().join("adelite"));
        let mut shell = Shell::new(modules::start(&data, &waker));
        (shell.notify, shell.sounds) = (config.notify, config.sounds);
        shell.close = CloseAction::from_key(&config.close);
        // The tray's clicks arrive on its own thread and are handed to this one.
        let clicks = proxy.clone();
        let tray = Tray::start(move |event| {
            let _ = clicks.send_event(UserEvent::Tray(event));
        });
        let tray = match tray {
            Ok(tray) => Some(tray),
            Err(e) => {
                if cfg!(windows) {
                    log::error(format!("there will be no tray icon: {e}"));
                }
                None
            }
        };

        let now = Instant::now();
        let mut app = Self {
            window,
            display,
            renderer,
            gpu_retry: None,
            fonts: Fonts::load(),
            atlas: GlyphAtlas::default(),
            images: ImageAtlas::default(),
            instances: Vec::new(),
            proxy,
            clipboard: None,
            ui: Ui::default(),
            start: now,
            last_frame: now,
            config,
            scheme,
            palette: *scheme.palette(),
            fade: None,
            shown: false,
            cursor: CursorIcon::Default,
            ime_area: None,
            closing: false,
            title_press: None,
            waking,
            notifier: platform::Notifier,
            tray,
            hidden: false,
            sounds: Sounds::default(),
            shell,
        };
        app.ui.focused = true;
        // Draw the first frame directly: hidden windows never receive
        // `RedrawRequested` on Windows, so waiting for one would keep the
        // window invisible forever. `frame` shows it once drawn.
        app.frame();
        Ok(app)
    }

    /// Starts a cross-fade to `scheme` and keeps it for the next start.
    fn set_scheme(&mut self, scheme: Scheme) {
        if scheme == self.scheme {
            return;
        }
        self.fade = Some((self.palette, 0.0));
        self.scheme = scheme;
        self.window.set_theme(Some(window_theme(scheme)));
        scheme.key().clone_into(&mut self.config.theme);
        self.config.save();
    }

    /// Physical pixels per logical pixel: the display's scale, zoomed.
    fn scale(&self) -> f32 {
        self.window.scale_factor() as f32 * f32::from(self.config.zoom) / 100.0
    }

    /// The window's area in logical pixels.
    fn view(&self) -> Rect {
        let (scale, size) = (self.scale(), self.window.inner_size());
        Rect::new(0.0, 0.0, size.width as f32 / scale, size.height as f32 / scale)
    }

    /// Carries out what the apps asked of the shell.
    fn serve(&mut self, requests: Vec<Request>) {
        for request in requests {
            match request {
                Request::Attention if !self.ui.focused => {
                    self.window.request_user_attention(Some(UserAttentionType::Informational));
                }
                Request::Notice(text) => self.shell.tell(text, true),
                Request::Info(text) => self.shell.tell(text, false),
                Request::Notify { title, body } if !self.ui.focused && self.config.notify => match &self.tray {
                    Some(tray) => tray.notify(&title, &body),
                    None => self.notifier.show(&title, &body),
                },
                // Heard, unless it happened where the user is looking.
                Request::Sound { sound, in_view } if self.config.sounds && !(in_view && self.ui.focused) => {
                    self.sounds.play(sound);
                }
                // The user is here and sees it arrive: nothing to say outside the window.
                Request::Attention | Request::Notify { .. } | Request::Sound { .. } => {}
            }
        }
    }

    /// Routes a window event.
    pub fn window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        let scale = self.scale();
        let mut requests = Vec::new();
        match event {
            WindowEvent::CloseRequested => return self.close(event_loop),
            WindowEvent::RedrawRequested => {
                self.frame();
                if std::mem::take(&mut self.closing) {
                    self.close(event_loop);
                }
                return;
            }
            WindowEvent::Resized(size) => self.renderer.resize(size.width, size.height),
            WindowEvent::Focused(focused) => {
                self.ui.focused = focused;
                if let (true, Some(tray)) = (focused, &self.tray) {
                    // Here again: what was said outside the window has been seen.
                    tray.clear();
                }
            }
            WindowEvent::DroppedFile(path) => self.shell.files_dropped(vec![path]),
            WindowEvent::ModifiersChanged(mods) => self.ui.mods = mods.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.ui.mouse = (position.x as f32 / scale, position.y as f32 / scale);
            }
            WindowEvent::CursorLeft { .. } => self.ui.mouse = (f32::MIN, f32::MIN),
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                if state == ElementState::Pressed {
                    let now = self.start.elapsed().as_secs_f32();
                    // The title bar and the window's edges take the press
                    // themselves; the page never sees it.
                    match titlebar::grab(self.view(), self.ui.mouse, self.window.is_maximized()) {
                        Some(Grab::Resize(direction)) => {
                            let _ = self.window.drag_resize_window(direction);
                        }
                        Some(Grab::Move) if self.title_press.is_some_and(|at| now - at < 0.4) => {
                            self.title_press = None;
                            self.window.set_maximized(!self.window.is_maximized());
                        }
                        Some(Grab::Move) => {
                            self.title_press = Some(now);
                            let _ = self.window.drag_window();
                        }
                        None => self.ui.press(now),
                    }
                } else {
                    self.ui.down = false;
                    self.ui.released = true;
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => {
                // The other button asks what is under the mouse for its menu; the title bar has none.
                if titlebar::grab(self.view(), self.ui.mouse, self.window.is_maximized()).is_none() {
                    self.ui.context = true;
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * 100.0, y * 100.0),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32 / scale, p.y as f32 / scale),
                };
                // Shift turns a plain wheel sideways, as everywhere else.
                let (dx, dy) = if self.ui.mods.shift_key() && dx == 0.0 { (dy, 0.0) } else { (dx, dy) };
                self.ui.scroll -= dy;
                self.ui.scroll_x -= dx;
            }
            WindowEvent::KeyboardInput { event, is_synthetic: false, .. } if event.state == ElementState::Pressed => {
                let primary =
                    if cfg!(target_os = "macos") { self.ui.mods.super_key() } else { self.ui.mods.control_key() };
                if primary && let Some(zoom) = zoomed(self.config.zoom, &event.logical_key) {
                    if zoom != self.config.zoom {
                        self.config.zoom = zoom;
                        // Resent next frame at the new zoom.
                        self.ime_area = None;
                        self.config.save();
                    }
                    self.window.request_redraw();
                    return;
                }
                // Ctrl+Q quits whatever closing the window does; macOS has Cmd+Q in its menu.
                if primary
                    && !cfg!(target_os = "macos")
                    && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("q"))
                {
                    return event_loop.exit();
                }
                let mut cx = Context { clipboard: &mut self.clipboard, requests: &mut requests };
                self.shell.key(&event, self.ui.mods, &mut cx);
                self.ui.last_edit = self.ui.time;
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.shell.ime_commit(&text);
                self.ui.last_edit = self.ui.time;
            }
            _ => return,
        }
        self.serve(requests);
        self.window.request_redraw();
    }

    /// Closes the window as the settings say: out of sight with the app
    /// still running, minimised, or by quitting.
    fn close(&mut self, event_loop: &ActiveEventLoop) {
        match self.shell.close {
            CloseAction::Quit => event_loop.exit(),
            CloseAction::Minimize => self.window.set_minimized(true),
            CloseAction::Background => self.go_to_background(event_loop),
        }
    }

    /// Takes the window off the screen while the app keeps running.
    #[cfg_attr(target_os = "macos", expect(clippy::unused_self, reason = "macOS hides the whole app"))]
    fn go_to_background(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::ActiveEventLoopExtMacOS;
            event_loop.hide_application();
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = event_loop;
            match &self.tray {
                Some(tray) => {
                    tray.show();
                    self.window.set_visible(false);
                    // Out of sight is out of focus, whether or not the system says so: what
                    // arrives from now on is said outside the window.
                    (self.hidden, self.ui.focused) = (true, false);
                }
                // Without a tray icon a hidden window could not be found again.
                None => self.window.set_minimized(true),
            }
        }
    }

    /// Brings the window back to the front, from the tray or minimised.
    fn restore(&mut self) {
        if std::mem::take(&mut self.hidden) {
            self.window.set_visible(true);
        }
        if let Some(tray) = &self.tray {
            tray.hide();
            tray.clear();
        }
        if self.window.is_minimized() == Some(true) {
            self.window.set_minimized(false);
        }
        self.window.focus_window();
    }

    /// Something arrived from another thread.
    pub fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Tray(TrayEvent::Open) => self.restore(),
            UserEvent::Tray(TrayEvent::Quit) => return event_loop.exit(),
            UserEvent::Thumbnail(key, pixels) => self.images.insert(key, pixels),
            UserEvent::Wake => {
                // Cleared first: what arrives while the apps are asked wakes the loop again.
                self.waking.store(false, Ordering::Release);
                let mut requests = Vec::new();
                self.shell.poll(&mut Context { clipboard: &mut self.clipboard, requests: &mut requests });
                self.serve(requests);
            }
        }
        self.window.request_redraw();
    }

    /// Creates the renderer again after the GPU was lost (a driver reset or
    /// update, a GPU switch), with fresh atlases: theirs lived on the old
    /// device. Tries again every [`GPU_RETRY`] while it fails.
    fn recover_gpu(&mut self) {
        if self.gpu_retry.is_some_and(|at| at > Instant::now()) {
            return;
        }
        match block_on(Renderer::new(Arc::clone(&self.window), self.display.clone())) {
            Ok(renderer) => {
                self.renderer = renderer;
                self.atlas.clear();
                self.images = ImageAtlas::default();
                self.gpu_retry = None;
                let size = self.window.inner_size();
                self.renderer.resize(size.width, size.height);
            }
            Err(e) => {
                log::error(format!("cannot restore the GPU: {e}"));
                self.gpu_retry = Some(Instant::now() + GPU_RETRY);
            }
        }
    }

    /// Builds and presents one frame.
    fn frame(&mut self) {
        if self.renderer.is_lost() {
            self.recover_gpu();
        }
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        self.ui.begin(dt, (now - self.start).as_secs_f32());

        // Cross-fade between colour schemes with a smoothstep ease.
        let target = self.scheme.palette();
        self.palette = match &mut self.fade {
            Some((from, progress)) if *progress < 1.0 => {
                *progress = (*progress + dt / THEME_FADE_SECS).min(1.0);
                let eased = *progress * *progress * (3.0 - 2.0 * *progress);
                self.ui.animating = true;
                from.mix(target, eased)
            }
            _ => {
                self.fade = None;
                *target
            }
        };

        let scale = self.scale();
        let view = self.view();
        let maximized = self.window.is_maximized();
        let mut requests = Vec::new();
        let (mut command, mut chosen) = (None, None);
        // A second pass runs only if the glyph atlas overflowed mid-frame.
        for _ in 0..2 {
            let mut painter = Painter::new(
                &self.fonts,
                (&mut self.atlas, &mut self.images),
                &mut self.instances,
                scale,
                view,
                &self.palette,
            );
            let mut cx = Context { clipboard: &mut self.clipboard, requests: &mut requests };
            chosen = chosen.or(self.shell.draw(&mut painter, &mut self.ui, view, self.scheme, &mut cx));
            command = command.or(titlebar::draw(&mut painter, &mut self.ui, view, maximized));
            if !painter.atlas_full {
                break;
            }
            // The first pass already acted on this frame's input; the redraw
            // must not act on it twice.
            self.atlas.clear();
            self.ui.end();
        }

        match command {
            Some(Command::Minimize) => self.window.set_minimized(true),
            Some(Command::Maximize) => self.window.set_maximized(!maximized),
            Some(Command::Close) => self.closing = true,
            None => {}
        }
        let drawn = self.renderer.render(
            &self.instances,
            &mut self.atlas.uploads,
            &mut self.images.uploads,
            scale,
            self.palette.panel,
        );
        self.ui.end();
        if drawn == Frame::Retry {
            // The surface was set up again: draw this frame onto it.
            self.window.request_redraw();
        }
        if !self.shown {
            self.shown = true;
            // Windows shows a window as it maximizes it: only now, drawn.
            if cfg!(windows) && window_state(&self.config.window).1 {
                self.window.set_maximized(true);
            }
            self.window.set_visible(true);
            // Some platforms refuse to present to a hidden surface; draw
            // again now that the window is visible.
            self.window.request_redraw();
        }
        if self.cursor != self.ui.cursor {
            self.cursor = self.ui.cursor;
            self.window.set_cursor(self.cursor);
        }
        // Keep the input method's candidate window next to the caret.
        let area = self.shell.ime_area();
        if area != self.ime_area {
            self.ime_area = area;
            if let Some(r) = area {
                let z = f32::from(self.config.zoom) / 100.0;
                self.window
                    .set_ime_cursor_area(LogicalPosition::new(r.x * z, r.y * z), LogicalSize::new(r.w * z, r.h * z));
            }
        }
        if let Some(scheme) = chosen {
            self.set_scheme(scheme);
        }
        // What the settings changed is kept for the next start.
        let closes = CloseAction::from_key(&self.config.close);
        if (self.shell.notify, self.shell.sounds, self.shell.close) != (self.config.notify, self.config.sounds, closes)
        {
            (self.config.notify, self.config.sounds) = (self.shell.notify, self.shell.sounds);
            self.shell.close.key().clone_into(&mut self.config.close);
            self.config.save();
        }
        self.serve(requests);
        self.decode_wanted();
        if self.ui.animating || self.shell.is_busy() {
            self.window.request_redraw();
        }
    }

    /// Decodes the pictures this frame missed: one worker per frame's
    /// misses, taking them in turn, so a page of pictures never decodes
    /// dozens at once.
    fn decode_wanted(&mut self) {
        let wanted = self.images.take_wanted();
        if wanted.is_empty() {
            return;
        }
        let proxy = self.proxy.clone();
        let spawned = std::thread::Builder::new().name("adelite-thumbnails".into()).spawn(move || {
            for key in wanted {
                let pixels = image::thumbnail(&key);
                // Fails only if the event loop is gone, i.e. the app is exiting.
                if proxy.send_event(UserEvent::Thumbnail(key, pixels)).is_err() {
                    return;
                }
            }
        });
        if let Err(e) = spawned {
            log::error(format!("cannot start the thumbnail thread: {e}"));
        }
    }

    /// How long the event loop may sleep: until the caret blinks next, a
    /// notice goes, or the GPU is tried again.
    pub fn control_flow(&self) -> ControlFlow {
        let blink = self.ui.focused.then(|| self.last_frame + Duration::from_secs_f32(self.ui.next_blink().max(0.01)));
        match blink.into_iter().chain(self.gpu_retry).chain(self.shell.next_wake()).min() {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        }
    }

    /// A timer fired: redraws for the caret.
    pub fn wake(&mut self) {
        self.window.request_redraw();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // The next start opens the window as it was closed. A minimized
        // window tells nothing, and a maximized one keeps its normal size.
        if self.window.is_minimized() == Some(true) {
            return;
        }
        // macOS's green button makes it fullscreen: as good as maximized.
        let maximized = self.window.is_maximized() || self.window.fullscreen().is_some();
        let (mut size, _) = window_state(&self.config.window);
        if !maximized {
            let logical = self.window.inner_size().to_logical::<f64>(self.window.scale_factor());
            size = (logical.width.round() as u32, logical.height.round() as u32);
        }
        let state = format!("{}x{}{}", size.0, size.1, if maximized { " maximized" } else { "" });
        if self.config.window != state {
            self.config.window = state;
            self.config.save();
        }
        // Gone from the screen before the apps are let go, which may take a moment.
        self.window.set_visible(false);
    }
}

/// Makes the OS draw what chrome it still draws to match `scheme`.
fn window_theme(scheme: Scheme) -> Theme {
    if scheme.is_light() { Theme::Light } else { Theme::Dark }
}

/// The app's icon on its window and in the taskbar.
fn set_icon(window: &Window) {
    #[cfg(windows)]
    {
        use winit::platform::windows::{IconExtWindows, WindowExtWindows};
        let scale = window.scale_factor();
        let load = |side: f64| {
            let side = (side * scale).round() as u32;
            winit::window::Icon::from_resource(1, Some(winit::dpi::PhysicalSize::new(side, side))).ok()
        };
        window.set_window_icon(load(16.0));
        window.set_taskbar_icon(load(32.0));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Ok((rgba, w, h)) = image::decode_png(include_bytes!("../assets/icon/256.png"))
        && let Ok(icon) = winit::window::Icon::from_rgba(rgba, w, h)
    {
        window.set_window_icon(Some(icon));
    }
    #[cfg(target_os = "macos")]
    let _ = window;
}

/// The zoom after the primary modifier plus `key`, or `None` if it is not
/// a zoom key: `+` (or `=`, unshifted on many layouts) in, `-` out, `0` back
/// to 100%.
fn zoomed(zoom: u16, key: &Key) -> Option<u16> {
    let Key::Character(c) = key else { return None };
    let zoom = match c.as_str() {
        "+" | "=" => zoom.saturating_add(ZOOM_STEP),
        "-" => zoom.saturating_sub(ZOOM_STEP),
        "0" => 100,
        _ => return None,
    };
    Some(zoom.clamp(ZOOM.0, ZOOM.1))
}

/// Logical size and maximized state of the window from the settings'
/// `window` (`1200x800`, `1200x800 maximized`); the first-start size when
/// absent or unreadable.
fn window_state(saved: &str) -> ((u32, u32), bool) {
    let (size, state) = saved.split_once(' ').unwrap_or((saved, ""));
    let size = size.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
    let (w, h) = size.unwrap_or(WINDOW_SIZE);
    // Never smaller than allowed, nor absurdly large from an edited file.
    ((w.clamp(MIN_WINDOW_SIZE.0, 16_384), h.clamp(MIN_WINDOW_SIZE.1, 16_384)), state == "maximized")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_comes_back_as_it_was_left() {
        assert_eq!(window_state(""), (WINDOW_SIZE, false));
        assert_eq!(window_state("1200x800"), ((1200, 800), false));
        assert_eq!(window_state("1200x800 maximized"), ((1200, 800), true));
        assert_eq!(window_state("10x10"), (MIN_WINDOW_SIZE, false), "never smaller than allowed");
        assert_eq!(window_state("99999999x1 what"), ((16_384, MIN_WINDOW_SIZE.1), false));
        assert_eq!(window_state("wide"), (WINDOW_SIZE, false));
    }

    #[test]
    fn zoom_keys_step_and_stop_at_the_limits() {
        let key = |c: &str| Key::Character(c.into());
        assert_eq!(zoomed(100, &key("+")), Some(110));
        assert_eq!(zoomed(100, &key("=")), Some(110));
        assert_eq!(zoomed(ZOOM.1, &key("+")), Some(ZOOM.1));
        assert_eq!(zoomed(ZOOM.0, &key("-")), Some(ZOOM.0));
        assert_eq!(zoomed(170, &key("0")), Some(100));
        assert_eq!(zoomed(100, &key("a")), None);
    }
}
