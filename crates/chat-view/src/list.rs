//! The sidebar: a search over everything, the strip of lists the chats are
//! kept in, then the chats, each a face, a name, its latest message and
//! what is unread. A forum's topics take the chats' place while it is
//! open; what a search found takes it while something is typed.

use adelite_chat::{Chat, ChatId, Command, Kind, List, Message, Preview, Topic};
use adelite_ui::paint::{Painter, Rect, fade, mix};
use adelite_ui::theme;
use adelite_ui::ui::{Ui, icon, icon_button, id};
use adelite_ui::widgets::line_field;
use winit::window::CursorIcon;

use crate::menu::{Menu, Target};
use crate::{ChatModule, Focus, Overlay, avatar, label};

/// Height of a chat's row.
const ROW_H: f32 = 64.0;
/// Height of the strip of lists, and of the line a forum is named on.
const STRIP_H: f32 = 34.0;
/// Height of a heading between what a search found.
const HEADING_H: f32 = 30.0;
/// Space between the sidebar's edge and its rows.
const INSET: f32 = theme::GAP;
/// How close to the list's end more chats are asked for.
const NEAR_END: f32 = 600.0;

/// What a click in the list asks for.
enum Pick {
    Chat(ChatId),
    Topic(ChatId, i64),
    Message(Box<Message>),
    Menu(ChatId, (f32, f32)),
}

impl ChatModule {
    pub(crate) fn draw_list(&mut self, p: &mut Painter<'_>, ui: &mut Ui, area: Rect) {
        let t = p.theme;
        let top = area.y + 2.0;
        let more = Rect::new(area.right() - INSET - 30.0, top, 30.0, 30.0);
        let field = Rect::new(area.x + INSET, top, more.x - 4.0 - area.x - INSET, 30.0);
        let searching = self.focus == Focus::Search || self.current.is_none();
        let glow = ui.anim(id("chat-search"), f32::from(u8::from(searching)));
        p.bordered(field, t.surface, theme::RADIUS_SM, 1.0, mix(t.border, t.border_focus, glow));
        icon::draw(p, icon::SEARCH, (field.x + 16.0, field.y + 15.0), 14.0, t.text_faint);
        let mut inner = Rect::new(field.x + 32.0, field.y, field.w - 40.0, field.h);
        // Looking through one chat only: it is named in the box, with a cross to look everywhere again.
        let scope = self.search_in.and_then(|chat| self.store.chat(chat)).map(|chat| chat.title.clone());
        if let Some(name) = scope {
            let mut name = p.layout(&name, theme::CAPTION, None);
            name.truncate(p.fonts, (inner.w * 0.45).max(20.0));
            let chip = Rect::new(inner.x - 4.0, field.y + 5.0, name.width() + 30.0, 20.0);
            p.rect(chip, t.active, 10.0);
            p.text(&name, chip.x + 9.0, chip.y + (chip.h - name.height()) * 0.5, t.text);
            let cross = Rect::new(chip.right() - 20.0, chip.y, 20.0, 20.0);
            let over = ui.hovered(cross);
            icon::draw(p, icon::X, (cross.x + 9.0, cross.y + 10.0), 11.0, if over { t.text } else { t.text_faint });
            if over && ui.clicked(cross) {
                self.search_in = None;
            }
            inner = Rect::new(chip.right() + 8.0, inner.y, inner.right() - chip.right() - 8.0, inner.h);
        }
        let hint = if self.search_in.is_some() { "Search messages" } else { "Search" };
        line_field(p, ui, inner, &mut self.search, (hint, theme::SMALL), searching);
        if searching && matches!(self.overlay, Overlay::None) {
            self.caret = Some(Rect::new(inner.x, inner.y + 6.0, 2.0, 18.0));
        }
        if ui.hovered(field) {
            ui.cursor = CursorIcon::Text;
            if ui.pressed {
                self.focus = Focus::Search;
            }
        }
        if icon_button(p, ui, more, icon::MORE, true) {
            self.overlay = Overlay::Menu(Menu::new(Target::More, (more.x, more.bottom() + 4.0)));
        }
        self.send_search(ui);

        // No connection is said at the foot, where it is out of the way.
        let foot = if self.store.online { 0.0 } else { 26.0 };
        if foot > 0.0 {
            let y = area.bottom() - INSET - 20.0;
            icon::draw(p, icon::WIFI_OFF, (area.x + INSET + 16.0, y + 9.0), 13.0, t.text_faint);
            p.label("Connecting…", theme::TINY, area.x + INSET + 30.0, y, t.text_faint);
        }
        let mut list = Rect::new(area.x, top + 38.0, area.w, area.bottom() - INSET - foot - top - 38.0);
        let pick = if self.searching() {
            self.draw_found(p, ui, list)
        } else if let Some(forum) = self.forum {
            let head = Rect::new(list.x, list.y, list.w, STRIP_H);
            self.draw_forum_head(p, ui, head, forum);
            list = Rect::new(list.x, list.y + STRIP_H + 4.0, list.w, list.h - STRIP_H - 4.0);
            self.draw_topics(p, ui, list, forum)
        } else {
            if self.store.lists.len() > 1 {
                self.draw_strip(p, ui, Rect::new(list.x + INSET, list.y, list.w - 2.0 * INSET, STRIP_H - 6.0));
                list = Rect::new(list.x, list.y + STRIP_H, list.w, list.h - STRIP_H);
            }
            self.draw_rows(p, ui, list)
        };
        match pick {
            Some(Pick::Chat(chat)) => {
                self.end_search();
                self.pick(chat);
            }
            Some(Pick::Topic(chat, topic)) => self.open((chat, topic), None),
            Some(Pick::Message(message)) => self.open((message.chat, message.topic), Some(message.id)),
            Some(Pick::Menu(chat, at)) => self.overlay = Overlay::Menu(Menu::new(Target::Chat(chat), at)),
            None => {}
        }
    }

