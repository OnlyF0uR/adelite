//! What is drawn around the apps: the brand in the title bar, the rail of
//! apps along the window's right edge, the settings, and notices. The app
//! on the rail that is selected fills the rest.

use std::time::{Duration, Instant};

use adelite_ui::logo::Logo;
use adelite_ui::paint::{Painter, Rect, fade, mix};
use adelite_ui::text::Style;
use adelite_ui::theme::{self, Scheme};
use adelite_ui::ui::{Ui, icon, icon_button, id};
use adelite_ui::widgets::{self, MenuItem, chip, dropdown, field_label, modal, toggle};
use adelite_ui::{Context, Module, titlebar};
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::CursorIcon;

use crate::close::CloseAction;

/// Side of an app's button on the rail.
const BUTTON: f32 = 38.0;
/// How long a notice stays.
const NOTICE: Duration = Duration::from_secs(7);

/// The apps and what the shell shows around them.
pub struct Shell {
    /// The apps, in the rail's order.
    modules: Vec<Box<dyn Module>>,
    /// The app that is shown.
    active: usize,
    /// The settings are open.
    settings: bool,
    /// What the user is being told, until when, and whether it is a problem.
    notice: Option<(String, Instant, bool)>,
    /// Whether what arrives in the background is said outside the window; the settings switch it.
    pub notify: bool,
    /// Whether a message that arrives or leaves is heard; the settings switch it.
    pub sounds: bool,
    /// What closing the window does; the settings choose it.
    pub close: CloseAction,
    /// The list of what closing the window can do is open.
    choosing: bool,
}

impl Shell {
    /// A shell around `modules`, the first one shown.
    pub fn new(modules: Vec<Box<dyn Module>>) -> Self {
        Self {
            modules,
            active: 0,
            settings: false,
            notice: None,
            notify: true,
            sounds: true,
            close: CloseAction::offered()[0],
            choosing: false,
        }
    }

    /// The app that is shown, if there is any.
    fn module(&mut self) -> Option<&mut Box<dyn Module>> {
        self.modules.get_mut(self.active)
    }

    /// Lets every app take in what its threads left for it.
    pub fn poll(&mut self, cx: &mut Context<'_>) {
        for module in &mut self.modules {
            module.poll(cx);
        }
    }

    /// Opens the settings, for the snapshots.
    #[cfg(test)]
    pub fn open_settings(&mut self, choosing: bool) {
        (self.settings, self.choosing) = (true, choosing);
    }

    /// Tells the user `text` in a notice over the page; `problem`: something went wrong.
    pub fn tell(&mut self, text: String, problem: bool) {
        self.notice = Some((text, Instant::now() + NOTICE, problem));
    }

    /// When the shell next changes by itself: the notice goes.
    pub fn next_wake(&self) -> Option<Instant> {
        self.notice.as_ref().map(|(_, until, _)| *until)
    }

    /// Whether the shown app wants a frame after this one.
    pub fn is_busy(&self) -> bool {
        self.modules.get(self.active).is_some_and(|module| module.is_busy())
    }

    /// Where the shown app's caret is, for the input method's window.
    pub fn ime_area(&self) -> Option<Rect> {
        self.modules.get(self.active).filter(|_| !self.settings).and_then(|module| module.ime_area())
    }

    /// A key was pressed.
    pub fn key(&mut self, event: &KeyEvent, mods: ModifiersState, cx: &mut Context<'_>) {
        let primary = if cfg!(target_os = "macos") { mods.super_key() } else { mods.control_key() };
        match &event.logical_key {
            // Escape closes the nearest thing: the open list, then the settings.
            Key::Named(NamedKey::Escape) if self.settings => {
                self.settings = std::mem::take(&mut self.choosing);
            }
            _ if self.settings => {}
            // Ctrl/Cmd and a digit: that app on the rail.
            Key::Character(c) if primary && c.len() == 1 && c.as_bytes()[0].is_ascii_digit() && c != "0" => {
                let index = usize::from(c.as_bytes()[0] - b'1');
                if index < self.modules.len() {
                    self.active = index;
                }
            }
            _ => {
                if let Some(module) = self.module() {
                    module.key(event, mods, cx);
                }
            }
        }
    }

    /// An input method committed `text`.
    pub fn ime_commit(&mut self, text: &str) {
        if self.settings {
            return;
        }
        if let Some(module) = self.module() {
            module.ime_commit(text);
        }
    }

    /// Files were dropped on the window.
    pub fn files_dropped(&mut self, files: Vec<std::path::PathBuf>) {
        if let Some(module) = self.module() {
            module.files_dropped(files);
        }
    }

