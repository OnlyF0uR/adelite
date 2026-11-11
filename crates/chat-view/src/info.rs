//! A chat's card: who or what it is, what is done with it most, what it
//! says about itself, how it is reached, and under tabs who is in it and
//! the pictures, files and links that were sent in it. Whoever runs the
//! chat changes its name and its words on a form of the card's own, with a
//! button to keep them and one to leave them as they were.

use adelite_chat::{Body, ChatId, Command, FileId, Kind, List, Message, MessageId, Role, SenderId, Shared, Style};
use adelite_ui::editor::Editor;
use adelite_ui::form::Fields;
use adelite_ui::image::Lookup;
use adelite_ui::paint::{Painter, Rect, fade, mix};
use adelite_ui::text::TextLayout;
use adelite_ui::ui::{ButtonStyle, Ui, button, copy, edit_key, icon, icon_button, id};
use adelite_ui::widgets::{Scroll, chip, line_field, modal};
use adelite_ui::{Context, Request, platform, theme};
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::CursorIcon;

use crate::picker::{Picker, Purpose};
use crate::viewer::Viewer;
use crate::{ChatModule, Overlay, avatar, label};

/// The card's size, and the space between its edge and what is in it.
const SIZE: (f32, f32) = (480.0, 660.0);
/// The card's size while the chat's name and words are being changed on it.
const FORM: (f32, f32) = (480.0, 396.0);
const PAD: f32 = 24.0;
/// Height of the card's head: the face and name, then the row of actions.
const HEAD_H: f32 = 164.0;
/// Height of a row: someone, a file, a link.
const ROW_H: f32 = 50.0;
/// Pictures in a row of the grid, and the space between them.
const GRID: (usize, f32) = (4, 3.0);
/// How close to the end of a tab's list more of it is asked for.
const NEAR_END: f32 = 320.0;

/// What the lower part of the card shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    /// Who is in the chat.
    Members,
    /// What was sent in it, of one kind.
    Sent(Shared),
}

/// The open card.
pub(crate) struct Card {
    /// The chat it is about.
    chat: ChatId,
    tab: Tab,
    scroll: Scroll,
    /// What the people are narrowed down by, and that as last asked of the backend.
    query: Editor,
    asked: String,
    /// The chat's name and its words about itself, while they are being changed.
    edit: Option<Fields>,
    /// Leaving was clicked once: again leaves.
    armed: bool,
    /// Removing this person was clicked once: again removes.
    removing: Option<SenderId>,
}

impl Card {
    fn new(chat: ChatId, tab: Tab) -> Self {
        Self {
            chat,
            tab,
            scroll: Scroll::default(),
            query: Editor::default(),
            asked: String::new(),
            edit: None,
            armed: false,
            removing: None,
        }
    }

    /// Text from an input method goes to the box that has the keyboard.
    pub(crate) fn insert(&mut self, text: &str) {
        match &mut self.edit {
            Some(fields) => fields.insert(text),
            None => self.query.insert(text),
        }
    }
}

/// What a click on the card asks for.
enum Act {
    Close,
    Edit,
    Save,
    Cancel,
    Mute(bool),
    Search,
    Pin(bool),
    Archive(bool),
    Leave,
    Copy(String),
    Invite,
    Add,
    Show(Tab),
    Talk(SenderId),
    Promote(SenderId, bool),
    Remove(SenderId),
    /// Look at a picture of the chat's.
    View(MessageId),
    Play(FileId),
    Fetch(FileId),
    Reveal(String),
    Follow(String),
    /// Go to a message in the conversation: its topic (zero: none), and itself.
    Goto(i64, MessageId),
    /// More of what was sent, older than this message.
    More(Shared, MessageId),
}

impl ChatModule {
    /// Opens the card of `chat`.
    pub(crate) fn show_info(&mut self, chat: ChatId) {
        let group = self.store.chat(chat).is_some_and(|chat| chat.kind != Kind::Private);
        let tab = if group { Tab::Members } else { Tab::Sent(Shared::Media) };
        self.behind = None;
        self.overlay = Overlay::Info(Card::new(chat, tab));
        self.backend.send(Command::Info(chat));
        self.ask_tab(chat, tab);
    }

    /// Asks the backend for what a tab of the card shows, from its start.
    fn ask_tab(&mut self, chat: ChatId, tab: Tab) {
        match tab {
            Tab::Members => self.backend.send(Command::Members { chat, query: String::new() }),
            Tab::Sent(kind) => {
                self.store.sent_mut(chat, kind).loading = true;
                self.backend.send(Command::Shared { chat, kind, before: 0 });
            }
        }
    }

