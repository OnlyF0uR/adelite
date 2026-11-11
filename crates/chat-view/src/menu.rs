//! The menus: what can be done with a chat or a message, and the app's
//! own few lines. A menu opens where the other mouse button
//! went down (or under the button that stands for it); leaving a chat and
//! signing out ask for a second click before they act. Over a message's menu
//! sits a strip of emoji to answer it with.

use adelite_chat::{ChatId, Command, Delivery, Kind, List, MessageId};
use adelite_ui::paint::{Painter, Rect, fade};
use adelite_ui::ui::{Ui, copy, icon, id};
use adelite_ui::widgets::{self, MenuItem};
use adelite_ui::{Context, Request};
use winit::window::CursorIcon;

use crate::picker::{Picker, Purpose};
use crate::{ChatModule, Focus, Overlay};

/// The emoji a message can be answered with at a click.
const ANSWERS: [&str; 8] = ["👍", "❤", "😂", "😮", "😢", "🙏", "🔥", "🎉"];
/// Side of an emoji's cell in the strip, and the strip's height.
const CELL: (f32, f32) = (36.0, 44.0);

/// What a menu is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Target {
    /// A chat in the list, or the open one.
    Chat(ChatId),
    /// A message of the open conversation.
    Message(MessageId),
    /// A message, to answer it with an emoji: the strip alone.
    React(MessageId),
    /// The app itself: what the button beside the search holds.
    More,
}

/// An open menu.
pub(crate) struct Menu {
    target: Target,
    /// Where it opened.
    at: (f32, f32),
    /// The line that was clicked once and asks to be clicked again.
    armed: Option<usize>,
}

impl Menu {
    /// A menu about `target`, opening at `at`.
    pub(crate) fn new(target: Target, at: (f32, f32)) -> Self {
        Self { target, at, armed: None }
    }
}

/// What a line of a menu does.
#[derive(Debug, Clone, Copy)]
enum Do {
    Mute(ChatId, bool),
    PinChat(ChatId, bool),
    Unread(ChatId, bool),
    Archive(ChatId, bool),
    Leave(ChatId),
    Info(ChatId),
    Reply(MessageId),
    Copy(MessageId),
    /// The words picked out of the messages with the mouse.
    CopyMarked,
    Forward(MessageId),
    PinMessage(MessageId, bool),
    Edit(MessageId),
    Select(MessageId),
    /// Delete a message; `true`: for everyone.
    Delete(MessageId, bool),
    NewGroup,
    SignOut,
}

impl ChatModule {
    /// The lines of a menu about `target`, each with what it does.
    fn lines(&self, target: Target) -> Vec<(MenuItem<'static>, Do)> {
        let line = MenuItem::new;
        let mut lines = Vec::new();
        match target {
            Target::Chat(id) => {
                let Some(chat) = self.store.chat(id) else { return lines };
                let list = self.store.list();
                let read = if chat.waiting() {
                    (line(icon::MAIL_OPEN, "Mark as read"), Do::Unread(id, false))
                } else {
                    (line(icon::MAIL, "Mark as unread"), Do::Unread(id, true))
                };
                lines.push(read);
                lines.push(if chat.muted {
                    (line(icon::BELL, "Unmute"), Do::Mute(id, false))
                } else {
                    (line(icon::BELL_OFF, "Mute"), Do::Mute(id, true))
                });
                // A community's chats are in its order, not the user's.
                if let Some(place) = chat.place(list).filter(|_| !matches!(list, List::Community(_))) {
                    lines.push(if place.pinned {
                        (line(icon::PIN_OFF, "Unpin"), Do::PinChat(id, false))
                    } else {
                        (line(icon::PIN, "Pin"), Do::PinChat(id, true))
                    });
                }
                if chat.place(List::Archive).is_some() {
                    lines.push((line(icon::ARCHIVE_OUT, "Unarchive"), Do::Archive(id, false)));
                } else if chat.place(List::Main).is_some() {
                    lines.push((line(icon::ARCHIVE, "Archive"), Do::Archive(id, true)));
                }
                lines.push((line(icon::INFO, "Info").divided(), Do::Info(id)));
                let leave = match chat.kind {
                    Kind::Private => line(icon::TRASH, "Delete chat"),
                    Kind::Group => line(icon::DOOR, "Leave group"),
                    Kind::Channel => line(icon::DOOR, "Leave channel"),
                };
                if chat.joined {
                    lines.push((leave.danger().divided(), Do::Leave(id)));
                }
            }
            Target::Message(id) => {
                let Some(key) = self.current else { return lines };
                let (Some(chat), Some(message)) = (self.store.chat(key.0), self.store.message(key, id)) else {
                    return lines;
                };
                let sent = message.delivery == Delivery::Sent;
                let private = chat.kind == Kind::Private;
                if chat.can_send && sent {
                    lines.push((line(icon::REPLY, "Reply"), Do::Reply(id)));
                }
                // Words picked in this message: those are what is copied.
                let thread = self.store.thread(key);
                let row = thread.and_then(|thread| thread.find(id));
                let marked = self
                    .marked
                    .and_then(|marked| marked.ordered(thread?))
                    .is_some_and(|(from, to)| row.is_some_and(|row| (from.0..=to.0).contains(&row)));
                if marked {
                    lines.push((line(icon::COPY, "Copy selection"), Do::CopyMarked));
                } else if !message.body.text().is_empty() {
                    lines.push((line(icon::COPY, "Copy"), Do::Copy(id)));
                }
                if sent {
                    lines.push((line(icon::FORWARD, "Forward"), Do::Forward(id)));
                }
                if sent && (chat.admin || private) {
                    lines.push(if message.pinned {
                        (line(icon::PIN_OFF, "Unpin"), Do::PinMessage(id, false))
                    } else {
                        (line(icon::PIN, "Pin"), Do::PinMessage(id, true))
                    });
                }
                if message.outgoing && sent && message.body.editable() && chat.can_send {
                    lines.push((line(icon::PEN, "Edit"), Do::Edit(id)));
                }
                if sent {
                    lines.push((line(icon::CIRCLE_CHECK, "Select"), Do::Select(id)));
                }
                // What others wrote goes for everyone only where the user runs the chat, or it is theirs alone.
                if sent && private {
                    lines.push((line(icon::TRASH, "Delete for everyone").danger().divided(), Do::Delete(id, true)));
                    lines.push((line(icon::TRASH, "Delete for me").danger(), Do::Delete(id, false)));
                } else if message.outgoing || chat.admin {
                    lines.push((line(icon::TRASH, "Delete").danger().divided(), Do::Delete(id, true)));
                }
            }
            Target::React(_) => {}
            Target::More => {
                lines.push((line(icon::USERS, "New group"), Do::NewGroup));
                lines.push((line(icon::LOG_OUT, "Sign out").danger().divided(), Do::SignOut));
            }
        }
        lines
    }