    /// Draws the window's contents. Returns the colour scheme the user chose, if they chose one.
    pub fn draw(
        &mut self,
        p: &mut Painter<'_>,
        ui: &mut Ui,
        view: Rect,
        scheme: Scheme,
        cx: &mut Context<'_>,
    ) -> Option<Scheme> {
        let t = p.theme;
        p.rect(view, t.panel, 0.0);
        let body = Rect::new(view.x, view.y + theme::TITLE_HEIGHT, view.w, view.h - theme::TITLE_HEIGHT);
        let rail = Rect::new(body.right() - theme::RAIL_WIDTH, body.y, theme::RAIL_WIDTH, body.h);
        let area = Rect::new(body.x, body.y, body.w - theme::RAIL_WIDTH, body.h);
        // The brand, where a title would be: on macOS the traffic lights sit there.
        if titlebar::CUSTOM {
            let mid = view.y + theme::TITLE_HEIGHT * 0.5 + 1.0;
            p.logo(Logo::App, (view.x + 24.0, mid), 20.0, t.text);
            p.label("Adelite", theme::LABEL, view.x + 40.0, mid - 9.0, t.text_muted);
        }
        // The settings take the mouse from the whole window.
        let was_open = self.settings;
        ui.blocker = was_open.then_some(view);
        if let Some(module) = self.modules.get_mut(self.active) {
            module.draw(p, ui, area, cx);
        }
        self.draw_rail(p, ui, rail);
        ui.blocker = None;
        // Opened by this frame's click, they must not take that same click for one outside them.
        let released = ui.released;
        ui.released = released && was_open;
        let chosen = if self.settings { self.draw_settings(p, ui, view, scheme) } else { None };
        ui.released = released;
        self.draw_notice(p, ui, area);
        chosen
    }

    /// The rail: an icon per app, the settings at its foot.
    fn draw_rail(&mut self, p: &mut Painter<'_>, ui: &mut Ui, rail: Rect) {
        let t = p.theme;
        let x = rail.x + (rail.w - BUTTON) * 0.5;
        let mut y = rail.y + 6.0;
        let mut named = None;
        for (index, module) in self.modules.iter().enumerate() {
            let button = Rect::new(x, y, BUTTON, BUTTON);
            y += BUTTON + 6.0;
            let selected = index == self.active;
            let hovered = ui.hovered(button);
            let hover = ui.anim(id(("rail", index)), f32::from(u8::from(hovered)));
            p.rect(button, if selected { t.active } else { fade(t.hover, hover) }, theme::RADIUS);
            let ink = if selected { t.text } else { mix(t.text_muted, t.text, hover) };
            icon::draw(p, module.icon(), (button.x + BUTTON * 0.5, button.y + BUTTON * 0.5), 18.0, ink);
            let waiting = module.badge();
            if waiting > 0 {
                let count = if waiting > 99 { "99+".to_owned() } else { waiting.to_string() };
                let count = p.layout(&count, Style::semibold(10.0), None);
                let width = (count.width() + 8.0).max(16.0);
                let pill = Rect::new(button.right() - width + 5.0, button.y - 3.0, width, 16.0);
                // A ring of the rail's own colour sets the count apart from the button.
                p.rect(Rect::new(pill.x - 2.0, pill.y - 2.0, pill.w + 4.0, pill.h + 4.0), t.panel, 10.0);
                p.rect(pill, t.accent, 8.0);
                p.text(
                    &count,
                    pill.x + (pill.w - count.width()) * 0.5,
                    pill.y + (pill.h - count.height()) * 0.5,
                    t.on_accent,
                );
            }
            if hovered {
                ui.cursor = CursorIcon::Pointer;
                named = Some((module.name(), button, hover));
                if ui.clicked(button) {
                    self.active = index;
                }
            }
        }
        let cog = Rect::new(x, rail.bottom() - theme::GAP - BUTTON, BUTTON, BUTTON);
        if ui.hovered(cog) {
            named = Some(("Settings", cog, 1.0));
        }
        if icon_button(p, ui, cog, icon::SETTINGS, true) {
            self.settings = true;
        }
        // What the icon under the mouse is, beside it.
        if let Some((name, button, appear)) = named {
            let name = p.layout(name, theme::CAPTION, None);
            let tip = Rect::new(button.x - name.width() - 28.0, button.y + 6.0, name.width() + 18.0, 26.0);
            p.shadow(Rect::new(tip.x, tip.y + 3.0, tip.w, tip.h), fade(t.shadow, 0.6 * appear), theme::RADIUS_SM, 12.0);
            p.bordered(tip, fade(t.surface, appear), theme::RADIUS_SM, 1.0, fade(t.border_strong, appear));
            p.text(&name, tip.x + 9.0, tip.y + (tip.h - name.height()) * 0.5, fade(t.text, appear));
        }
    }

