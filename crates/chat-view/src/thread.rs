//! The conversation: who it is with at the top, the message kept there
//! under that, its messages, the message box at the foot.
//!
//! Only the rows on screen are laid out and drawn. Every row's height is
//! kept from when it was last measured (they change only with the column's
//! width or the message itself), so the rows above and below cost an
//! addition each, however long the history is.

use adelite_chat::{Body, ChatId, Command, Delivery, FileId, Kind, MessageId, Page, Press, Style};
use adelite_ui::paint::{Painter, Rect, fade, mix};
use adelite_ui::ui::{Ui, icon, icon_button, id};
use adelite_ui::{Context, platform, theme};
use winit::window::CursorIcon;

use crate::bubble::{self, Over, Scene};
use crate::marked::{self, Marked};
use crate::menu::{Menu, Target};
use crate::store::{Item, Key};
use crate::viewer::Viewer;
use crate::{ChatModule, Focus, Jump, Overlay, avatar, label, list};

/// Space between the canvas's edge and the column of messages.
const MARGIN: f32 = 24.0;
/// The widest the column of messages gets.
const COLUMN_MAX: f32 = 920.0;
/// The widest a bubble gets.
const BUBBLE_MAX: f32 = 540.0;
/// Room beside the bubbles of a group for the faces of who wrote them.
const FACE_ROOM: f32 = 38.0;
/// Height of a day between messages, of a line the chat says itself, and of the mark over what is new.
const DAY_H: f32 = 40.0;
const SERVICE_H: f32 = 30.0;
const NEW_H: f32 = 34.0;
/// Height of the bar that shows the message kept at the chat's top.
const TOP_H: f32 = 40.0;
/// Space over a message that follows one of the same sender, and over any other.
const GAPS: (f32, f32) = (2.0, 10.0);
/// Messages of one sender closer together than this many seconds are a run.
const RUN_SECS: i64 = 300;
/// How close to the first or last loaded message more are asked for.
const NEAR_EDGE: f32 = 500.0;
/// How long a message the user was taken to glows, in seconds.
const FLASH_SECS: f32 = 1.6;
/// How far the mouse goes from where it was pressed before a press on words is a drag.
const DRAG_SLOP: f32 = 3.0;
/// How near the view's edge a drag over words starts scrolling it.
const DRAG_EDGE: f32 = 24.0;
/// How long a conversation's messages may take before the wait is said.
const LOADING_AFTER: std::time::Duration = std::time::Duration::from_millis(400);

/// What a click in the conversation asks for.
enum Act {
    Reply(MessageId),
    /// The menu of a message, at this place; `true`: only what to answer it with.
    Menu(MessageId, (f32, f32), bool),
    /// Pick a message, or let it go.
    Toggle(MessageId),
    /// Styled words were clicked: the span of this message.
    Span(MessageId, usize),
    /// Follow an address: a link's page, a button.
    Follow(String),
    /// Look at the pictures of this row, starting with this one.
    View(MessageId, usize),
    /// Play a video or a recording, fetching it first.
    Play(FileId),
    /// Show a fetched file in its folder.
    Show(String),
    /// Fetch a file.
    Fetch(FileId),
    /// Scroll to this message.
    Jump(MessageId),
    React(MessageId, String, bool),
    Vote(MessageId, usize),
    Press(MessageId, String),
    Comments(MessageId),
    ToEnd,
    Mention,
    Info,
    Search,
    ChatMenu((f32, f32)),
}

/// What stands over a message's bubble, from the one before it.
struct Above {
    /// The day, when it is the first message of one.
    day: bool,
    /// It continues a run of its sender's.
    joined: bool,
    /// It is the first that is new since the chat was read.
    new: bool,
    /// The height of all of it.
    height: f32,
}

fn above(previous: Option<&Item>, item: &Item, new: bool) -> Above {
    let service = |item: &Item| matches!(item.message.body, Body::Service(_));
    let day = previous.is_none_or(|previous| previous.day != item.day);
    let joined = !day
        && !new
        && previous.is_some_and(|previous| {
            previous.message.sender == item.message.sender
                && previous.message.outgoing == item.message.outgoing
                && !service(previous)
                && !service(item)
                && item.message.date - previous.message.date < RUN_SECS
        });
    let gap = if joined { GAPS.0 } else { GAPS.1 };
    Above { day, joined, new, height: gap + if day { DAY_H } else { 0.0 } + if new { NEW_H } else { 0.0 } }
}