    /// Draws the open menu over the page and carries out the line that was clicked.
    pub(crate) fn draw_menu(&mut self, p: &mut Painter<'_>, ui: &mut Ui, area: Rect, cx: &mut Context<'_>) {
        let Overlay::Menu(menu) = &self.overlay else { return };
        let (target, at, armed) = (menu.target, menu.at, menu.armed);
        let lines = self.lines(target);
        let items: Vec<MenuItem<'_>> = lines
            .iter()
            .enumerate()
            .map(
                |(index, (item, _))| {
                    if armed == Some(index) { MenuItem { label: "Click again to confirm", ..*item } } else { *item }
                },
            )
            .collect();
        // A message that is on the server can be answered with an emoji.
        let answering = match target {
            Target::Message(message) | Target::React(message) => self
                .current
                .and_then(|key| self.store.message(key, message))
                .filter(|message| message.delivery == Delivery::Sent)
                .map(|message| message.id),
            _ => None,
        };
        if items.is_empty() && answering.is_none() {
            self.overlay = Overlay::None;
            return;
        }
        let strip_w = ANSWERS.len() as f32 * CELL.0 + 12.0;
        let room = if answering.is_some() { CELL.1 + 6.0 } else { 0.0 };
        // The strip goes over the lines: room is kept for it above them.
        let rect = if items.is_empty() {
            Rect::new(at.0.min(area.right() - strip_w - 6.0).max(area.x + 6.0), at.1 + room, 0.0, 0.0)
        } else {
            let anchor = (at.0, (at.1 + room).min(area.bottom()));
            let rect = widgets::menu_rect(p, anchor, Rect::new(area.x, area.y + room, area.w, area.h - room), &items);
            Rect::new(rect.x.min(area.right() - strip_w - 6.0).max(area.x + 6.0), rect.y, rect.w, rect.h)
        };
        let strip = Rect::new(rect.x, rect.y - room, strip_w, CELL.1);
        let mut done = None;
        if let Some(message) = answering {
            let chosen = self.current.and_then(|key| self.store.message(key, message));
            let chosen = |emoji: &str| chosen.is_some_and(|m| m.reactions.iter().any(|r| r.chosen && r.emoji == emoji));
            if let Some(emoji) = draw_answers(p, ui, strip, &chosen) {
                if let Some((chat, _)) = self.current {
                    let on = !chosen(emoji);
                    self.backend.send(Command::React { chat, message, emoji: emoji.to_owned(), on });
                }
                done = Some(None);
            }
        }
        if !items.is_empty()
            && let Some(index) = widgets::menu(p, ui, rect, id(target), &items)
        {
            done = Some(Some(index));
        }
        let inside = rect.contains(ui.mouse) || (answering.is_some() && strip.contains(ui.mouse));
        match done {
            // Leaving and signing out ask once more; deleting a message does not.
            Some(Some(index))
                if lines.get(index).is_some_and(|(item, action)| item.danger && !matches!(action, Do::Delete(..)))
                    && armed != Some(index) =>
            {
                if let Overlay::Menu(menu) = &mut self.overlay {
                    menu.armed = Some(index);
                }
            }
            Some(picked) => {
                self.overlay = Overlay::None;
                if let Some((_, action)) = picked.and_then(|index| lines.get(index)) {
                    self.carry_out(*action, cx);
                }
            }
            None if (ui.released || ui.context) && !inside => self.overlay = Overlay::None,
            None => {}
        }
    }

