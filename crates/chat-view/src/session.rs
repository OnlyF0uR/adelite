//! Before the chats: the backend starting, saying what it lacks, or asking
//! the user for what signs them in. One card draws whatever a backend asks
//! for, so another network needs no screen of its own.

use adelite_chat::{Command, Prompt, Session};
use adelite_ui::paint::{Painter, Rect};
use adelite_ui::ui::{ButtonStyle, Ui, button, icon};
use adelite_ui::{platform, theme};

use crate::{ChatModule, canvas_under};

/// Width of the card's column.
const WIDTH: f32 = 380.0;
/// Height of a field's box, and of the buttons.
const FIELD_H: f32 = 40.0;

impl ChatModule {
    pub(crate) fn draw_session(&mut self, p: &mut Painter<'_>, ui: &mut Ui, area: Rect) {
        let t = p.theme;
        let canvas = Rect::new(area.x + theme::GAP, area.y, area.w - theme::GAP, area.h - theme::GAP);
        canvas_under(p, canvas);
        let clip = p.push_clip(canvas);
        let width = WIDTH.min(canvas.w - 48.0);
        let x = (canvas.x + (canvas.w - width) * 0.5).round();
        let mid = canvas.y + canvas.h * 0.5;
        match &self.store.session {
            Session::Asking(_) => {
                // Taken out while it is drawn: the fields are the module's too.
                let prompt = std::mem::take(&mut self.asked);
                self.draw_prompt(p, ui, &prompt, (x, mid, width));
                self.asked = prompt;
            }
            Session::Unavailable { title, detail } => {
                let detail = p.layout(detail, theme::SMALL, Some(width));
                let top = mid - f32::midpoint(70.0, detail.height());
                icon::draw(p, icon::ALERT, (canvas.x + canvas.w * 0.5, top + 14.0), 28.0, t.text_faint);
                p.label_centered(title, theme::HEADING, Rect::new(x, top + 38.0, width, 24.0), t.text);
                p.text_aligned(&detail, x, top + 70.0, adelite_ui::text::Align::Center, width, t.text_muted);
            }
            Session::Starting | Session::Ready => {
                let words = format!("Starting {}…", self.backend.name());
                icon::draw(p, icon::MESSAGES, (canvas.x + canvas.w * 0.5, mid - 30.0), 30.0, t.text_faint);
                p.label_centered(&words, theme::SMALL, Rect::new(x, mid, width, 20.0), t.text_muted);
            }
        }
        p.set_clip(clip);
    }

    /// The card for `prompt`, its column `width` wide at `x`, centred on `mid`.
    fn draw_prompt(&mut self, p: &mut Painter<'_>, ui: &mut Ui, prompt: &Prompt, (x, mid, width): (f32, f32, f32)) {
        let t = p.theme;
        let detail = p.layout(&prompt.detail, theme::SMALL, Some(width));
        let problem = p.layout(&prompt.problem, theme::TINY, Some(width));
        let fields = prompt.fields.len().min(self.fields.len());
        let mut height = 34.0 + detail.height() + 18.0 + fields as f32 * (FIELD_H + 32.0);
        height += if prompt.problem.is_empty() { 0.0 } else { problem.height() + 10.0 };
        height += if prompt.action.is_empty() { 0.0 } else { FIELD_H + 8.0 };
        height += if prompt.link.is_some() { 38.0 } else { 0.0 } + if prompt.back { 38.0 } else { 0.0 };
        height += if prompt.alternative.is_empty() { 0.0 } else { 38.0 };
        let mut y = (mid - height * 0.5).round();

        p.label(&prompt.title, theme::TITLE, x, y, t.text);
        y += 34.0;
        p.text(&detail, x, y, t.text_muted);
        y += detail.height() + 18.0;
        for (index, field) in prompt.fields.iter().enumerate().take(fields) {
            p.label(&field.label, theme::CAPTION, x, y, t.text_muted);
            let layout = self.fields.layout(p, index, width);
            self.fields.draw(index, p, ui, Rect::new(x, y + 20.0, width, FIELD_H), layout, &field.hint, !self.waiting);
            y += FIELD_H + 32.0;
        }
        self.caret = self.fields.caret();
        if !prompt.problem.is_empty() {
            p.text(&problem, x, y - 4.0, t.danger);
            y += problem.height() + 10.0;
        }
        if !prompt.action.is_empty() {
            let filled = (0..fields).all(|index| !self.fields.text(index).trim().is_empty());
            let label = if self.waiting { "One moment…" } else { prompt.action.as_str() };
            let action = Rect::new(x, y, width, FIELD_H);
            if button(p, ui, action, label, ButtonStyle::Primary, filled && !self.waiting) {
                self.answer();
            }
            y += FIELD_H + 8.0;
        }
        if !prompt.alternative.is_empty() {
            let other = Rect::new(x, y, width, 32.0);
            if button(p, ui, other, &prompt.alternative, ButtonStyle::Secondary, !self.waiting) {
                self.waiting = true;
                self.backend.send(Command::Alternative);
            }
            y += 38.0;
        }
        if let Some((label, url)) = &prompt.link {
            let words = format!("Open {label}");
            if button(p, ui, Rect::new(x, y, width, 32.0), &words, ButtonStyle::Ghost, true)
                && url.starts_with("https://")
            {
                let _ = platform::open_url(url);
            }
            y += 38.0;
        }
        if prompt.back && button(p, ui, Rect::new(x, y, width, 32.0), "Back", ButtonStyle::Ghost, !self.waiting) {
            self.backend.send(Command::Back);
        }
    }
}