    /// A key while the card is open: Escape leaves the form, or the card;
    /// the rest is typed into the form, or narrows down the people.
    pub(crate) fn card_key(&mut self, event: &KeyEvent, mods: ModifiersState, cx: &mut Context<'_>) {
        let Overlay::Info(card) = &mut self.overlay else { return };
        let primary = if cfg!(target_os = "macos") { mods.super_key() } else { mods.control_key() };
        let enter = event.logical_key == Key::Named(NamedKey::Enter);
        match &mut card.edit {
            Some(_) if event.logical_key == Key::Named(NamedKey::Escape) => card.edit = None,
            Some(_) if enter && primary => self.save_card(),
            Some(fields) => {
                fields.key(event, mods, cx.clipboard);
            }
            None if event.logical_key == Key::Named(NamedKey::Escape) => self.close_overlay(),
            None if card.tab == Tab::Members && !enter => {
                edit_key(&mut card.query, event, mods, cx.clipboard);
            }
            None => {}
        }
    }

    /// Opens the card's form on the chat's name and its words about itself as they are.
    pub(crate) fn edit_card(&mut self) {
        let Overlay::Info(card) = &mut self.overlay else { return };
        let (mut name, mut about) = (Editor::default(), Editor::default());
        name.insert(self.store.chat(card.chat).map_or("", |chat| chat.title.as_str()));
        about.insert(self.store.info(card.chat).map_or("", |info| info.about.as_str()));
        card.edit = Some(Fields::new([name, about], [false, true]));
    }

    /// Shows another tab of the card, asking for what it lists.
    pub(crate) fn show_tab(&mut self, tab: Tab) {
        let Overlay::Info(card) = &mut self.overlay else { return };
        (card.tab, card.armed, card.removing) = (tab, false, None);
        card.scroll.reset();
        let chat = card.chat;
        self.ask_tab(chat, tab);
    }

    /// Keeps what the card's form holds: the chat's new name and its new words about itself.
    fn save_card(&mut self) {
        let Overlay::Info(card) = &mut self.overlay else { return };
        let (chat, Some(fields)) = (card.chat, card.edit.take()) else { return };
        let (title, about) = (fields.text(0).trim().to_owned(), fields.text(1).trim().to_owned());
        if !title.is_empty() && self.store.chat(chat).is_some_and(|chat| chat.title != title) {
            self.backend.send(Command::Rename { chat, title });
        }
        // The card shows the new words when the network has kept them and says so.
        if self.store.info(chat).is_none_or(|info| info.about != about) {
            self.backend.send(Command::Describe { chat, about });
        }
    }

    /// Leaves a chat, or drops the conversation with a person: it goes from the list.
    pub(crate) fn leave_chat(&mut self, chat: ChatId) {
        if self.current.is_some_and(|key| key.0 == chat) {
            // What was typed there goes with it.
            self.draft = Editor::default();
            self.leave();
        }
        if self.forum == Some(chat) {
            self.forum = None;
        }
        self.drafts.retain(|key, _| key.0 != chat);
        self.backend.send(Command::Leave(chat));
    }