    /// Sends the search once the typing has paused; an emptied box forgets what was found.
    fn send_search(&mut self, ui: &mut Ui) {
        let wanted = (self.search.text().trim().to_owned(), self.search_in);
        if wanted == self.searched {
            return;
        }
        if !wanted.0.is_empty() && self.now.duration_since(self.search_edited) < crate::SEARCH_AFTER {
            // Still typing: look again in a moment.
            ui.animating = true;
            return;
        }
        self.search_token += 1;
        self.store.search(self.search_token);
        self.list_scroll.reset();
        if !wanted.0.is_empty() {
            let (query, chat) = wanted.clone();
            self.backend.send(Command::Search { query, chat, token: self.search_token });
        }
        self.searched = wanted;
    }

    /// The lists the chats are kept in, as a strip of chips: the shown one filled.
    fn draw_strip(&mut self, p: &mut Painter<'_>, ui: &mut Ui, strip: Rect) {
        let t = p.theme;
        let clip = p.push_clip(strip);
        let over = ui.hovered(strip);
        if over {
            // The wheel moves a strip longer than its room sideways.
            self.strip += ui.scroll + ui.scroll_x;
        }
        let shown = self.store.list();
        let mut x = strip.x - self.strip;
        let mut chosen = None;
        for info in &self.store.lists {
            let name = p.layout(&info.name, theme::SMALL, None);
            let count = (info.unread > 0 && info.list != shown).then(|| label::count(info.unread));
            let count = count.map(|count| p.layout(&count, theme::CAPTION, None));
            let w = name.width() + 22.0 + count.as_ref().map_or(0.0, |count| count.width() + 7.0);
            let pill = Rect::new(x, strip.y, w, strip.h);
            x += w + 4.0;
            let selected = info.list == shown;
            let hovered = over && ui.hovered(pill);
            let hover = ui.anim(id(("list", info.list)), f32::from(u8::from(hovered)));
            p.rect(pill, if selected { t.active } else { fade(t.hover, hover) }, pill.h * 0.5);
            let ink = if selected { t.text } else { mix(t.text_muted, t.text, hover) };
            p.text(&name, pill.x + 11.0, pill.y + (pill.h - name.height()) * 0.5, ink);
            if let Some(count) = count {
                p.text(&count, pill.x + 18.0 + name.width(), pill.y + (pill.h - count.height()) * 0.5, t.text_faint);
            }
            if hovered {
                ui.cursor = CursorIcon::Pointer;
                if ui.clicked(pill) {
                    chosen = Some(info.list);
                }
            }
        }
        // Never moved past its ends.
        let content = x + self.strip - strip.x - 4.0;
        self.strip = self.strip.clamp(0.0, (content - strip.w).max(0.0));
        if self.strip > 0.5 {
            p.gradient_sideways(Rect::new(strip.x, strip.y, 18.0, strip.h), t.panel, fade(t.panel, 0.0));
        }
        if content - self.strip > strip.w + 0.5 {
            p.gradient_sideways(Rect::new(strip.right() - 18.0, strip.y, 18.0, strip.h), fade(t.panel, 0.0), t.panel);
        }
        p.set_clip(clip);
        if let Some(list) = chosen {
            self.show_list(list);
        }
    }

