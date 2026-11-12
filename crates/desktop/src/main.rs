//! Adelite: a native, GPU-rendered desktop app for the day's work.
//!
//! The window is a shell (`shell`): a title bar, a rail of apps along the
//! right edge, and the app that is selected. The apps are crates of their
//! own behind [`adelite_ui::Module`], named in one place (`modules`); chat
//! is the first. No web view and no UI framework: `adelite-ui` draws every
//! pixel with wgpu, only when something changes.

// Release builds are GUI apps on Windows: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod close;
mod config;
mod modules;
mod shell;
#[cfg(test)]
mod snapshot;

use std::fs::File;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use adelite_ui::{log, platform};
use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::WindowId;

use app::{App, UserEvent};

/// Bridges winit callbacks to the [`App`], which can only be created once
/// the event loop is running.
struct Handler {
    app: Option<App>,
    proxy: EventLoopProxy<UserEvent>,
    error: Option<app::StartupError>,
}

impl ApplicationHandler<UserEvent> for Handler {
    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if let (StartCause::ResumeTimeReached { .. }, Some(app)) = (cause, &mut self.app) {
            app.wake();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_some() {
            return;
        }
        match App::new(event_loop, self.proxy.clone()) {
            Ok(app) => self.app = Some(app),
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        if let Some(app) = &mut self.app {
            app.user_event(event_loop, event);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if let Some(app) = &mut self.app {
            app.window_event(event_loop, event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(app) = &self.app {
            event_loop.set_control_flow(app.control_flow());
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        // On macOS, quitting from the menu (Cmd+Q) exits the process right
        // after this, so `run_app` never returns: drop the app here, which
        // lets its apps finish what they are writing.
        let update = self.app.as_mut().and_then(App::take_update);
        self.app = None;
        // Only now, with everything let go of, may the update replace the app.
        if let Some(update) = update
            && let Err(e) = update.apply()
        {
            log::error(format!("updating: {e} {}", e.detail()));
            platform::alert("Adelite could not update", &format!("{e} Adelite stays as it was."));
        }
    }
}

/// Tells the user why the app cannot run, in a dialog and the log.
fn fail(message: &str) -> ExitCode {
    log::error(message);
    platform::alert("Adelite cannot start", message);
    ExitCode::FAILURE
}

/// Takes the lock that keeps a second copy away: two would open the same
/// databases. `Ok(None)` when another copy holds it. The lock is held for
/// as long as the file is. A copy started by an update waits for the one it
/// replaces to finish quitting.
fn lock_instance() -> std::io::Result<Option<File>> {
    let dir = platform::data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let file = File::options().create(true).truncate(false).write(true).open(dir.join("desktop.lock"))?;
    let patience = if std::env::args().any(|arg| arg == adelite_update::AFTER_UPDATE) { 30 } else { 0 };
    let until = Instant::now() + Duration::from_secs(patience);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
    }
}

fn main() -> ExitCode {
    log::init();
    let _lock = match lock_instance() {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => {
            // Perhaps it only waits in the tray: it shows its window instead.
            if platform::tray::wake_running() {
                return ExitCode::SUCCESS;
            }
            return fail("Adelite is already running. Switch to its window, or close it before starting it again.");
        }
        Err(e) => {
            log::error(format!("cannot take the instance lock, running without it: {e}"));
            None
        }
    };
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(e) => return fail(&format!("The window system could not be started: {e}")),
    };
    let mut handler = Handler { app: None, proxy: event_loop.create_proxy(), error: None };
    let result = event_loop.run_app(&mut handler);
    if let Err(e) = result {
        return fail(&format!("The window system stopped unexpectedly: {e}"));
    }
    if let Some(e) = handler.error {
        return fail(&format!("Adelite could not open its window: {e}. Updating your graphics driver may help."));
    }
    ExitCode::SUCCESS
}