    /// Draws the card over the page and carries out what was clicked on it.
    pub(crate) fn draw_card(&mut self, p: &mut Painter<'_>, ui: &mut Ui, area: Rect, cx: &mut Context<'_>) {
        // The form is a card of its own size: two boxes and two buttons.
        let editing = matches!(&self.overlay, Overlay::Info(card) if card.edit.is_some());
        let (card, outside) = modal(p, ui, area, if editing { FORM } else { SIZE }, "info");
        let act = self.draw_card_parts(p, ui, card);
        let Overlay::Info(state) = &mut self.overlay else { return };
        let chat = state.chat;
        // The people are narrowed down as the search changes.
        let query = state.query.text().trim().to_owned();
        if state.tab == Tab::Members && state.edit.is_none() && query != state.asked {
            state.asked.clone_from(&query);
            state.scroll.reset();
            self.backend.send(Command::Members { chat, query });
        }
        match act {
            Some(Act::Close) => self.close_overlay(),
            Some(Act::Edit) => self.edit_card(),
            Some(Act::Save) => self.save_card(),
            Some(Act::Cancel) => state.edit = None,
            Some(Act::Mute(on)) => self.backend.send(Command::Mute { chat, on }),
            Some(Act::Search) => {
                self.close_overlay();
                if self.current.is_none_or(|key| key.0 != chat) {
                    self.open((chat, 0), None);
                }
                self.search_here(true);
            }
            Some(Act::Pin(on)) => self.backend.send(Command::PinChat { chat, list: self.store.list(), on }),
            Some(Act::Archive(on)) => {
                let to = if on { List::Archive } else { List::Main };
                self.backend.send(Command::Move { chat, to });
            }
            Some(Act::Leave) => {
                self.close_overlay();
                self.leave_chat(chat);
            }
            Some(Act::Copy(text)) => {
                copy(cx.clipboard, &text);
                cx.requests.push(Request::Info("Copied".to_owned()));
            }
            Some(Act::Invite) => self.backend.send(Command::Invite(chat)),
            Some(Act::Add) => {
                // The card waits behind the one people are picked on.
                if let Overlay::Info(card) = std::mem::replace(&mut self.overlay, Overlay::None) {
                    self.behind = Some(card);
                }
                self.overlay = Overlay::Picker(Picker::new(Purpose::Add(chat)));
            }
            Some(Act::Show(tab)) => self.show_tab(tab),
            Some(Act::Talk(user)) => {
                self.close_overlay();
                self.backend.send(Command::Talk(user));
            }
            Some(Act::Promote(user, on)) => {
                self.backend.send(Command::Promote { chat, user, on });
                self.backend.send(Command::Members { chat, query: state.asked.clone() });
            }
            Some(Act::Remove(user)) => {
                self.backend.send(Command::RemoveMember { chat, user });
                self.backend.send(Command::Members { chat, query: state.asked.clone() });
            }
            Some(Act::View(message)) => {
                let sent = self.store.sent(chat, Shared::Media).map_or(&[][..], |list| list.messages.as_slice());
                if let Some(viewer) = Viewer::among(sent, message) {
                    if let Overlay::Info(card) = std::mem::replace(&mut self.overlay, Overlay::None) {
                        self.behind = Some(card);
                    }
                    self.overlay = Overlay::Viewer(viewer);
                }
            }
            Some(Act::Play(file)) => {
                self.playing = Some(file);
                self.want.push(file);
            }
            Some(Act::Fetch(file)) => self.want.push(file),
            Some(Act::Reveal(path)) => {
                if let Some(folder) = std::path::Path::new(&path).parent() {
                    let _ = platform::open_folder(folder);
                }
            }
            Some(Act::Follow(to)) => {
                self.close_overlay();
                self.backend.send(Command::Follow(to));
            }
            Some(Act::Goto(topic, message)) => {
                self.close_overlay();
                self.open((chat, topic), Some(message));
            }
            Some(Act::More(kind, before)) => {
                self.store.sent_mut(chat, kind).loading = true;
                self.backend.send(Command::Shared { chat, kind, before });
            }
            // A click beside the card closes it, unless something is being changed on it.
            None if outside && state.edit.is_none() => self.close_overlay(),
            None => {}
        }
    }