impl ChatModule {
    pub(crate) fn draw_thread(&mut self, p: &mut Painter<'_>, ui: &mut Ui, canvas: Rect, cx: &mut Context<'_>) {
        let Some(key) = self.current else { return };
        let t = p.theme;
        let header = Rect::new(canvas.x, canvas.y, canvas.w, theme::HEADER_HEIGHT);
        let kept = self.store.top(key.0).is_some() && key.1 == 0;
        let bar = Rect::new(canvas.x, header.bottom(), canvas.w, if kept { TOP_H } else { 0.0 });
        let compose = self.compose(key);
        let column_w = (canvas.w - 2.0 * MARGIN).clamp(120.0, COLUMN_MAX);
        let column = Rect::new((canvas.x + (canvas.w - column_w) * 0.5).round(), bar.bottom(), column_w, 0.0);
        let (draft, box_h) = self.composer_layout(p, column.w, compose);
        let card = Rect::new(column.x, canvas.bottom() - 14.0 - box_h, column.w, box_h);
        let area = Rect::new(canvas.x, bar.bottom(), canvas.w, card.y - 6.0 - bar.bottom());

        let clip = p.push_clip(area);
        let mut act = self.draw_messages(p, ui, area, (column.x, column.w), key);
        p.set_clip(clip);
        act = self.draw_header(p, ui, header, key).or(act);
        p.rect(Rect::new(header.x, header.bottom() - 1.0, header.w, 1.0), t.border, 0.0);
        if kept {
            act = self.draw_top(p, ui, bar, key.0).or(act);
        }
        self.draw_composer(p, ui, card, draft, compose, cx);
        self.act(act, key, area.h);
    }

    /// Carries out what a click in the conversation asked for.
    fn act(&mut self, act: Option<Act>, key: Key, view: f32) {
        let chat = key.0;
        match act {
            Some(Act::Reply(message)) => {
                (self.reply_to, self.editing, self.focus) = (Some(message), None, Focus::Draft);
            }
            Some(Act::Menu(message, at, only_answers)) => {
                let target = if only_answers { Target::React(message) } else { Target::Message(message) };
                self.overlay = Overlay::Menu(Menu::new(target, at));
            }
            Some(Act::Toggle(message)) => match self.selected.iter().position(|picked| *picked == message) {
                Some(at) => drop(self.selected.remove(at)),
                None => self.selected.push(message),
            },
            Some(Act::Span(message, span)) => {
                let item = self.store.thread(key).and_then(|thread| thread.items.get(thread.find(message)?));
                let found = item.and_then(bubble::words).and_then(|words| {
                    let span = words.spans.get(span)?;
                    Some((span.style.clone(), words.text.get(span.range.clone()).unwrap_or_default().to_owned()))
                });
                match found {
                    Some((Style::Link(to) | Style::Mention(to), _)) => self.backend.send(Command::Follow(to)),
                    Some((Style::Tag, tag)) => {
                        // A tag is something to look for.
                        self.search = adelite_ui::editor::Editor::default();
                        self.search.insert(&tag);
                        self.search_here(false);
                    }
                    Some((Style::Spoiler, _)) => drop(self.revealed.insert(message)),
                    _ => {}
                }
            }
            Some(Act::Follow(to)) => self.backend.send(Command::Follow(to)),
            Some(Act::View(message, at)) => {
                let item = self.store.thread(key).and_then(|thread| thread.items.get(thread.find(message)?));
                if let Some(viewer) = item.and_then(|item| Viewer::of(item, at)) {
                    self.overlay = Overlay::Viewer(viewer);
                }
            }
            Some(Act::Play(file)) => {
                self.playing = Some(file);
                self.want.push(file);
            }
            Some(Act::Show(path)) => {
                if let Some(folder) = std::path::Path::new(&path).parent() {
                    let _ = platform::open_folder(folder);
                }
            }
            // Fetched with what else this frame missed, once.
            Some(Act::Fetch(file)) => self.want.push(file),
            Some(Act::Jump(message)) => self.jump_to(key, message, view),
            Some(Act::React(message, emoji, on)) => self.backend.send(Command::React { chat, message, emoji, on }),
            Some(Act::Vote(message, option)) => {
                self.backend.send(Command::Vote { chat, message, options: vec![option] });
            }
            Some(Act::Press(message, data)) => self.backend.send(Command::Press { chat, message, data }),
            Some(Act::Comments(message)) => self.backend.send(Command::Comments { chat, message }),
            Some(Act::ToEnd) => self.show_end(),
            Some(Act::Mention) => self.backend.send(Command::NextMention(chat)),
            Some(Act::Info) => self.show_info(chat),
            Some(Act::Search) => self.search_here(true),
            Some(Act::ChatMenu(at)) => self.overlay = Overlay::Menu(Menu::new(Target::Chat(chat), at)),
            None => {}
        }
    }