    /// The settings: the colour scheme, notifications, sounds, and what closing the window does.
    /// Returns the scheme chosen.
    fn draw_settings(&mut self, p: &mut Painter<'_>, ui: &mut Ui, view: Rect, scheme: Scheme) -> Option<Scheme> {
        let t = p.theme;
        let (card, outside) = modal(p, ui, view, (440.0, 492.0), "settings");
        let x = card.x + 24.0;
        // Under the open list, the card does not answer the mouse.
        let was_choosing = self.choosing;
        let outer = ui.blocker;
        if was_choosing {
            ui.blocker = Some(view);
        }
        p.label("Settings", theme::TITLE, x, card.y + 20.0, t.text);
        let close = icon_button(p, ui, Rect::new(card.right() - 44.0, card.y + 18.0, 28.0, 28.0), icon::X, true);
        if (outside || close) && !was_choosing {
            self.settings = false;
        }
        field_label(p, icon::PALETTE, "Colour scheme", x, card.y + 74.0);
        let mut chosen = None;
        for (index, option) in Scheme::ALL.into_iter().enumerate() {
            let rect = Rect::new(x + index as f32 * 96.0, card.y + 100.0, 88.0, 32.0);
            if chip(p, ui, rect, option.label(), option == scheme) {
                chosen = Some(option);
            }
        }
        let modifier = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
        let hint = format!("{modifier} + and {modifier} − zoom the interface, {modifier} 0 puts it back.");
        p.label(&hint, theme::TINY, x, card.y + 152.0, t.text_faint);
        field_label(p, icon::BELL, "Notifications", x, card.y + 192.0);
        if toggle(p, ui, (card.right() - 24.0 - 38.0, card.y + 189.0), self.notify, "notify") {
            self.notify = !self.notify;
        }
        let hint = "Says what arrives while Adelite is in the background.";
        p.label(hint, theme::TINY, x, card.y + 218.0, t.text_faint);
        field_label(p, icon::MUSIC, "Sounds", x, card.y + 258.0);
        if toggle(p, ui, (card.right() - 24.0 - 38.0, card.y + 255.0), self.sounds, "sounds") {
            self.sounds = !self.sounds;
        }
        p.label(
            "A soft sound when a message arrives, and when yours has left.",
            theme::TINY,
            x,
            card.y + 284.0,
            t.text_faint,
        );
        // What closing the window does: one of a few, in a list.
        field_label(p, icon::X, "When the window is closed", x, card.y + 324.0);
        let button = Rect::new(x, card.y + 350.0, card.w - 48.0, 36.0);
        if dropdown(p, ui, button, Some(self.close.glyph()), self.close.label(), was_choosing) {
            self.choosing = true;
        }
        p.label(self.close.detail(), theme::TINY, x, card.y + 396.0, t.text_faint);
        p.label(concat!("Adelite ", env!("CARGO_PKG_VERSION")), theme::TINY, x, card.bottom() - 36.0, t.text_faint);
        ui.blocker = outer;
        if was_choosing {
            let offered = CloseAction::offered();
            let items: Vec<MenuItem<'_>> =
                offered.iter().map(|action| MenuItem::new(action.glyph(), action.label())).collect();
            let at = (button.x, button.bottom() + 4.0);
            let list = widgets::menu_rect(p, at, view, &items);
            // As wide as the button it hangs from.
            let list = Rect::new(list.x, list.y, list.w.max(button.w), list.h);
            if let Some(picked) = widgets::menu(p, ui, list, id("close-action"), &items) {
                self.close = offered.get(picked).copied().unwrap_or(self.close);
                self.choosing = false;
            } else if ui.released && !list.contains(ui.mouse) {
                self.choosing = false;
            }
        }
        chosen
    }

    /// The notice, at the foot of the page until it is clicked or its time is up.
    fn draw_notice(&mut self, p: &mut Painter<'_>, ui: &mut Ui, area: Rect) {
        let Some((text, until, problem)) = &self.notice else { return };
        let problem = *problem;
        if *until <= Instant::now() {
            self.notice = None;
            return;
        }
        let t = p.theme;
        let words = p.layout(text, theme::SMALL, Some(420.0_f32.min(area.w - 120.0).max(80.0)));
        let card = Rect::new(
            (area.x + (area.w - words.width() - 56.0) * 0.5).round(),
            area.bottom() - 34.0 - words.height() - 24.0,
            words.width() + 56.0,
            words.height() + 24.0,
        );
        let appear = ui.appear(id("notice"));
        p.shadow(Rect::new(card.x, card.y + 8.0, card.w, card.h), fade(t.shadow, appear), theme::RADIUS, 28.0);
        p.bordered(card, fade(t.surface, appear), theme::RADIUS, 1.0, fade(t.border_strong, appear));
        let (glyph, tint) = if problem { (icon::ALERT, t.danger) } else { (icon::INFO, t.text_muted) };
        icon::draw(p, glyph, (card.x + 22.0, card.y + 22.0), 15.0, fade(tint, appear));
        p.text(&words, card.x + 40.0, card.y + 12.0, fade(t.text, appear));
        if ui.hovered(card) {
            ui.cursor = CursorIcon::Pointer;
            if ui.clicked(card) {
                self.notice = None;
            }
        }
    }
}