    /// The card's parts in `card`. Returns what was clicked.
    #[expect(clippy::many_single_char_names, reason = "geometry: the painter, the theme, and x, y, w")]
    fn draw_card_parts(&mut self, p: &mut Painter<'_>, ui: &mut Ui, card: Rect) -> Option<Act> {
        let t = p.theme;
        let Overlay::Info(state) = &mut self.overlay else { return None };
        let chat = self.store.chats.get(&state.chat)?;
        let info = self.store.info(chat.id);
        let (x, w) = (card.x + PAD, card.w - 2.0 * PAD);
        let mut act = None;
        // The form has its own two ways out; the card has a cross.
        let cross = Rect::new(card.right() - 44.0, card.y + 16.0, 28.0, 28.0);
        if state.edit.is_none() && icon_button(p, ui, cross, icon::X, true) {
            act = Some(Act::Close);
        }

        // The form: the chat's name and its words about itself, and the two ways out of it.
        if let Some(fields) = &mut state.edit {
            p.label("Edit", theme::HEADING, x, card.y + 20.0, t.text);
            let mut y = card.y + 64.0;
            for (index, (name, hint, h)) in
                [("Name", "What it is called", 40.0), ("Description", "What it is about", 132.0)]
                    .into_iter()
                    .enumerate()
            {
                p.label(name, theme::CAPTION, x, y, t.text_muted);
                let layout = fields.layout(p, index, w);
                fields.draw(index, p, ui, Rect::new(x, y + 20.0, w, h), layout, hint, true);
                y += h + 36.0;
            }
            self.caret = fields.caret();
            let modifier = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
            p.label(
                &format!("{modifier} + Enter saves, Escape leaves it as it was."),
                theme::TINY,
                x,
                y - 8.0,
                t.text_faint,
            );
            let named = !fields.text(0).trim().is_empty();
            let save = Rect::new(card.right() - PAD - 96.0, card.bottom() - PAD - 34.0, 96.0, 34.0);
            let cancel = Rect::new(save.x - 104.0, save.y, 96.0, 34.0);
            if button(p, ui, cancel, "Cancel", ButtonStyle::Secondary, true) {
                act = Some(Act::Cancel);
            }
            if button(p, ui, save, "Save", ButtonStyle::Primary, named) {
                act = Some(Act::Save);
            }
            return act;
        }

        // Who or what it is.
        avatar(p, (&self.store.files, &mut self.want), chat.photo, &chat.title, (x + 28.0, card.y + 48.0), 56.0);
        let can_edit = info.is_some_and(|info| info.can_edit);
        let room = w - 72.0 - if can_edit { 72.0 } else { 38.0 };
        let mut title = p.layout(&chat.title, theme::HEADING, None);
        title.truncate(p.fonts, room);
        let status = match (chat.status.as_str(), chat.kind) {
            ("", Kind::Private) if chat.last_seen > 0 => label::last_seen(chat.last_seen, self.today),
            ("", Kind::Private) => String::new(),
            ("", Kind::Group) => "group".to_owned(),
            ("", Kind::Channel) => "channel".to_owned(),
            (status, _) => status.to_owned(),
        };
        if status.is_empty() {
            p.text(&title, x + 72.0, card.y + 48.0 - title.height() * 0.5, t.text);
        } else {
            p.text(&title, x + 72.0, card.y + 28.0, t.text);
            p.label(&status, theme::SMALL, x + 72.0, card.y + 51.0, t.text_faint);
        }
        if can_edit && icon_button(p, ui, Rect::new(card.right() - 78.0, card.y + 16.0, 28.0, 28.0), icon::PEN, true) {
            act = Some(Act::Edit);
        }

        // What is done with it most, each a tile: its bell, a look through it, its place in the list, the way out.
        let shown = self.store.list();
        let place = chat.place(shown).filter(|_| !matches!(shown, List::Community(_)));
        let archived = chat.place(List::Archive).is_some();
        let leave = match (state.armed, chat.kind) {
            (true, _) => "Sure?",
            (false, Kind::Private) => "Delete",
            (false, _) => "Leave",
        };
        let tiles = [
            Some(if chat.muted { (icon::BELL, "Unmute", false) } else { (icon::BELL_OFF, "Mute", false) }),
            Some((icon::SEARCH, "Search", false)),
            place.map(|place| if place.pinned { (icon::PIN_OFF, "Unpin", false) } else { (icon::PIN, "Pin", false) }),
            (archived || chat.place(List::Main).is_some()).then_some(if archived {
                (icon::ARCHIVE_OUT, "Unarchive", false)
            } else {
                (icon::ARCHIVE, "Archive", false)
            }),
            chat.joined.then_some((if chat.kind == Kind::Private { icon::TRASH } else { icon::DOOR }, leave, true)),
        ];
        let count = tiles.iter().flatten().count() as f32;
        let tile_w = (w - 8.0 * (count - 1.0)) / count;
        let mut leaving = false;
        for (at, (glyph, name, danger)) in tiles.into_iter().flatten().enumerate() {
            let rect = Rect::new(x + at as f32 * (tile_w + 8.0), card.y + 92.0, tile_w, 56.0);
            if !tile(p, ui, rect, glyph, name, danger, danger && state.armed) {
                continue;
            }
            act = match name {
                "Mute" | "Unmute" => Some(Act::Mute(!chat.muted)),
                "Search" => Some(Act::Search),
                "Pin" | "Unpin" => Some(Act::Pin(name == "Pin")),
                "Archive" | "Unarchive" => Some(Act::Archive(!archived)),
                _ => {
                    leaving = true;
                    state.armed.then_some(Act::Leave)
                }
            };
        }
        // A click anywhere else takes the question back.
        if ui.released {
            state.armed = leaving && !state.armed;
        }

        // Under the head, everything scrolls: its words, how it is reached, then the tabs.
        let body = Rect::new(card.x, card.y + HEAD_H, card.w, card.h - HEAD_H - 12.0);
        p.rect(Rect::new(x, body.y, w, 1.0), t.border, 0.0);
        let body = Rect::new(body.x, body.y + 1.0, body.w, body.h - 1.0);
        let offset = state.scroll.update(ui, ui.hovered(body));
        let clip = p.push_clip(body);
        let mut y = body.y + 14.0 - offset;
        let about = info.map_or("", |info| info.about.as_str());
        if !about.is_empty() {
            let layout = TextLayout::new(p.fonts, about, theme::SMALL, Some(w - 30.0), p.scale);
            icon::draw(p, icon::INFO, (x + 8.0, y + 9.0), 14.0, t.text_faint);
            p.text(&layout, x + 30.0, y, t.text);
            y += layout.height().max(20.0) + 12.0;
        }
        let lines = [
            (icon::AT, info.map_or("", |info| info.handle.as_str())),
            (icon::PHONE, info.map_or("", |info| info.phone.as_str())),
            (icon::LINK, info.map_or("", |info| info.link.as_str())),
        ];
        for (glyph, words) in lines.into_iter().filter(|(_, words)| !words.is_empty()) {
            let row = Rect::new(x - 8.0, y - 5.0, w + 16.0, 30.0);
            let hovered = ui.hovered(row) && body.contains(ui.mouse);
            let hover = ui.anim(id(("info-line", glyph)), f32::from(u8::from(hovered)));
            p.rect(row, fade(t.hover, hover), theme::RADIUS_SM);
            icon::draw(p, glyph, (x + 8.0, y + 9.0), 14.0, t.text_faint);
            let mut layout = p.layout(words.trim_start_matches('@'), theme::SMALL, None);
            layout.truncate(p.fonts, w - 56.0);
            p.text(&layout, x + 30.0, y, t.text);
            icon::draw(p, icon::COPY, (row.right() - 16.0, y + 9.0), 12.0, fade(t.text_faint, hover));
            if hovered {
                ui.cursor = CursorIcon::Pointer;
                if ui.clicked(row) {
                    act = Some(Act::Copy(words.to_owned()));
                }
            }
            y += 32.0;
        }
        let can_invite = info.is_some_and(|info| info.can_invite);
        if can_invite && info.is_some_and(|info| info.link.is_empty()) && chat.kind != Kind::Private {
            let row = Rect::new(x - 8.0, y - 5.0, w + 16.0, 30.0);
            let hovered = ui.hovered(row) && body.contains(ui.mouse);
            let hover = ui.anim(id("info-invite"), f32::from(u8::from(hovered)));
            p.rect(row, fade(t.hover, hover), theme::RADIUS_SM);
            icon::draw(p, icon::LINK, (x + 8.0, y + 9.0), 14.0, t.text_faint);
            p.label("Make an invite link", theme::SMALL, x + 30.0, y, mix(t.text_muted, t.text, hover));
            if hovered {
                ui.cursor = CursorIcon::Pointer;
                if ui.clicked(row) {
                    act = Some(Act::Invite);
                }
            }
            y += 32.0;
        }

        // The tabs: a chip each.
        y += 6.0;
        let tabs = [
            (chat.kind != Kind::Private).then_some((Tab::Members, "Members")),
            Some((Tab::Sent(Shared::Media), "Media")),
            Some((Tab::Sent(Shared::Files), "Files")),
            Some((Tab::Sent(Shared::Links), "Links")),
        ];
        let mut chip_x = x;
        for (tab, name) in tabs.into_iter().flatten() {
            let width = p.layout(name, theme::SMALL, None).width() + 26.0;
            let rect = Rect::new(chip_x, y, width, 28.0);
            chip_x += width + 6.0;
            if chip(p, ui, rect, name, state.tab == tab) && body.contains(ui.mouse) && state.tab != tab {
                act = Some(Act::Show(tab));
            }
        }
        y += 42.0;

        let view = (body, ui.hovered(body).then_some(ui.mouse));
        let (store, want) = (&self.store, &mut self.want);
        let mut removing = None;
        let found = match state.tab {
            Tab::Members => {
                // A search through them, and a way to put someone in.
                let add = if can_invite { 36.0 } else { 0.0 };
                let field = Rect::new(x, y, w - add, 32.0);
                p.bordered(field, t.surface, theme::RADIUS_SM, 1.0, t.border_focus);
                icon::draw(p, icon::SEARCH, (field.x + 16.0, field.y + 16.0), 14.0, t.text_faint);
                let inner = Rect::new(field.x + 32.0, field.y, field.w - 40.0, field.h);
                line_field(p, ui, inner, &mut state.query, ("Search members", theme::SMALL), true);
                self.caret = Some(Rect::new(inner.x, inner.y + 7.0, 2.0, 18.0));
                if can_invite && icon_button(p, ui, Rect::new(x + w - 30.0, y + 1.0, 30.0, 30.0), icon::USER_PLUS, true)
                {
                    act = Some(Act::Add);
                }
                y += 42.0;
                let (of, members) = &store.members;
                let people = if *of == chat.id { members.as_slice() } else { &[] };
                for member in people {
                    let row = Rect::new(x - 8.0, y, w + 16.0, ROW_H - 4.0);
                    y += ROW_H;
                    if row.bottom() < body.y || row.y > body.bottom() {
                        continue;
                    }
                    let hovered = view.1.is_some_and(|mouse| row.contains(mouse));
                    let hover = ui.anim(id(("member", member.id)), f32::from(u8::from(hovered)));
                    p.rect(row, fade(t.hover, hover), theme::RADIUS_SM);
                    let face = (row.x + 25.0, row.y + row.h * 0.5);
                    avatar(p, (&store.files, &mut *want), member.photo, &member.name, face, 34.0);
                    // Whoever runs the chat sees, under the mouse, what can be done with them.
                    let manage = hovered && chat.admin && member.role != Role::Owner;
                    let mark = match member.role {
                        Role::Owner => Some(icon::CROWN),
                        Role::Admin => Some(icon::SHIELD),
                        Role::Member => None,
                    };
                    let room = row.w - 60.0 - if manage { 68.0 } else { 28.0 };
                    two_lines(p, (row.x + 50.0, row.y, room, row.h), &member.name, &member.status);
                    let mut on_tool = false;
                    if manage {
                        let promote = Rect::new(row.right() - 66.0, row.y + 8.0, 30.0, 30.0);
                        let remove = Rect::new(row.right() - 34.0, row.y + 8.0, 30.0, 30.0);
                        on_tool = ui.hovered(promote) || ui.hovered(remove);
                        let admin = member.role == Role::Admin;
                        if admin {
                            p.rect(promote, t.active, theme::RADIUS_SM);
                        }
                        if icon_button(p, ui, promote, icon::SHIELD, true) {
                            act = Some(Act::Promote(member.id, !admin));
                        }
                        let armed = state.removing == Some(member.id);
                        if armed {
                            p.rect(remove, fade(t.danger, 0.16), theme::RADIUS_SM);
                        }
                        if icon_button(p, ui, remove, icon::USER_MINUS, true) {
                            removing = Some(member.id);
                            act = armed.then_some(Act::Remove(member.id));
                        }
                    } else if let Some(mark) = mark {
                        icon::draw(p, mark, (row.right() - 18.0, row.y + row.h * 0.5), 14.0, t.text_faint);
                    }
                    if hovered && !on_tool {
                        ui.cursor = CursorIcon::Pointer;
                        if ui.clicked(row) {
                            act = Some(Act::Talk(member.id));
                        }
                    }
                }
                (!people.is_empty()).then_some(()).ok_or(if *of == chat.id { "Nobody to show" } else { "Loading…" })
            }
            Tab::Sent(kind) => {
                let list = store.sent(chat.id, kind);
                let messages = list.map_or(&[][..], |list| list.messages.as_slice());
                let today = self.today;
                let mut sent = Sent { files: &store.files, want, fetching: &self.asked_files, view, today };
                let clicked = match kind {
                    Shared::Media => sent.grid(p, ui, messages, (x, &mut y, w)),
                    Shared::Files | Shared::Links => sent.rows(p, ui, messages, kind, (x, &mut y, w)),
                };
                act = clicked.or(act);
                // Near the end of what is there: the page before it.
                let waiting = list.is_none_or(|list| list.loading);
                let more = list.filter(|list| !list.loading && !list.complete).and_then(|list| list.messages.last());
                if let Some(last) = more.filter(|_| y - body.bottom() < NEAR_END && act.is_none()) {
                    act = Some(Act::More(kind, last.id));
                }
                (!messages.is_empty()).then_some(()).ok_or(if waiting { "Loading…" } else { "Nothing here yet" })
            }
        };
        if let Err(words) = found {
            p.label_centered(words, theme::SMALL, Rect::new(x, y + 18.0, w, 20.0), t.text_faint);
            y += 56.0;
        }
        // A click anywhere else takes the question back.
        if ui.released {
            state.removing = removing.filter(|user| state.removing != Some(*user));
        }
        let content = y + offset - body.y + 8.0;
        state.scroll.finish(ui, content, body.h);
        if offset > 0.5 {
            p.gradient(Rect::new(body.x + 1.0, body.y, body.w - 2.0, 12.0), t.bg, fade(t.bg, 0.0), 0.0);
        }
        state.scroll.bar(p, Rect::new(body.x, body.y, body.w - 4.0, body.h), content);
        p.set_clip(clip);
        act
    }
}