    /// Shows another list of chats, asking for its first chats.
    pub(crate) fn show_list(&mut self, list: List) {
        if list != self.store.list() {
            self.store.show(list);
            self.list_scroll.reset();
            self.asked_chats = (list, usize::MAX);
            self.backend.send(Command::LoadChats(list));
        }
    }

    /// The chats of the shown list, in their order. Returns what was clicked.
    fn draw_rows(&mut self, p: &mut Painter<'_>, ui: &mut Ui, list: Rect) -> Option<Pick> {
        let t = p.theme;
        self.store.refresh();
        let offset = self.list_scroll.update(ui, ui.hovered(list));
        let clip = p.push_clip(list);
        let (store, want) = (&self.store, &mut self.want);
        let files = &store.files;
        let shown = store.list();
        let mut y = list.y - offset;
        let mut pick = None;
        for chat in store.listed().iter().filter_map(|chat| store.chat(*chat)) {
            let row = Rect::new(list.x + INSET, y, list.w - 2.0 * INSET, ROW_H - 2.0);
            y += ROW_H;
            if row.bottom() < list.y || row.y > list.bottom() {
                continue;
            }
            let selected = self.current.is_some_and(|key| key.0 == chat.id);
            let hovered = ui.hovered(row) && list.contains(ui.mouse);
            let hover = ui.anim(id(("chat", chat.id)), f32::from(u8::from(hovered)));
            p.rect(row, if selected { t.active } else { fade(t.hover, hover) }, theme::RADIUS);
            avatar(p, (files, &mut *want), chat.photo, &chat.title, (row.x + 31.0, row.y + row.h * 0.5), 42.0);
            // Words begun here show instead of the latest message, unless the chat is the open one.
            let draft = self.drafts.get(&(chat.id, 0)).map_or(chat.draft.as_str(), String::as_str);
            let about = About {
                typing: store.typing(chat.id, self.now),
                draft: Some(draft).filter(|draft| !draft.is_empty() && !selected),
                pinned: chat.place(shown).is_some_and(|place| place.pinned),
            };
            draw_row(p, row, chat, &about, self.today, if selected { 1.0 } else { hover });
            if hovered {
                ui.cursor = CursorIcon::Pointer;
                if ui.clicked(row) {
                    pick = Some(Pick::Chat(chat.id));
                } else if ui.menu_asked(row) {
                    pick = Some(Pick::Menu(chat.id, ui.mouse));
                }
            }
        }
        let content = y + offset - list.y;
        let listed = store.listed().len();
        if content <= 0.0 {
            let words = if shown == List::Main { "No chats yet" } else { "Nothing in this list" };
            p.label_centered(words, theme::SMALL, Rect::new(list.x, list.y + 24.0, list.w, 20.0), t.text_faint);
        }
        self.finish_list(p, ui, list, content, offset);
        p.set_clip(clip);
        // Near the end of what is loaded: ask for more, once per length.
        if content - offset - list.h < NEAR_END && (shown, listed) != self.asked_chats {
            self.asked_chats = (shown, listed);
            self.backend.send(Command::LoadChats(shown));
        }
        pick
    }

