//! The message box: a card at the foot of the conversation that grows with
//! what is typed, with what it answers or edits and the files that go along
//! over the words, the paperclip at its left and the send button at its right.

use adelite_chat::{Command, Kind};
use adelite_ui::paint::{Painter, Rect, fade, mix};
use adelite_ui::text::TextLayout;
use adelite_ui::theme;
use adelite_ui::ui::{Ui, copy, icon, icon_button, id};
use adelite_ui::{Context, Request};
use winit::window::CursorIcon;

use crate::picker::{Picker, Purpose};
use crate::store::Key;
use crate::{ChatModule, Focus, Overlay};

/// What stands at the foot of a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Compose {
    /// The message box.
    Write,
    /// Nothing can be written here: why.
    Closed(&'static str),
    /// The user is not in this chat: a way in.
    Join,
    /// A bot nobody has spoken to yet: a way to start.
    Start,
    /// Messages are picked: what to do with them.
    Picked,
}

/// Space the paperclip and the send button take beside the words.
const SIDES: (f32, f32) = (46.0, 50.0);
/// Space over and under the words.
const PAD_Y: f32 = 11.0;
/// Height of the line saying what is answered or edited, and of the row of files.
const BANNER_H: f32 = 36.0;
const FILES_H: f32 = 34.0;
/// Lines shown before the words scroll.
const MAX_LINES: f32 = 8.0;

impl ChatModule {
    /// What stands at the foot of the conversation `key`.
    pub(crate) fn compose(&self, key: Key) -> Compose {
        let Some(chat) = self.store.chat(key.0) else { return Compose::Closed("") };
        let closed = self.store.topic(key.0, key.1).is_some_and(|topic| topic.closed);
        let untouched = self.store.thread(key).is_some_and(|thread| thread.loaded && thread.items.is_empty());
        if !self.selected.is_empty() {
            Compose::Picked
        } else if !chat.joined && chat.kind != Kind::Private {
            Compose::Join
        } else if closed && !chat.admin {
            Compose::Closed("This topic is closed")
        } else if !chat.can_send {
            Compose::Closed("You can read here, but not write")
        } else if chat.status == "bot" && untouched {
            Compose::Start
        } else {
            Compose::Write
        }
    }

    /// The draft laid out for a card `width` wide, and the card's height.
    pub(crate) fn composer_layout(&self, p: &Painter<'_>, width: f32, compose: Compose) -> (TextLayout, f32) {
        let layout = p.layout(self.draft.text(), theme::BODY, Some((width - SIDES.0 - SIDES.1).max(40.0)));
        if compose != Compose::Write {
            return (layout, 44.0);
        }
        let line = layout.line_height();
        let banner = if self.reply_to.is_some() || self.editing.is_some() { BANNER_H } else { 0.0 };
        let files = if self.attached.is_empty() { 0.0 } else { FILES_H };
        let words = layout.height().clamp(line, line * MAX_LINES);
        (layout, banner + files + words + 2.0 * PAD_Y)
    }