    /// Takes the conversation to the message `message`: by scrolling when
    /// it is loaded, by loading its surroundings when it is not.
    pub(crate) fn jump_to(&mut self, key: Key, message: MessageId, view: f32) {
        let Some(thread) = self.store.thread(key) else { return };
        let Some(at) = thread.find(message) else {
            return self.open(key, Some(message));
        };
        let top: f32 = thread.items.iter().take(at).map(|item| item.height.abs()).sum();
        let height = thread.items.get(at).map_or(0.0, |item| item.height.abs());
        self.scroll.reveal(top - 40.0, top + height + 40.0, view);
        self.flash = Some((message, f32::NAN));
    }

    /// The messages in `area`, in a column at `column` (its left edge and
    /// width). Returns what a click asked for.
    fn draw_messages(
        &mut self,
        p: &mut Painter<'_>,
        ui: &mut Ui,
        area: Rect,
        column: (f32, f32),
        key: Key,
    ) -> Option<Act> {
        let t = p.theme;
        let topic = self.store.topic(key.0, key.1).map(|topic| (topic.read_outbox, topic.mentioned));
        let store = &mut self.store;
        let (files, want) = (&store.files, &mut self.want);
        let chat = store.chats.get(&key.0)?;
        // Who wrote what is named where several write; a channel speaks with one voice.
        let group = chat.kind == Kind::Group || (chat.kind == Kind::Channel && key.1 != 0);
        let read_outbox = chat.read_outbox.max(topic.map_or(0, |topic| topic.0));
        let mentioned = topic.map_or(chat.mentioned, |topic| topic.1);
        let face_room = if group { FACE_ROOM } else { 0.0 };
        let max_w = ((column.1 - face_room) * 0.76).clamp(120.0, BUBBLE_MAX);
        let asides = &store.asides;
        let thread = store.threads.entry(key).or_default();
        let picking = !self.selected.is_empty();

        // Measure what is new or changed. Rows that arrived above what is on
        // screen push it down by their height: the view moves with them.
        if (thread.width - max_w).abs() > 0.5 {
            thread.width = max_w;
            thread.items.iter_mut().for_each(|item| item.height = 0.0);
            thread.prepended = 0;
        }
        let first_new = thread.first_new();
        let mut grown = 0.0;
        for at in 0..thread.items.len() {
            let (before, rest) = thread.items.split_at_mut(at);
            let Some(item) = rest.first_mut().filter(|item| item.height <= 0.0) else { continue };
            let was = item.height.abs();
            let over = above(before.last(), item, first_new == Some(at));
            let own = if matches!(item.message.body, Body::Service(_)) {
                SERVICE_H
            } else {
                let named = group && !item.message.outgoing && !over.joined;
                bubble::lay(p, item, named, max_w, self.revealed.contains(&item.message.id)).h
            };
            item.height = over.height + own;
            if at < thread.prepended {
                grown += item.height;
            } else if at == thread.prepended && thread.prepended > 0 {
                grown += item.height - was;
            }
        }
        if std::mem::take(&mut thread.prepended) > 0 {
            self.scroll.shift(grown);
        }

        let content: f32 = thread.items.iter().map(|item| item.height).sum::<f32>() + 16.0;
        // Where the user asked to be taken, now that the rows there are measured.
        if let Some(jump) = self.jump {
            let target = match jump {
                Jump::Message(message) => thread.find(message),
                Jump::New => first_new,
            };
            if let Some(at) = target {
                let top: f32 = thread.items.iter().take(at).map(|item| item.height).sum();
                let lead = if jump == Jump::New { 6.0 } else { area.h * 0.3 };
                self.scroll.place(top - lead, content, area.h);
                if let Jump::Message(message) = jump {
                    self.flash = Some((message, ui.time));
                }
                (self.jump, self.pin_end, ui.animating) = (None, false, true);
            } else if thread.loaded && !thread.loading {
                // It is not there (deleted, or nothing is new after all): the end is the place to be.
                (self.jump, self.pin_end) = (None, jump == Jump::New);
            }
        }
        // Opened at its end: put there before anything is drawn, so that no frame shows another part of it.
        if self.pin_end && self.jump.is_none() {
            self.scroll.place(content, content, area.h);
        }
        // Words picked with the mouse; gone with their messages. Let go of, the pick stays.
        let mut marking = self.marked.filter(|marked| marked.ordered(thread).is_some());
        // The release that ends a drag over words is not a click on what lies under it.
        let swallow = ui.released && marking.is_some_and(|marked| marked.dragging && !marked.is_empty());
        if let Some(marked) = &mut marking {
            if !ui.down {
                marked.dragging = false;
            } else if !marked.moved {
                let (dx, dy) = (ui.mouse.0 - ui.press_pos.0, ui.mouse.1 - ui.press_pos.1);
                marked.moved = dx.hypot(dy) > DRAG_SLOP;
            }
        }
        let dragging = marking.is_some_and(|marked| marked.dragging && marked.moved);
        if dragging {
            // Held near or past the view's edge: the conversation scrolls on, faster the further out.
            let up = area.y + DRAG_EDGE - ui.mouse.1;
            let down = ui.mouse.1 - (area.bottom() - DRAG_EDGE);
            let past = if up > 0.0 { -up } else { down.max(0.0) };
            if past != 0.0 {
                let speed = 240.0 + past.abs().min(160.0) * 12.0;
                self.scroll.nudge(past.signum() * speed * ui.dt.min(0.05));
                ui.animating = true;
            }
        }
        let order = marking.and_then(|marked| marked.ordered(thread));
        // Where the drag's end is now: in the first words that reach below the mouse, or after the last.
        let mut drag_to: Option<marked::End> = None;
        let mut last_words: Option<marked::End> = None;
        let mut on_words = false;
        let offset = self.scroll.update(ui, ui.hovered(area));
        let at_end = self.scroll.at_end();
        // A conversation shorter than its view sits at the foot of it.
        let mut y = if content < area.h { area.bottom() - content } else { area.y - offset } + 8.0;
        let pointer = area.contains(ui.mouse).then_some(ui.mouse).filter(|_| ui.hovered(area));
        // A glow that was asked for before its row was drawn starts now.
        if let Some((_, since)) = &mut self.flash
            && since.is_nan()
        {
            *since = ui.time;
        }
        let glow = self.flash.map(|(message, since)| (message, 1.0 - (ui.time - since) / FLASH_SECS));
        if glow.is_some_and(|(_, left)| left <= 0.0) {
            self.flash = None;
        } else if glow.is_some() {
            ui.animating = true;
        }
        let mut act = None;
        let mut seen: MessageId = 0;
        let mut missing: Vec<MessageId> = Vec::new();
        // Near the first loaded message: fetch the ones before it.
        if thread.loaded && !thread.loading && !thread.complete && offset < NEAR_EDGE && self.jump.is_none() {
            thread.loading = true;
            let from = thread.oldest();
            self.backend.send(Command::LoadHistory { chat: key.0, topic: key.1, from, page: Page::Older });
        }
        // A chat just opened whose first page does not fill the view has more on its way (a
        // first page may hold one message only): for a moment nothing is shown half-arrived,
        // then all of it at once.
        let settling =
            self.pin_end && thread.loading && content < area.h && self.now.duration_since(self.opened) < LOADING_AFTER;
        ui.animating |= settling;
        let mut previous: Option<&Item> = None;
        let mut items = thread.items.iter().enumerate().peekable();
        while let Some((at, item)) = items.next() {
            if settling {
                break;
            }
            let top = y;
            y += item.height;
            let before = previous.replace(item);
            if y < area.y {
                continue;
            }
            if top > area.bottom() {
                break;
            }
            // On screen, at least its top: the user has seen it.
            if top < area.bottom() - 24.0 {
                seen = seen.max(item.last_id());
            }
            let over = above(before, item, first_new == Some(at));
            let message = &item.message;
            let mut top = top + if over.joined { GAPS.0 } else { GAPS.1 };
            if over.day {
                bubble::pill(
                    p,
                    Rect::new(column.0, top + 2.0, column.1, DAY_H - 6.0),
                    &label::day(item.day, self.today),
                );
                top += DAY_H;
            }
            if over.new {
                // A hairline with its name on it: what is under it has not been read.
                let mid = top + NEW_H * 0.5 - 4.0;
                let words = p.layout("New messages", theme::CAPTION, None);
                let x = column.0 + (column.1 - words.width()) * 0.5;
                p.rect(Rect::new(column.0, mid, x - column.0 - 12.0, 1.0), t.border_strong, 0.0);
                let after = x + words.width() + 12.0;
                p.rect(Rect::new(after, mid, column.0 + column.1 - after, 1.0), t.border_strong, 0.0);
                p.text(&words, x, mid - words.height() * 0.5, t.text_muted);
                top += NEW_H;
            }
            if let Body::Service(words) = &message.body {
                bubble::pill(p, Rect::new(column.0, top, column.1, SERVICE_H), words);
                continue;
            }
            let named = group && !message.outgoing && !over.joined;
            let laid = bubble::lay(p, item, named, max_w, self.revealed.contains(&message.id));
            let x = if message.outgoing { column.0 + column.1 - laid.w } else { column.0 + face_room };
            let rect = Rect::new(x, top, laid.w, laid.h);
            let row = Rect::new(column.0, top, column.1, laid.h);
            // What marks the row: picked by the user, or where they were just taken.
            let picked = picking && self.selected.contains(&message.id);
            let lit = glow.filter(|(lit, _)| item.messages().any(|message| message.id == *lit)).map_or(0.0, |g| g.1);
            if picked || lit > 0.0 {
                let band = Rect::new(row.x - 10.0, row.y - 3.0, row.w + 20.0, row.h + 6.0);
                p.rect(band, fade(t.text, if picked { 0.07 } else { 0.09 * lit.clamp(0.0, 1.0) }), theme::RADIUS);
            }
            // The face of who wrote a run stands beside its last message.
            let ends_run =
                items.peek().is_none_or(|(next, after)| !above(Some(item), after, first_new == Some(*next)).joined);
            if group && !message.outgoing && ends_run {
                let center = (column.0 + 15.0, laid.body((x, top)).bottom() - 15.0);
                avatar(p, (files, &mut *want), message.sender_photo, &message.sender_name, center, 30.0);
            }
            let mouse = pointer.filter(|mouse| row.contains(*mouse));
            let mut part = None;
            if let Some((layout, origin)) = laid.words_at((x, top)) {
                let len = bubble::words(item).map_or(0, |words| words.text.len());
                let local = (ui.mouse.0 - origin.0, ui.mouse.1 - origin.1);
                let hit = || if local.1 < 0.0 { 0 } else { layout.hit(local.0, local.1) };
                if dragging && drag_to.is_none() && origin.1 + layout.height() > ui.mouse.1 {
                    drag_to = Some((message.id, hit()));
                }
                last_words = Some((message.id, len));
                let words = Rect::new(origin.0 - 4.0, origin.1 - 2.0, layout.width() + 8.0, layout.height() + 4.0);
                let over_words = !picking && mouse.is_some() && ui.hovered(words);
                on_words |= over_words;
                if over_words && ui.pressed {
                    // Shift extends what is picked; a double press picks a word, a triple one all of it.
                    marking = Some(match marking {
                        Some(marked) if ui.mods.shift_key() => {
                            Marked { focus: (message.id, hit()), dragging: true, moved: true, ..marked }
                        }
                        _ => match ui.clicks {
                            1 => Marked::new(message.id, hit()..hit()),
                            2 => Marked::new(
                                message.id,
                                bubble::words(item).map_or(0..0, |words| marked::word_at(&words.text, hit())),
                            ),
                            _ => Marked::new(message.id, 0..len),
                        },
                    });
                }
                part = order.and_then(|(from, to)| marked::part(from, to, at, len));
            }
            let replied = message.reply_to.and_then(|reply| {
                let known = thread.message(reply).or_else(|| asides.get(&(key.0, reply)));
                if known.is_none() {
                    missing.push(reply);
                }
                known
            });
            let scene = Scene {
                files: (files, &mut *want),
                fetching: &self.asked_files,
                replied,
                read: message.id <= read_outbox,
                // While messages are picked, a click picks: nothing in a bubble answers it.
                mouse: mouse.filter(|_| !picking && !dragging),
                marked: part,
            };
            let on = bubble::draw(p, &laid, item, (x, top), scene);
            if on != Over::Nothing {
                ui.cursor = CursorIcon::Pointer;
            } else if on_words && mouse.is_some() {
                ui.cursor = CursorIcon::Text;
            }
            if picking {
                let glyph = if picked { icon::CIRCLE_CHECK } else { icon::CIRCLE };
                // Across from the bubble, at the column's edge: the marks line up whatever the bubbles' widths.
                let side = if message.outgoing { column.0 + 10.0 } else { column.0 + column.1 - 10.0 };
                icon::draw(p, glyph, (side, top + 16.0), 16.0, if picked { t.text } else { t.text_faint });
                if mouse.is_some() {
                    ui.cursor = CursorIcon::Pointer;
                    if ui.clicked(row) {
                        act = Some(Act::Toggle(message.id));
                    }
                }
                continue;
            }
            if mouse.is_some() && ui.clicked(rect) && !swallow {
                act = clicked(on, item, files).or(act);
            } else if mouse.is_some() && ui.menu_asked(row) {
                act = Some(Act::Menu(message.id, ui.mouse, false));
            }
            if mouse.is_some() && !dragging {
                // What can be done with it, beside the bubble.
                let sent = message.delivery == Delivery::Sent;
                let tools =
                    [(chat.can_send && sent).then_some(icon::REPLY), sent.then_some(icon::SMILE), Some(icon::MORE)];
                let count = tools.iter().flatten().count() as f32;
                let mut tool_x = if message.outgoing { rect.x - 6.0 - count * 30.0 } else { rect.right() + 6.0 };
                for glyph in tools.into_iter().flatten() {
                    let button = Rect::new(tool_x, top + 1.0, 28.0, 28.0);
                    tool_x += 30.0;
                    if icon_button(p, ui, button, glyph, true) {
                        let under = (button.x, button.bottom() + 4.0);
                        act = Some(match glyph {
                            icon::REPLY => Act::Reply(message.id),
                            icon::SMILE => Act::Menu(message.id, under, true),
                            _ => Act::Menu(message.id, under, false),
                        });
                    }
                }
            }
        }

        // A press anywhere else in the conversation lets the pick go.
        if ui.pressed && pointer.is_some() && !on_words && !ui.mods.shift_key() {
            marking = None;
        }
        if let Some(marked) = &mut marking
            && dragging
            && let Some(to) = drag_to.or(last_words)
            && marked.focus != to
        {
            // Drawn with last frame's end: one more frame shows where it is now.
            marked.focus = to;
            ui.animating = true;
        }
        if dragging {
            ui.cursor = CursorIcon::Text;
        }
        // A press that was let go of without picking anything was a click.
        self.marked = marking.filter(|marked| marked.dragging || !marked.is_empty());

        if thread.items.is_empty() {
            // Its messages are a moment away, mostly: only a wait worth the name is called one.
            let waited = self.now.duration_since(self.opened) >= LOADING_AFTER;
            let words = match (thread.loaded, waited) {
                (true, _) => "No messages here yet",
                (false, true) => "Loading…",
                (false, false) => "",
            };
            ui.animating |= !thread.loaded && !waited;
            p.label_centered(
                words,
                theme::SMALL,
                Rect::new(area.x, area.y + area.h * 0.5 - 10.0, area.w, 20.0),
                t.text_faint,
            );
        }
        if offset > 0.5 && content > area.h {
            p.gradient(Rect::new(area.x, area.y, area.w, 14.0), t.bg, fade(t.bg, 0.0), 0.0);
        }
        self.scroll.finish(ui, content, area.h);
        if self.pin_end {
            // Kept at its end until what was asked for is there: a first page may hold one
            // message only, and the rest comes in over it.
            self.scroll.jump_to_end();
            self.pin_end = !thread.loaded || (thread.loading && content < area.h);
        } else if at_end && thread.live && self.jump.is_none() {
            self.scroll.follow_end();
        }
        // Away from the latest message: a way back down, wearing what is unread; over it, a way to where the user is mentioned.
        let away = !self.scroll.at_end() || !thread.live;
        if away && thread.loaded {
            let button = Rect::new(area.right() - MARGIN - 38.0, area.bottom() - 46.0, 38.0, 38.0);
            if round_button(p, ui, button, icon::ARROW_DOWN, chat.unread) {
                act = Some(Act::ToEnd);
            }
        }
        if mentioned {
            let lift = if away { 46.0 } else { 0.0 };
            let button = Rect::new(area.right() - MARGIN - 38.0, area.bottom() - 46.0 - lift, 38.0, 38.0);
            if round_button(p, ui, button, icon::AT, 0) {
                act = Some(Act::Mention);
            }
        }

        // Near the last loaded message, where the conversation's end is not loaded: fetch the ones after it.
        let (chat, topic) = key;
        let below = content - offset - area.h;
        if thread.loaded && !thread.live && !thread.loading_newer && below < NEAR_EDGE && self.jump.is_none() {
            thread.loading_newer = true;
            self.backend.send(Command::LoadHistory { chat, topic, from: thread.newest(), page: Page::Newer });
        }
        // With the window in front, what was on screen has been seen.
        if ui.focused && seen > 0 {
            let messages = thread.take_unseen(seen);
            if !messages.is_empty() {
                self.backend.send(Command::Read { chat, messages });
            }
        }
        // What is answered and not loaded is asked for, once, so the answer can show it.
        missing.retain(|message| self.asked_asides.insert((chat, *message)));
        if !missing.is_empty() {
            self.backend.send(Command::Fetch { chat, messages: missing });
        }
        act
    }