    /// Ends a scrolled list: how far it goes, and the fades at its edges.
    fn finish_list(&mut self, p: &mut Painter<'_>, ui: &mut Ui, list: Rect, content: f32, offset: f32) {
        let t = p.theme;
        self.list_scroll.finish(ui, content, list.h);
        if content > list.h {
            // The list fades out at its edges rather than being cut.
            if offset > 0.5 {
                p.gradient(Rect::new(list.x, list.y, list.w, 16.0), t.panel, fade(t.panel, 0.0), 0.0);
            }
            p.gradient(Rect::new(list.x, list.bottom() - 16.0, list.w, 16.0), fade(t.panel, 0.0), t.panel, 0.0);
        }
    }

    /// The chats a search would open: those whose names hold what is typed,
    /// the most recently active first, then what the backend found.
    fn hits(&self) -> Vec<ChatId> {
        let query = self.search.text().trim().to_lowercase();
        let named = |id: &ChatId| self.store.chat(*id).is_some_and(|chat| chat.title.to_lowercase().contains(&query));
        let mut hits: Vec<ChatId> = self.store.recent().into_iter().filter(named).collect();
        for chat in &self.store.found.chats {
            if !hits.contains(chat) && self.store.chat(*chat).is_some() {
                hits.push(*chat);
            }
        }
        hits
    }

    /// The first chat the search leaves: Enter opens it.
    pub(crate) fn first_hit(&self) -> Option<ChatId> {
        if self.search_in.is_some() {
            return None;
        }
        self.hits().first().copied()
    }