    pub(crate) fn draw_composer(
        &mut self,
        p: &mut Painter<'_>,
        ui: &mut Ui,
        card: Rect,
        layout: TextLayout,
        compose: Compose,
        cx: &mut Context<'_>,
    ) {
        let t = p.theme;
        let Some(key) = self.current else { return };
        match compose {
            Compose::Write => {}
            Compose::Closed(why) => {
                p.bordered(card, t.surface, theme::RADIUS_LG, 1.0, t.border);
                p.label_centered(why, theme::SMALL, card, t.text_faint);
                return;
            }
            Compose::Join | Compose::Start => {
                // The whole card is the button.
                let hovered = ui.hovered(card);
                let hover = ui.anim(id("join"), f32::from(u8::from(hovered)));
                p.rect(card, fade(t.accent, 1.0 - 0.12 * hover), theme::RADIUS_LG);
                let words = if compose == Compose::Join { "Join" } else { "Start" };
                p.label_centered(words, theme::LABEL, card, t.on_accent);
                if hovered {
                    ui.cursor = CursorIcon::Pointer;
                }
                if ui.clicked(card) && compose == Compose::Join {
                    self.backend.send(Command::Join(key.0));
                } else if ui.clicked(card) {
                    let text = "/start".to_owned();
                    self.backend.send(Command::Send { chat: key.0, topic: key.1, text, reply_to: None });
                }
                return;
            }
            Compose::Picked => return self.draw_picked(p, ui, card, key, cx),
        }
        // Under a menu or a card the box is not being typed in: no caret there.
        let focused = self.focus == Focus::Draft && matches!(self.overlay, Overlay::None);
        let glow = ui.anim(id("composer"), f32::from(u8::from(focused)));
        p.shadow(Rect::new(card.x, card.y + 6.0, card.w, card.h), fade(t.shadow, 0.5), theme::RADIUS_LG, 24.0);
        p.bordered(card, t.surface, theme::RADIUS_LG, 1.0, mix(t.border_strong, t.border_focus, glow));
        let mut y = card.y;

        // What the words answer or replace.
        if self.reply_to.is_some() || self.editing.is_some() {
            let about = self.editing.or(self.reply_to).and_then(|message| self.store.known(key, message));
            let (glyph, title) = match (self.editing.is_some(), about) {
                (true, _) => (icon::PEN, "Edit message".to_owned()),
                (false, Some(message)) if message.outgoing => (icon::REPLY, "Reply to yourself".to_owned()),
                (false, Some(message)) => (icon::REPLY, format!("Reply to {}", message.sender_name)),
                (false, None) => (icon::REPLY, "Reply".to_owned()),
            };
            icon::draw(p, glyph, (card.x + 23.0, y + 20.0), 15.0, t.text_muted);
            let title = p.layout(&title, theme::CAPTION, None);
            p.text(&title, card.x + SIDES.0, y + 11.0, t.text);
            let mut words =
                p.layout(&about.map(|message| message.body.preview()).unwrap_or_default(), theme::TINY, None);
            words.truncate(p.fonts, card.w - SIDES.0 - title.width() - 60.0);
            p.text(&words, card.x + SIDES.0 + title.width() + 8.0, y + 11.0, t.text_faint);
            if icon_button(p, ui, Rect::new(card.right() - 38.0, y + 6.0, 28.0, 28.0), icon::X, true) {
                if self.editing.take().is_some() {
                    self.draft = adelite_ui::editor::Editor::default();
                }
                self.reply_to = None;
            }
            y += BANNER_H;
        }

        // The files that go along, each a chip with a cross.
        if !self.attached.is_empty() {
            let clip = p.push_clip(Rect::new(card.x + 8.0, y, card.w - 16.0, FILES_H));
            let mut x = card.x + 12.0;
            let mut removed = None;
            for (index, path) in self.attached.iter().enumerate() {
                let name =
                    std::path::Path::new(path).file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
                let mut name = p.layout(&name, theme::TINY, None);
                name.truncate(p.fonts, 180.0);
                let tag = Rect::new(x, y + 8.0, name.width() + 52.0, 24.0);
                p.bordered(tag, t.bg, 12.0, 1.0, t.border_strong);
                icon::draw(p, icon::FILE, (tag.x + 14.0, tag.y + 12.0), 12.0, t.text_muted);
                p.text(&name, tag.x + 25.0, tag.y + (tag.h - name.height()) * 0.5, t.text);
                let cross = Rect::new(tag.right() - 24.0, tag.y, 24.0, 24.0);
                let over = ui.hovered(cross);
                icon::draw(
                    p,
                    icon::X,
                    (cross.x + 11.0, cross.y + 12.0),
                    12.0,
                    if over { t.text } else { t.text_faint },
                );
                if over {
                    ui.cursor = CursorIcon::Pointer;
                    if ui.clicked(cross) {
                        removed = Some(index);
                    }
                }
                x += tag.w + 6.0;
            }
            p.set_clip(clip);
            if let Some(index) = removed {
                self.attached.remove(index);
            }
            y += FILES_H;
        }

        // The words: at most a few lines, the caret's line kept in view.
        let line = layout.line_height();
        let words =
            Rect::new(card.x + SIDES.0, y + PAD_Y, card.w - SIDES.0 - SIDES.1, card.bottom() - PAD_Y - y - PAD_Y);
        let (caret_x, caret_y) = layout.caret(self.draft.cursor());
        let slide = (caret_y + line - words.h).max(0.0);
        let origin = (words.x, words.y - slide);
        let hovered = ui.hovered(Rect::new(words.x, y, words.w, card.bottom() - y));
        if hovered {
            ui.cursor = CursorIcon::Text;
            if ui.pressed {
                self.focus = Focus::Draft;
                self.draft.set_cursor(layout.hit(ui.mouse.0 - origin.0, ui.mouse.1 - origin.1), ui.mods.shift_key());
                if ui.clicks == 2 {
                    self.draft.select_word();
                }
                ui.last_edit = ui.time;
            }
        }
        if ui.down && ui.clicks < 2 && words.contains(ui.press_pos) && ui.mouse != ui.press_pos {
            self.draft.set_cursor(layout.hit(ui.mouse.0 - origin.0, ui.mouse.1 - origin.1), true);
        }
        let clip = p.push_clip(Rect::new(words.x - 2.0, words.y, words.w + 4.0, words.h));
        let selection = self.draft.selection();
        if focused && !selection.is_empty() {
            for (from, to, top) in layout.selection_spans(selection.start, selection.end, false) {
                p.rect(Rect::new(origin.0 + from, origin.1 + top, to - from, line), t.selection, 2.0);
            }
        }
        if self.draft.text().is_empty() {
            let hint = match (self.editing.is_some(), self.attached.is_empty()) {
                (true, _) => "Edit message",
                (false, false) => "Add a caption",
                (false, true) => "Message",
            };
            p.label(hint, theme::BODY, origin.0, origin.1, t.text_faint);
        } else {
            p.text(&layout, origin.0, origin.1, t.text);
        }
        if focused {
            self.caret = Some(Rect::new(origin.0 + caret_x, origin.1 + caret_y, 2.0, line));
            if ui.caret_visible() && selection.is_empty() {
                p.rect(Rect::new(origin.0 + caret_x - 1.0, origin.1 + caret_y + 3.0, 2.0, line - 6.0), t.accent, 0.0);
            }
        }
        p.set_clip(clip);

        if icon_button(p, ui, Rect::new(card.x + 8.0, card.bottom() - 38.0, 30.0, 30.0), icon::PAPERCLIP, true) {
            self.pick_files();
        }
        let ready = !self.draft.text().trim().is_empty() || !self.attached.is_empty();
        let send = Rect::new(card.right() - 40.0, card.bottom() - 38.0, 30.0, 30.0);
        let over = ready && ui.hovered(send);
        let hover = ui.anim(id("send"), f32::from(u8::from(over)));
        p.rect(send, if ready { fade(t.accent, 1.0 - 0.12 * hover) } else { t.hover }, 15.0);
        let glyph = if self.editing.is_some() { icon::CHECK } else { icon::ARROW_UP };
        icon::draw(p, glyph, (send.x + 15.0, send.y + 15.0), 16.0, if ready { t.on_accent } else { t.text_faint });
        if over {
            ui.cursor = CursorIcon::Pointer;
        }
        self.draft_view = Some((layout, origin));
        if over && ui.clicked(send) {
            self.submit();
        }
    }