    /// Who the conversation is with: their face, name, and the line under
    /// it; at its right, a search through it and its menu. A click on the
    /// name opens the chat's panel.
    fn draw_header(&mut self, p: &mut Painter<'_>, ui: &mut Ui, header: Rect, key: Key) -> Option<Act> {
        let t = p.theme;
        let chat = self.store.chats.get(&key.0)?;
        let topic = self.store.topic(key.0, key.1);
        let typing = self.store.typing(key.0, self.now);
        let center = (header.x + MARGIN + 17.0, header.y + header.h * 0.5);
        // A topic is named, with the chat it is in under it; comments are called that.
        let (title, under) = match (topic, key.1) {
            (Some(topic), _) => {
                list::topic_mark(p, topic, center, 34.0);
                (topic.name.as_str(), Some(chat.title.clone()))
            }
            (None, 0) => {
                avatar(p, (&self.store.files, &mut self.want), chat.photo, &chat.title, center, 34.0);
                (chat.title.as_str(), None)
            }
            (None, _) => {
                p.rect(Rect::new(center.0 - 17.0, center.1 - 17.0, 34.0, 34.0), t.hover, 17.0);
                icon::draw(p, icon::COMMENT, center, 16.0, t.text_muted);
                ("Comments", Some(chat.title.clone()))
            }
        };
        let x = header.x + MARGIN + 46.0;
        let status = match (typing, chat.kind) {
            (Some(_), Kind::Private) => "typing…".to_owned(),
            (Some(who), _) => format!("{who} is typing…"),
            (None, _) if under.is_some() => under.unwrap_or_default(),
            (None, _) if !chat.status.is_empty() => chat.status.clone(),
            (None, _) if chat.last_seen > 0 => label::last_seen(chat.last_seen, self.today),
            (None, Kind::Private) => String::new(),
            (None, Kind::Group) => "group".to_owned(),
            (None, Kind::Channel) => "channel".to_owned(),
        };
        let tools = header.right() - MARGIN - 64.0;
        let mut title = p.layout(title, theme::HEADING, None);
        title.truncate(p.fonts, tools - 12.0 - x);
        if status.is_empty() {
            p.text(&title, x, header.y + (header.h - title.height()) * 0.5, t.text);
        } else {
            p.text(&title, x, header.y + 8.0, t.text);
            let ink = if typing.is_some() || status == "online" { t.text_muted } else { t.text_faint };
            let mut status = p.layout(&status, theme::TINY, None);
            status.truncate(p.fonts, tools - 12.0 - x);
            p.text(&status, x, header.y + 29.0, ink);
        }
        let mut act = None;
        let name =
            Rect::new(header.x + MARGIN - 6.0, header.y + 6.0, x + title.width() + 12.0 - header.x - MARGIN, 40.0);
        if ui.hovered(name) {
            ui.cursor = CursorIcon::Pointer;
            if ui.clicked(name) {
                act = Some(Act::Info);
            }
        }
        if icon_button(p, ui, Rect::new(tools, header.y + 11.0, 30.0, 30.0), icon::SEARCH, true) {
            act = Some(Act::Search);
        }
        let more = Rect::new(tools + 34.0, header.y + 11.0, 30.0, 30.0);
        if icon_button(p, ui, more, icon::MORE, true) {
            act = Some(Act::ChatMenu((more.right() - 176.0, more.bottom() + 4.0)));
        }
        act
    }