    /// What a search found: chats and people, then messages.
    fn draw_found(&mut self, p: &mut Painter<'_>, ui: &mut Ui, list: Rect) -> Option<Pick> {
        let t = p.theme;
        let hits = if self.search_in.is_some() { Vec::new() } else { self.hits() };
        let offset = self.list_scroll.update(ui, ui.hovered(list));
        let clip = p.push_clip(list);
        let (store, want) = (&self.store, &mut self.want);
        let files = &store.files;
        let mut y = list.y - offset;
        let mut pick = None;
        let heading = |p: &mut Painter<'_>, y: &mut f32, words: &str| {
            p.label(words, theme::CAPTION, list.x + INSET + 10.0, *y + 9.0, t.text_faint);
            *y += HEADING_H;
        };
        if !hits.is_empty() {
            heading(p, &mut y, "Chats");
        }
        for chat in hits.iter().filter_map(|chat| store.chat(*chat)) {
            let row = Rect::new(list.x + INSET, y, list.w - 2.0 * INSET, ROW_H - 2.0);
            y += ROW_H;
            if row.bottom() < list.y || row.y > list.bottom() {
                continue;
            }
            let hovered = ui.hovered(row) && list.contains(ui.mouse);
            let hover = ui.anim(id(("found-chat", chat.id)), f32::from(u8::from(hovered)));
            p.rect(row, fade(t.hover, hover), theme::RADIUS);
            avatar(p, (files, &mut *want), chat.photo, &chat.title, (row.x + 31.0, row.y + row.h * 0.5), 42.0);
            draw_row(p, row, chat, &About::default(), self.today, hover);
            if hovered {
                ui.cursor = CursorIcon::Pointer;
                if ui.clicked(row) {
                    pick = Some(Pick::Chat(chat.id));
                }
            }
        }
        if !store.found.messages.is_empty() {
            heading(p, &mut y, "Messages");
        }
        for message in &store.found.messages {
            let row = Rect::new(list.x + INSET, y, list.w - 2.0 * INSET, ROW_H - 2.0);
            y += ROW_H;
            if row.bottom() < list.y || row.y > list.bottom() {
                continue;
            }
            let Some(chat) = store.chat(message.chat) else { continue };
            let open =
                self.current == Some((message.chat, message.topic)) && self.flash.is_some_and(|f| f.0 == message.id);
            let hovered = ui.hovered(row) && list.contains(ui.mouse);
            let hover = ui.anim(id(("found", message.chat, message.id)), f32::from(u8::from(hovered)));
            p.rect(row, if open { t.active } else { fade(t.hover, hover) }, theme::RADIUS);
            // Inside one chat, who wrote it tells the rows apart; everywhere, the chat does.
            let inside = self.search_in.is_some() && chat.kind != Kind::Private;
            let (name, photo) = if inside && !message.outgoing {
                (message.sender_name.as_str(), message.sender_photo)
            } else if inside {
                ("You", None)
            } else {
                (chat.title.as_str(), chat.photo)
            };
            avatar(p, (files, &mut *want), photo, name, (row.x + 31.0, row.y + row.h * 0.5), 42.0);
            let sender = if inside || chat.kind != Kind::Group {
                String::new()
            } else if message.outgoing {
                "You".to_owned()
            } else {
                message.sender_name.split_whitespace().next().unwrap_or_default().to_owned()
            };
            let found =
                Preview { id: message.id, sender, text: message.body.preview(), date: message.date, outgoing: false };
            draw_lines(p, row, name, None, Some(&found), &Marks::default(), self.today, hover);
            if hovered {
                ui.cursor = CursorIcon::Pointer;
                if ui.clicked(row) {
                    pick = Some(Pick::Message(Box::new(message.clone())));
                }
            }
        }
        let content = y + offset - list.y;
        if content <= 0.0 {
            let waiting = self.searched.0 != self.search.text().trim();
            let words = if waiting { "Searching…" } else { "Nothing found" };
            p.label_centered(words, theme::SMALL, Rect::new(list.x, list.y + 24.0, list.w, 20.0), t.text_faint);
        }
        self.finish_list(p, ui, list, content, offset);
        p.set_clip(clip);
        pick
    }

    /// The line a forum is named on while its topics are listed: a way back, and its name.
    fn draw_forum_head(&mut self, p: &mut Painter<'_>, ui: &mut Ui, head: Rect, forum: ChatId) {
        let t = p.theme;
        if icon_button(p, ui, Rect::new(head.x + INSET, head.y + 2.0, 30.0, 30.0), icon::ARROW_LEFT, true) {
            self.forum = None;
            self.list_scroll.reset();
        }
        let title = self.store.chat(forum).map_or("", |chat| chat.title.as_str());
        let mut title = p.layout(title, theme::LABEL, None);
        title.truncate(p.fonts, head.w - 2.0 * INSET - 46.0);
        p.text(&title, head.x + INSET + 40.0, head.y + (head.h - title.height()) * 0.5, t.text);
    }

    /// The topics of a forum, in their order. Returns what was clicked.
    fn draw_topics(&mut self, p: &mut Painter<'_>, ui: &mut Ui, list: Rect, forum: ChatId) -> Option<Pick> {
        let t = p.theme;
        let offset = self.list_scroll.update(ui, ui.hovered(list));
        let clip = p.push_clip(list);
        let store = &self.store;
        let mut y = list.y - offset;
        let mut pick = None;
        for topic in store.topics(forum) {
            let row = Rect::new(list.x + INSET, y, list.w - 2.0 * INSET, ROW_H - 2.0);
            y += ROW_H;
            if row.bottom() < list.y || row.y > list.bottom() {
                continue;
            }
            let selected = self.current == Some((forum, topic.id));
            let hovered = ui.hovered(row) && list.contains(ui.mouse);
            let hover = ui.anim(id(("topic", forum, topic.id)), f32::from(u8::from(hovered)));
            p.rect(row, if selected { t.active } else { fade(t.hover, hover) }, theme::RADIUS);
            topic_mark(p, topic, (row.x + 31.0, row.y + row.h * 0.5), 42.0);
            let draft = self.drafts.get(&(forum, topic.id)).filter(|_| !selected).map(String::as_str);
            let marks = Marks {
                muted: topic.muted,
                unread: topic.unread,
                mentioned: topic.mentioned,
                pinned: topic.pinned,
                read_outbox: topic.read_outbox,
                draft,
                ..Marks::default()
            };
            let bright = if selected { 1.0 } else { hover };
            draw_lines(p, row, &topic.name, None, topic.last.as_ref(), &marks, self.today, bright);
            if hovered {
                ui.cursor = CursorIcon::Pointer;
                if ui.clicked(row) {
                    pick = Some(Pick::Topic(forum, topic.id));
                }
            }
        }
        let content = y + offset - list.y;
        if content <= 0.0 {
            p.label_centered(
                "Loading topics…",
                theme::SMALL,
                Rect::new(list.x, list.y + 24.0, list.w, 20.0),
                t.text_faint,
            );
        }
        self.finish_list(p, ui, list, content, offset);
        p.set_clip(clip);
        pick
    }
}