    /// The bar that stands for the message box while messages are picked:
    /// how many, and what to do with them.
    fn draw_picked(&mut self, p: &mut Painter<'_>, ui: &mut Ui, card: Rect, key: Key, cx: &mut Context<'_>) {
        let t = p.theme;
        p.shadow(Rect::new(card.x, card.y + 6.0, card.w, card.h), fade(t.shadow, 0.5), theme::RADIUS_LG, 24.0);
        p.bordered(card, t.surface, theme::RADIUS_LG, 1.0, t.border_strong);
        let count = self.selected.len();
        let words = if count == 1 { "1 message".to_owned() } else { format!("{count} messages") };
        p.label(&words, theme::LABEL, card.x + 18.0, card.y + (card.h - 17.0) * 0.5, t.text);
        let button = |index: f32| Rect::new(card.right() - 10.0 - 34.0 * index, card.y + 7.0, 30.0, 30.0);
        let cancel = icon_button(p, ui, button(1.0), icon::X, true);
        let delete = icon_button(p, ui, button(2.0), icon::TRASH, true);
        let copies = icon_button(p, ui, button(3.0), icon::COPY, true);
        let forward = icon_button(p, ui, button(4.0), icon::FORWARD, true);
        // A row of pictures sent together is picked as one: all of it goes along.
        let row = |message: &i64| {
            let thread = self.store.thread(key)?;
            Some(thread.items.get(thread.find(*message)?)?.messages().map(|message| message.id).collect::<Vec<_>>())
        };
        let mut picked: Vec<i64> =
            self.selected.iter().flat_map(|message| row(message).unwrap_or_else(|| vec![*message])).collect();
        picked.sort_unstable();
        if forward {
            self.overlay = Overlay::Picker(Picker::new(Purpose::Forward { from: key.0, messages: picked }));
            self.selected.clear();
        } else if copies {
            // The words of each, in the order they were written.
            let words: Vec<&str> = picked
                .iter()
                .filter_map(|message| self.store.message(key, *message))
                .map(|message| message.body.text())
                .filter(|words| !words.is_empty())
                .collect();
            copy(cx.clipboard, &words.join("\n\n"));
            cx.requests.push(Request::Info("Copied".to_owned()));
            self.selected.clear();
        } else if delete {
            self.backend.send(Command::Delete { chat: key.0, messages: picked, for_all: true });
            self.selected.clear();
        } else if cancel {
            self.selected.clear();
        }
    }
}
