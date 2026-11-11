//! The card chats and people are picked on: where a message is passed on
//! to, who is put into a chat, who a new group starts with. A search at its
//! top, the candidates under it.

use adelite_chat::{ChatId, Command, FileId, MessageId, SenderId};
use adelite_ui::editor::Editor;
use adelite_ui::paint::{Painter, Rect, fade, mix};
use adelite_ui::ui::{ButtonStyle, Ui, button, icon, icon_button, id};
use adelite_ui::widgets::{Scroll, line_field, modal};
use adelite_ui::{Context, Request, theme};
use winit::window::CursorIcon;

use crate::{ChatModule, Overlay, avatar};

/// Height of a candidate's row.
const ROW_H: f32 = 48.0;

/// What is being picked, and what for.
pub(crate) enum Purpose {
    /// The chat these messages are passed on to.
    Forward {
        /// The chat they are in.
        from: ChatId,
        /// The messages.
        messages: Vec<MessageId>,
    },
    /// Someone to put into this chat.
    Add(ChatId),
    /// The people a new group starts with.
    Group {
        /// Who was picked so far.
        members: Vec<SenderId>,
    },
}

/// The open card.
pub(crate) struct Picker {
    purpose: Purpose,
    /// What the candidates are narrowed down by.
    query: Editor,
    /// The new group's name.
    name: Editor,
    /// Typing goes to the name, not the search.
    naming: bool,
    scroll: Scroll,
    /// The search as last sent to the backend.
    asked: Option<String>,
}

impl Picker {
    /// A card to pick for `purpose`.
    pub(crate) fn new(purpose: Purpose) -> Self {
        let naming = matches!(purpose, Purpose::Group { .. });
        Self {
            purpose,
            query: Editor::default(),
            name: Editor::default(),
            naming,
            scroll: Scroll::default(),
            asked: None,
        }
    }

    /// The box typing goes to.
    pub(crate) fn editor(&mut self) -> &mut Editor {
        if self.naming { &mut self.name } else { &mut self.query }
    }

    /// Moves typing to the card's other box, where it has two.
    pub(crate) fn next_field(&mut self) {
        self.naming = !self.naming && matches!(self.purpose, Purpose::Group { .. });
    }

    /// Whether the new group can be made: it has a name and someone in it.
    fn ready(&self) -> bool {
        matches!(&self.purpose, Purpose::Group { members } if !members.is_empty() && !self.name.text().trim().is_empty())
    }
}

/// A candidate: a chat or a person, as its row shows it.
struct Candidate {
    id: i64,
    name: String,
    photo: Option<FileId>,
    detail: String,
}

impl ChatModule {
    /// Enter on the card: makes the group, where that is what it is for.
    pub(crate) fn picker_enter(&mut self, cx: &mut Context<'_>) {
        let Overlay::Picker(picker) = &mut self.overlay else { return };
        if picker.ready() {
            self.make_group(cx);
        } else if picker.naming {
            picker.naming = false;
        }
    }

    /// Makes the group the card describes.
    fn make_group(&mut self, cx: &mut Context<'_>) {
        if let Overlay::Picker(Picker { purpose: Purpose::Group { members }, name, .. }) = &self.overlay {
            let title = name.text().trim().to_owned();
            cx.requests.push(Request::Info(format!("Creating “{title}”…")));
            self.backend.send(Command::NewGroup { title, members: members.clone() });
        }
        self.close_overlay();
    }

    /// Draws the card over the page.
    pub(crate) fn draw_picker(&mut self, p: &mut Painter<'_>, ui: &mut Ui, area: Rect, cx: &mut Context<'_>) {
        let t = p.theme;
        let (card, outside) = modal(p, ui, area, (420.0, 560.0), "picker");
        let Overlay::Picker(picker) = &mut self.overlay else { return };
        let x = card.x + 20.0;
        let grouping = matches!(picker.purpose, Purpose::Group { .. });
        let title = match picker.purpose {
            Purpose::Forward { .. } => "Forward to",
            Purpose::Add(_) => "Add someone",
            Purpose::Group { .. } => "New group",
        };
        p.label(title, theme::HEADING, x, card.y + 18.0, t.text);
        let close = icon_button(p, ui, Rect::new(card.right() - 44.0, card.y + 12.0, 28.0, 28.0), icon::X, true);
        let mut y = card.y + 52.0;
        let mut field = |p: &mut Painter<'_>, ui: &mut Ui, editor: &mut Editor, glyph, hint, focused: bool| {
            let rect = Rect::new(x, y, card.w - 40.0, 32.0);
            let glow = ui.anim(id(("picker-field", hint)), f32::from(u8::from(focused)));
            p.bordered(rect, t.surface, theme::RADIUS_SM, 1.0, mix(t.border, t.border_focus, glow));
            icon::draw(p, glyph, (rect.x + 16.0, rect.y + 16.0), 14.0, t.text_faint);
            let inner = Rect::new(rect.x + 32.0, rect.y, rect.w - 40.0, rect.h);
            line_field(p, ui, inner, editor, (hint, theme::SMALL), focused);
            y += 40.0;
            let pressed = ui.pressed && ui.hovered(rect);
            if ui.hovered(rect) {
                ui.cursor = CursorIcon::Text;
            }
            (pressed, Rect::new(inner.x, inner.y + 7.0, 2.0, 18.0))
        };
        let mut caret = None;
        if grouping {
            let (pressed, at) = field(p, ui, &mut picker.name, icon::USERS, "Group name", picker.naming);
            picker.naming |= pressed;
            caret = picker.naming.then_some(at);
        }
        let (pressed, at) = field(p, ui, &mut picker.query, icon::SEARCH, "Search", !picker.naming);
        picker.naming &= !pressed;
        self.caret = caret.or(Some(at));