/// A topic's mark where a chat has its face: a hash on the topic's colour, a lock when it is closed.
pub(crate) fn topic_mark(p: &mut Painter<'_>, topic: &Topic, center: (f32, f32), size: f32) {
    let t = p.theme;
    let color = if topic.color == 0 { theme::name_color(&topic.name) } else { adelite_ui::paint::hex(topic.color) };
    let rect = Rect::new(center.0 - size * 0.5, center.1 - size * 0.5, size, size);
    p.rect(rect, mix(t.surface, color, 0.22), size * 0.3);
    let glyph = if topic.closed { icon::LOCK } else { icon::HASH };
    // The colour is Telegram's own and may be dark: lifted towards the text so it reads on every scheme.
    icon::draw(p, glyph, center, size * 0.42, mix(color, t.text, 0.35));
}

/// What a chat's row shows besides the chat itself.
#[derive(Default)]
struct About<'a> {
    /// Who is typing there.
    typing: Option<&'a str>,
    /// Words begun there and not sent.
    draft: Option<&'a str>,
    /// It is kept at the top of the shown list.
    pinned: bool,
}

/// The marks on a row: what waits there, and what the row says instead of its latest message.
#[derive(Default)]
struct Marks<'a> {
    muted: bool,
    unread: u32,
    mentioned: bool,
    /// Marked as unread, with nothing to count.
    marked: bool,
    pinned: bool,
    read_outbox: i64,
    typing: Option<String>,
    draft: Option<&'a str>,
}

/// A chat's row beside its face: its name and when it last spoke, then
/// what was said and what is unread.
fn draw_row(p: &mut Painter<'_>, row: Rect, chat: &Chat, about: &About<'_>, today: label::Today, bright: f32) {
    let mark = match chat.kind {
        Kind::Private => None,
        Kind::Group if chat.forum => Some(icon::HASH),
        Kind::Group => Some(icon::USERS),
        Kind::Channel => Some(icon::MEGAPHONE),
    };
    let typing = about
        .typing
        .map(|who| if chat.kind == Kind::Private { "typing…".to_owned() } else { format!("{who} is typing…") });
    let marks = Marks {
        muted: chat.muted,
        unread: chat.unread,
        mentioned: chat.mentioned,
        marked: chat.marked_unread,
        pinned: about.pinned,
        read_outbox: chat.read_outbox,
        typing,
        draft: about.draft,
    };
    draw_lines(p, row, &chat.title, mark, chat.last.as_ref(), &marks, today, bright);
}