    /// The message kept at the chat's top, as a bar under its name: a click goes to it.
    fn draw_top(&mut self, p: &mut Painter<'_>, ui: &mut Ui, bar: Rect, chat: ChatId) -> Option<Act> {
        let t = p.theme;
        let top = self.store.top(chat)?;
        let hovered = ui.hovered(bar);
        let hover = ui.anim(id("kept"), f32::from(u8::from(hovered)));
        p.rect(bar, mix(t.bg, t.hover, hover * 0.6), 0.0);
        p.rect(Rect::new(bar.x, bar.bottom() - 1.0, bar.w, 1.0), t.border, 0.0);
        let x = bar.x + MARGIN;
        icon::draw(p, icon::PIN, (x + 17.0, bar.y + bar.h * 0.5), 14.0, t.text_muted);
        p.label("Pinned", theme::CAPTION, x + 46.0, bar.y + 5.0, t.text);
        let mut words = p.layout(&top.body.preview(), theme::TINY, None);
        words.truncate(p.fonts, bar.w - 2.0 * MARGIN - 56.0);
        p.text(&words, x + 46.0, bar.y + 21.0, t.text_muted);
        if hovered {
            ui.cursor = CursorIcon::Pointer;
        }
        ui.clicked(bar).then_some(Act::Jump(top.id))
    }
}