        // People come from the backend, narrowed down as the search changes; chats are all here.
        let query = picker.query.text().trim().to_lowercase();
        let people = !matches!(picker.purpose, Purpose::Forward { .. });
        if people && picker.asked.as_deref() != Some(query.as_str()) {
            picker.asked = Some(query.clone());
            self.backend.send(Command::People(query.clone()));
        }
        let store = &self.store;
        let candidates: Vec<Candidate> = if people {
            let named = |name: &str| query.is_empty() || name.to_lowercase().contains(&query);
            let person = |m: &adelite_chat::Member| Candidate {
                id: m.id,
                name: m.name.clone(),
                photo: m.photo,
                detail: m.status.clone(),
            };
            store.people.iter().filter(|member| named(&member.name)).map(person).collect()
        } else {
            let fits = |chat: &&adelite_chat::Chat| chat.can_send && chat.title.to_lowercase().contains(&query);
            let chats = store.recent().into_iter().filter_map(|chat| store.chat(chat)).filter(fits);
            chats
                .map(|chat| Candidate {
                    id: chat.id,
                    name: chat.title.clone(),
                    photo: chat.photo,
                    detail: chat.status.clone(),
                })
                .collect()
        };

        let foot = if grouping { 52.0 } else { 12.0 };
        let list = Rect::new(card.x + 8.0, y + 2.0, card.w - 16.0, card.bottom() - foot - y - 2.0);
        let offset = picker.scroll.update(ui, ui.hovered(list));
        let clip = p.push_clip(list);
        let mut top = list.y - offset;
        let mut taken = None;
        for candidate in &candidates {
            let row = Rect::new(list.x, top, list.w, ROW_H - 2.0);
            top += ROW_H;
            if row.bottom() < list.y || row.y > list.bottom() {
                continue;
            }
            let hovered = ui.hovered(row) && list.contains(ui.mouse);
            let hover = ui.anim(id(("candidate", candidate.id)), f32::from(u8::from(hovered)));
            p.rect(row, fade(t.hover, hover), theme::RADIUS_SM);
            let face = (row.x + 28.0, row.y + row.h * 0.5);
            avatar(p, (&store.files, &mut self.want), candidate.photo, &candidate.name, face, 34.0);
            let mut name = p.layout(&candidate.name, theme::LABEL, None);
            name.truncate(p.fonts, row.w - 96.0);
            let mut detail = p.layout(&candidate.detail, theme::TINY, None);
            detail.truncate(p.fonts, row.w - 96.0);
            if detail.width() > 0.0 {
                p.text(&name, row.x + 54.0, row.y + 6.0, t.text);
                p.text(&detail, row.x + 54.0, row.y + 25.0, t.text_faint);
            } else {
                p.text(&name, row.x + 54.0, row.y + (row.h - name.height()) * 0.5, t.text);
            }
            if let Purpose::Group { members } = &picker.purpose {
                let chosen = members.contains(&candidate.id);
                let glyph = if chosen { icon::CIRCLE_CHECK } else { icon::CIRCLE };
                let ink = if chosen { t.text } else { t.text_faint };
                icon::draw(p, glyph, (row.right() - 20.0, row.y + row.h * 0.5), 17.0, ink);
            }
            if hovered {
                ui.cursor = CursorIcon::Pointer;
                if ui.clicked(row) {
                    taken = Some((candidate.id, candidate.name.clone()));
                }
            }
        }
        let content = top + offset - list.y;
        if candidates.is_empty() {
            let words = if people && store.people.is_empty() { "Nobody yet" } else { "Nothing by that name" };
            p.label_centered(words, theme::SMALL, Rect::new(list.x, list.y + 20.0, list.w, 20.0), t.text_faint);
        }
        picker.scroll.finish(ui, content, list.h);
        p.set_clip(clip);
        let make = Rect::new(card.right() - 20.0 - 110.0, card.bottom() - 44.0, 110.0, 32.0);
        let create = grouping && button(p, ui, make, "Create", ButtonStyle::Primary, picker.ready());
        if grouping {
            let count = match &picker.purpose {
                Purpose::Group { members } => members.len(),
                _ => 0,
            };
            let words = if count == 1 { "1 person".to_owned() } else { format!("{count} people") };
            p.label(&words, theme::TINY, x, card.bottom() - 36.0, t.text_faint);
        }

        if create {
            return self.make_group(cx);
        }
        match (&mut picker.purpose, taken) {
            (Purpose::Group { members }, Some((id, _))) => match members.iter().position(|member| *member == id) {
                Some(at) => drop(members.remove(at)),
                None => members.push(id),
            },
            (Purpose::Forward { from, messages }, Some((to, name))) => {
                self.backend.send(Command::Forward { from: *from, messages: std::mem::take(messages), to });
                cx.requests.push(Request::Info(format!("Forwarded to {name}")));
                self.close_overlay();
            }
            (Purpose::Add(chat), Some((user, _))) => {
                let chat = *chat;
                self.backend.send(Command::AddMember { chat, user });
                self.backend.send(Command::Members { chat, query: String::new() });
                self.close_overlay();
            }
            (_, None) if outside || close => self.close_overlay(),
            (_, None) => {}
        }
    }
}