/// The two lines of a row: a name with what kind of thing it is before it
/// and a time after it, then the latest words and what waits.
#[expect(clippy::too_many_arguments, reason = "a row is its name, its latest message and its marks")]
fn draw_lines(
    p: &mut Painter<'_>,
    row: Rect,
    name: &str,
    mark: Option<char>,
    last: Option<&Preview>,
    marks: &Marks<'_>,
    today: label::Today,
    bright: f32,
) {
    let t = p.theme;
    let (left, right) = (row.x + 62.0, row.right() - 10.0);
    // Top line, from the right: the time, the ticks of one's own message.
    let mut title_room = right - left;
    if let Some(last) = last {
        let time = p.layout(&label::list_time(last.date, today), theme::TINY, None);
        p.text(&time, right - time.width(), row.y + 12.0, t.text_faint);
        title_room -= time.width() + 8.0;
        if last.outgoing {
            let glyph = if last.id <= marks.read_outbox { icon::CHECK_CHECK } else { icon::CHECK };
            icon::draw(p, glyph, (right - time.width() - 12.0, row.y + 20.0), 13.0, t.text_faint);
            title_room -= 18.0;
        }
    }
    // The name, with what kind of chat it is before it.
    let mut x = left;
    if let Some(mark) = mark {
        icon::draw(p, mark, (x + 7.0, row.y + 20.0), 13.0, t.text_faint);
        x += 19.0;
    }
    let mut title = p.layout(name, theme::LABEL, None);
    title.truncate(p.fonts, title_room - (x - left) - if marks.muted { 18.0 } else { 0.0 });
    p.text(&title, x, row.y + 11.0, mix(t.text_muted, t.text, 0.55 + 0.45 * bright));
    if marks.muted {
        icon::draw(p, icon::BELL_OFF, (x + title.width() + 11.0, row.y + 20.0), 12.0, t.text_faint);
    }

    // Bottom line, from the right: what is unread, or the pin.
    let mut preview_room = right - left;
    if marks.unread > 0 || marks.mentioned || marks.marked {
        let count = match marks.unread {
            0 if marks.mentioned => "1".to_owned(),
            0 => String::new(),
            1..=999 => marks.unread.to_string(),
            _ => "999+".to_owned(),
        };
        let count = p.layout(&count, theme::CAPTION, None);
        let w = (count.width() + 12.0).max(20.0);
        let pill = Rect::new(right - w, row.y + 33.0, w, 20.0);
        let (fill, ink) = if marks.muted { (t.border_strong, t.text_muted) } else { (t.accent, t.on_accent) };
        p.rect(pill, fill, 10.0);
        p.text(&count, pill.x + (pill.w - count.width()) * 0.5, pill.y + (pill.h - count.height()) * 0.5, ink);
        preview_room -= pill.w + 8.0;
        if marks.mentioned {
            let at = Rect::new(pill.x - 24.0, pill.y, 20.0, 20.0);
            p.rect(at, t.accent, 10.0);
            icon::draw(p, icon::AT, (at.x + 10.0, at.y + 10.0), 12.0, t.on_accent);
            preview_room -= 24.0;
        }
    } else if marks.pinned {
        icon::draw(p, icon::PIN, (right - 8.0, row.y + 43.0), 13.0, t.text_faint);
        preview_room -= 22.0;
    }
    let y = row.y + 32.0;
    let say = |p: &mut Painter<'_>, lead: &str, lead_ink, words: &str, ink| {
        let mut x = left;
        if !lead.is_empty() {
            let mut lead = p.layout(lead, theme::SMALL, None);
            lead.truncate(p.fonts, preview_room * 0.5);
            p.text(&lead, x, y, lead_ink);
            x += lead.width() + 4.0;
        }
        let mut words = p.layout(words, theme::SMALL, None);
        words.truncate(p.fonts, preview_room - (x - left));
        p.text(&words, x, y, ink);
    };
    if let Some(typing) = &marks.typing {
        say(p, "", t.text, typing, t.text);
    } else if let Some(draft) = marks.draft {
        // One line of it: a draft may hold several.
        let line: String = draft.chars().take(120).map(|c| if c.is_control() { ' ' } else { c }).collect();
        say(p, "Draft:", t.danger, &line, t.text_faint);
    } else if let Some(last) = last {
        let lead = if last.sender.is_empty() { String::new() } else { format!("{}:", last.sender) };
        say(p, &lead, t.text_muted, &last.text, t.text_faint);
    }
}