/// What a click on `on` of the row `item` asks for.
fn clicked(on: Over, item: &Item, files: &std::collections::HashMap<FileId, String>) -> Option<Act> {
    let message = &item.message;
    let fetched = |file: &Option<FileId>| file.and_then(|file| files.get(&file));
    match on {
        Over::Span(span) => Some(Act::Span(message.id, span)),
        Over::Reply => message.reply_to.map(Act::Jump),
        Over::Preview => message.link.as_ref().map(|link| Act::Follow(link.url.clone())),
        Over::Reaction(at) => {
            let reaction = message.reactions.get(at)?;
            Some(Act::React(message.id, reaction.emoji.clone(), !reaction.chosen))
        }
        Over::Answer(option) => Some(Act::Vote(message.id, option)),
        Over::Comments => Some(Act::Comments(message.id)),
        Over::Button(row, at) => match &message.buttons.get(row)?.get(at)?.press {
            Press::Link(to) => Some(Act::Follow(to.clone())),
            Press::Data(data) => Some(Act::Press(message.id, data.clone())),
            Press::Nothing => None,
        },
        Over::Media(at) => match &item.messages().nth(at)?.body {
            Body::Photo { .. } => Some(Act::View(message.id, at)),
            Body::Video { file, .. } | Body::Audio { file, .. } => (*file).map(Act::Play),
            Body::File { file, .. } => match fetched(file) {
                Some(path) => Some(Act::Show(path.clone())),
                None => (*file).map(Act::Fetch),
            },
            _ => None,
        },
        Over::Nothing => None,
    }
}