/// What the tabs of sent things are drawn with.
struct Sent<'a> {
    files: &'a std::collections::HashMap<FileId, String>,
    want: &'a mut Vec<FileId>,
    /// The files that were asked for and have not come.
    fetching: &'a std::collections::HashSet<FileId>,
    /// The part of the card that scrolls, and the mouse when it is over it.
    view: (Rect, Option<(f32, f32)>),
    today: label::Today,
}

impl Sent<'_> {
    /// The pictures and videos that were sent, as a grid of squares from
    /// `y` down in the column `(x, w)`. Returns what was clicked.
    #[expect(clippy::many_single_char_names, reason = "geometry: the painter, the theme, and x, y, w")]
    fn grid(
        &mut self,
        p: &mut Painter<'_>,
        ui: &mut Ui,
        messages: &[Message],
        (x, y, w): (f32, &mut f32, f32),
    ) -> Option<Act> {
        let t = p.theme;
        let side = ((w - GRID.1 * (GRID.0 as f32 - 1.0)) / GRID.0 as f32).floor();
        let mut act = None;
        for (at, message) in messages.iter().enumerate() {
            let (column, row) = ((at % GRID.0) as f32, (at / GRID.0) as f32);
            let cell = Rect::new(x + column * (side + GRID.1), *y + row * (side + GRID.1), side, side);
            if cell.bottom() < self.view.0.y || cell.y > self.view.0.bottom() {
                continue;
            }
            let (still, moving) = match &message.body {
                Body::Photo { file, .. } => (*file, None),
                Body::Video { still, file, seconds, .. } => (*still, Some((*file, *seconds))),
                _ => (None, None),
            };
            let path = still.and_then(|file| self.files.get(&file));
            if path.is_none() {
                self.want.extend(still);
            }
            if !path.is_some_and(|path| matches!(p.image(path, cell, 6.0), Lookup::Ready(_))) {
                p.rect(cell, t.hover, 6.0);
            }
            if let Some((_, seconds)) = moving {
                let length = p.layout(&label::duration(seconds), theme::TINY, None);
                let pill = Rect::new(cell.x + 5.0, cell.bottom() - 23.0, length.width() + 24.0, 18.0);
                p.rect(pill, t.scrim, 9.0);
                icon::draw(p, icon::PLAY, (pill.x + 10.0, pill.y + 9.0), 9.0, t.on_scrim);
                p.text(&length, pill.x + 18.0, pill.y + (pill.h - length.height()) * 0.5, t.on_scrim);
            }
            if self.view.1.is_some_and(|mouse| cell.contains(mouse)) {
                ui.cursor = CursorIcon::Pointer;
                p.bordered(cell, fade(t.text, 0.0), 6.0, 2.0, fade(t.on_scrim, 0.7));
                if ui.clicked(cell) {
                    act = match moving {
                        Some((file, _)) => file.map(Act::Play),
                        None => Some(Act::View(message.id)),
                    };
                }
            }
        }
        let rows = messages.len().div_ceil(GRID.0) as f32;
        *y += rows * (side + GRID.1);
        act
    }

    /// The files or the links that were sent, a row each from `y` down in
    /// the column `(x, w)`. Returns what was clicked.
    #[expect(clippy::many_single_char_names, reason = "geometry: the painter, the theme, and x, y, w")]
    fn rows(
        &mut self,
        p: &mut Painter<'_>,
        ui: &mut Ui,
        messages: &[Message],
        kind: Shared,
        (x, y, w): (f32, &mut f32, f32),
    ) -> Option<Act> {
        let t = p.theme;
        let mut act = None;
        for message in messages {
            let row = Rect::new(x - 8.0, *y, w + 16.0, ROW_H - 4.0);
            *y += ROW_H;
            if row.bottom() < self.view.0.y || row.y > self.view.0.bottom() {
                continue;
            }
            let hovered = self.view.1.is_some_and(|mouse| row.contains(mouse));
            let hover = ui.anim(id(("sent", message.id)), f32::from(u8::from(hovered)));
            p.rect(row, fade(t.hover, hover), theme::RADIUS_SM);
            let when = label::list_time(message.date, self.today);
            // What the row is, what a click on it does, and the two lines that say it.
            let file = match &message.body {
                Body::File { file, name, size, .. } if kind == Shared::Files => Some((file, name, size)),
                _ => None,
            };
            let (glyph, click, name, detail) = if let Some((file, name, size)) = file {
                let path = file.and_then(|file| self.files.get(&file));
                let state = match (path, file) {
                    (Some(_), _) => "Show in folder",
                    (None, Some(file)) if self.fetching.contains(file) => "Downloading…",
                    _ => "Download",
                };
                let click = match path {
                    Some(path) => Some(Act::Reveal(path.clone())),
                    None => (*file).map(Act::Fetch),
                };
                let glyph = if path.is_some() { icon::FILE } else { icon::DOWNLOAD };
                (glyph, click, name.clone(), format!("{} · {when} · {state}", label::file_size(*size)))
            } else {
                let url = address(message);
                let name = message.link.as_ref().map(|link| link.title.clone()).filter(|title| !title.is_empty());
                let name = name.or_else(|| url.clone()).unwrap_or_else(|| message.body.preview());
                let detail = format!("{} · {when}", url.as_deref().unwrap_or(&message.sender_name));
                (icon::LINK, url.map(Act::Follow), name, detail)
            };
            let badge = Rect::new(row.x + 8.0, row.y + 6.0, 34.0, 34.0);
            p.bordered(badge, t.bg, 10.0, 1.0, t.border_strong);
            icon::draw(p, glyph, (badge.x + 17.0, badge.y + 17.0), 15.0, t.text_muted);
            two_lines(p, (row.x + 52.0, row.y, row.w - 62.0 - if hovered { 34.0 } else { 0.0 }, row.h), &name, &detail);
            // Under the mouse: the way to where it was said.
            let goto = Rect::new(row.right() - 34.0, row.y + 8.0, 30.0, 30.0);
            if hovered && icon_button(p, ui, goto, icon::COMMENT, true) {
                act = Some(Act::Goto(message.topic, message.id));
            } else if hovered && !goto.contains(ui.mouse) {
                ui.cursor = CursorIcon::Pointer;
                if ui.clicked(row) {
                    act = click.or(act);
                }
            }
        }
        act
    }
}