    /// Does what a line of a menu stands for.
    fn carry_out(&mut self, action: Do, cx: &mut Context<'_>) {
        // A row of pictures sent together is forwarded and deleted together.
        let row = |module: &Self, message: MessageId| -> Vec<MessageId> {
            let thread = module.current.and_then(|key| module.store.thread(key));
            let item = thread.and_then(|thread| thread.items.get(thread.find(message)?));
            item.map_or_else(|| vec![message], |item| item.messages().map(|message| message.id).collect())
        };
        let chat = self.current.map(|key| key.0);
        match action {
            Do::Mute(chat, on) => self.backend.send(Command::Mute { chat, on }),
            Do::PinChat(chat, on) => self.backend.send(Command::PinChat { chat, list: self.store.list(), on }),
            Do::Unread(chat, on) => self.backend.send(Command::MarkUnread { chat, on }),
            Do::Archive(chat, on) => {
                let to = if on { List::Archive } else { List::Main };
                self.backend.send(Command::Move { chat, to });
            }
            Do::Leave(chat) => self.leave_chat(chat),
            Do::Info(chat) => self.show_info(chat),
            Do::Reply(message) => (self.reply_to, self.editing, self.focus) = (Some(message), None, Focus::Draft),
            Do::Copy(message) => {
                let words = self.current.and_then(|key| self.store.message(key, message)).map(|m| m.body.text());
                copy(cx.clipboard, words.unwrap_or_default());
            }
            Do::CopyMarked => copy(cx.clipboard, &self.marked_text().unwrap_or_default()),
            Do::Forward(message) => {
                if let Some(from) = chat {
                    let purpose = Purpose::Forward { from, messages: row(self, message) };
                    self.overlay = Overlay::Picker(Picker::new(purpose));
                }
            }
            Do::PinMessage(message, on) => {
                if let Some(chat) = chat {
                    self.backend.send(Command::Pin { chat, message, on });
                }
            }
            Do::Edit(message) => self.edit(message),
            Do::Select(message) => self.selected.push(message),
            Do::Delete(message, for_all) => {
                if let Some(chat) = chat {
                    self.backend.send(Command::Delete { chat, messages: row(self, message), for_all });
                }
            }
            Do::NewGroup => {
                self.backend.send(Command::People(String::new()));
                self.overlay = Overlay::Picker(Picker::new(Purpose::Group { members: Vec::new() }));
            }
            Do::SignOut => {
                cx.requests.push(Request::Info("Signing out…".to_owned()));
                self.backend.send(Command::LogOut);
            }
        }
    }
}

/// The strip of emoji a message can be answered with. Returns the one that was clicked.
fn draw_answers(p: &mut Painter<'_>, ui: &mut Ui, strip: Rect, chosen: &dyn Fn(&str) -> bool) -> Option<&'static str> {
    let t = p.theme;
    let appear = ui.appear(id("answers"));
    let strip = Rect::new(strip.x, strip.y - 4.0 * (1.0 - appear), strip.w, strip.h);
    p.shadow(Rect::new(strip.x, strip.y + 8.0, strip.w, strip.h), fade(t.shadow, appear), strip.h * 0.5, 28.0);
    p.bordered(strip, fade(t.surface, appear), strip.h * 0.5, 1.0, fade(t.border_strong, appear));
    let mut picked = None;
    for (index, emoji) in ANSWERS.into_iter().enumerate() {
        let cell = Rect::new(strip.x + 6.0 + index as f32 * CELL.0, strip.y + 4.0, CELL.0, CELL.0);
        let hovered = ui.hovered(cell);
        let hover = ui.anim(id(("answer", index)), f32::from(u8::from(hovered)));
        // The one already given is marked; a click takes it back.
        let lit = if chosen(emoji) { 1.0 } else { hover };
        p.rect(cell, fade(t.hover, lit), CELL.0 * 0.5);
        let style = adelite_ui::text::Style::regular(19.0 + 3.0 * hover);
        p.label_centered(emoji, style, cell, fade(t.text, appear));
        if hovered {
            ui.cursor = CursorIcon::Pointer;
            if ui.clicked(cell) {
                picked = Some(emoji);
            }
        }
    }
    picked
}