/// A round button floating over the conversation's corner; with `count`, it wears it. Returns whether it was clicked.
fn round_button(p: &mut Painter<'_>, ui: &mut Ui, button: Rect, glyph: char, count: u32) -> bool {
    let t = p.theme;
    let hovered = ui.hovered(button);
    let hover = ui.anim(id(("round", glyph)), f32::from(u8::from(hovered)));
    p.shadow(Rect::new(button.x, button.y + 3.0, button.w, button.h), fade(t.shadow, 0.5), 19.0, 12.0);
    p.bordered(button, mix(t.surface, t.hover, hover), 19.0, 1.0, t.border_strong);
    icon::draw(p, glyph, (button.x + 19.0, button.y + 19.0), 16.0, t.text);
    if count > 0 {
        let count = p.layout(&count.min(999).to_string(), theme::CAPTION, None);
        let w = (count.width() + 10.0).max(18.0);
        let pill = Rect::new(button.right() - w + 4.0, button.y - 7.0, w, 18.0);
        p.rect(pill, t.accent, 9.0);
        p.text(&count, pill.x + (pill.w - count.width()) * 0.5, pill.y + (pill.h - count.height()) * 0.5, t.on_accent);
    }
    if hovered {
        ui.cursor = CursorIcon::Pointer;
    }
    hovered && ui.clicked(button)
}