/// The address a message leads to: its page's, or its first link's.
fn address(message: &Message) -> Option<String> {
    let linked = message.link.as_ref().map(|link| link.url.clone()).filter(|url| !url.is_empty());
    let spans = message.body.words().map_or(&[][..], |words| words.spans.as_slice());
    linked.or_else(|| {
        spans.iter().find_map(|span| match &span.style {
            Style::Link(url) => Some(url.clone()),
            _ => None,
        })
    })
}

/// A name with a quieter line under it, in the room `(x, y, w, h)`; the name alone where there is no line.
fn two_lines(p: &mut Painter<'_>, (left, top, width, height): (f32, f32, f32, f32), name: &str, detail: &str) {
    let t = p.theme;
    let mut name = p.layout(name, theme::LABEL, None);
    name.truncate(p.fonts, width);
    let mut detail = p.layout(detail, theme::TINY, None);
    detail.truncate(p.fonts, width);
    if detail.width() > 0.0 {
        p.text(&name, left, top + height * 0.5 - 17.0, t.text);
        p.text(&detail, left, top + height * 0.5 + 2.0, t.text_faint);
    } else {
        p.text(&name, left, top + (height - name.height()) * 0.5, t.text);
    }
}

/// A tile of the card's row of actions: an icon over its name. `danger`:
/// it cannot be undone; `armed`: it was clicked once and asks to be
/// clicked again. Returns whether it was clicked.
fn tile(p: &mut Painter<'_>, ui: &mut Ui, rect: Rect, glyph: char, name: &str, danger: bool, armed: bool) -> bool {
    let t = p.theme;
    let hovered = ui.hovered(rect);
    let hover = ui.anim(id(("info-tile", rect.x.to_bits())), f32::from(u8::from(hovered)));
    let tint = if danger { t.danger } else { t.text };
    let fill = if armed { fade(t.danger, 0.16) } else { mix(t.surface, t.hover, hover) };
    let line = if danger { fade(t.danger, 0.25 + 0.3 * hover) } else { mix(t.border, t.border_strong, hover) };
    p.bordered(rect, fill, theme::RADIUS, 1.0, line);
    let ink = if danger { t.danger } else { mix(t.text_muted, t.text, hover) };
    icon::draw(p, glyph, (rect.x + rect.w * 0.5, rect.y + 20.0), 16.0, ink);
    let words = p.layout(name, theme::TINY, None);
    p.text(&words, rect.x + (rect.w - words.width()) * 0.5, rect.y + 33.0, mix(ink, tint, hover));
    if hovered {
        ui.cursor = CursorIcon::Pointer;
    }
    hovered && ui.clicked(rect)
}
