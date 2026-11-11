//! Adelite's chat app: the list of chats beside the conversation, its
//! message box under it, and the card that signs the user in.
//!
//! [`ChatModule`] is a [`Module`] for the shell's rail and knows no
//! network: it draws what a [`Backend`] tells it and tells the backend what
//! the user does. Swapping Telegram for another network is handing
//! [`ChatModule::new`] another backend.
//!
//! Everything is drawn on demand. A conversation lays out only the
//! messages on screen; the heights of the others are kept from when they
//! were last measured, so a long history scrolls as cheaply as a short one.
//!
//! What floats over the page (a menu, a chat's card, the chat picker, a
//! picture looked at alone) is one [`Overlay`] at a time: while it is
//! there, the page under it does not answer the mouse.

mod bubble;
mod composer;
#[cfg(any(test, feature = "demo"))]
pub mod demo;
mod info;
mod label;
mod list;
mod marked;
mod menu;
mod picker;
mod session;
#[cfg(test)]
mod snapshot;
mod store;
mod thread;
mod viewer;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use adelite_chat::{Backend, Body, ChatId, Command, Event, FileId, List, MessageId, Page, Prompt, Session};
use adelite_ui::editor::Editor;
use adelite_ui::form::Fields;
use adelite_ui::image::Lookup;
use adelite_ui::paint::{Painter, Rect, mix};
use adelite_ui::text::TextLayout;
use adelite_ui::ui::{self, Ui, edit_key, icon, move_line};
use adelite_ui::widgets::Scroll;
use adelite_ui::{Context, Module, Request, Sound, Waker, platform, theme};
use arboard::Clipboard;
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use label::Today;
use menu::Menu;
use picker::Picker;
use store::{Effect, Key as Convo, Store};
use viewer::Viewer;

/// How often the other side is told the user is typing.
const TYPING_EVERY: Duration = Duration::from_secs(4);
/// How long after the last key a search is sent.
const SEARCH_AFTER: Duration = Duration::from_millis(280);

/// Which box typing goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The message box.
    Draft,
    /// The search over the chat list.
    Search,
}

/// Where the conversation is to go once its messages are there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Jump {
    /// To this message.
    Message(MessageId),
    /// To the first message that is new since the chat was read.
    New,
}

/// What floats over the page.
enum Overlay {
    /// Nothing.
    None,
    /// A menu.
    Menu(Menu),
    /// A chat's card: what it is, who is in it, what was sent in it.
    Info(info::Card),
    /// The card to pick chats or people on.
    Picker(Picker),
    /// A picture, looked at alone.
    Viewer(Viewer),
}

/// The chat app.
pub struct ChatModule {
    backend: Box<dyn Backend>,
    events: Receiver<Event>,
    waker: Waker,
    store: Store,
    /// The conversation on the canvas.
    current: Option<Convo>,
    /// The forum whose topics the sidebar shows instead of the chats.
    forum: Option<ChatId>,
    focus: Focus,
    /// What "now" is for this frame's labels.
    today: Today,
    now: Instant,

    list_scroll: Scroll,
    /// How far the strip of lists is moved sideways.
    strip: f32,
    search: Editor,
    /// The chat whose messages the search looks through; everywhere when none.
    search_in: Option<ChatId>,
    /// The search as last sent, when what is typed last changed, and the running search's token.
    searched: (String, Option<ChatId>),
    search_edited: Instant,
    search_token: u64,
    /// The list more chats were last asked for, and how many it had then.
    asked_chats: (List, usize),

    scroll: Scroll,
    /// Keep the conversation at its end until its first page is drawn.
    pin_end: bool,
    /// Where the conversation goes once the messages are loaded.
    jump: Option<Jump>,
    /// The message the user was just taken to, and when ([`Ui::time`]): it glows for a moment.
    flash: Option<(MessageId, f32)>,
    /// The messages picked to forward, copy or delete together.
    selected: Vec<MessageId>,
    /// Words picked out of the messages with the mouse.
    marked: Option<marked::Marked>,
    /// The video or recording that plays once it is fetched.
    playing: Option<FileId>,
    /// The messages whose hidden words the user asked to see.
    revealed: HashSet<MessageId>,
    /// Answered messages that were asked of the backend, to show what a reply answers.
    asked_asides: HashSet<(ChatId, MessageId)>,

    draft: Editor,
    /// The draft as last drawn and where, for the arrow keys.
    draft_view: Option<(TextLayout, (f32, f32))>,
    /// The message the draft answers.
    reply_to: Option<MessageId>,
    /// The message the draft replaces the words of.
    editing: Option<MessageId>,
    /// Files that go along with the draft.
    attached: Vec<String>,
    /// Words begun in other conversations.
    drafts: HashMap<Convo, String>,
    /// When the other side was last told the user is typing.
    typed: Option<Instant>,
    /// The focused box's caret, for the input method's window.
    caret: Option<Rect>,

    /// What floats over the page.
    overlay: Overlay,
    /// It was drawn already: the click that opened it is over.
    overlay_shown: bool,
    /// A chat's card that waits behind what was opened from it (a picture,
    /// the card people are picked on): it comes back when that closes.
    behind: Option<info::Card>,
    /// When the conversation on the canvas was opened.
    opened: Instant,

    /// The sign-in fields, and the prompt they were made for.
    fields: Fields,
    asked: Prompt,
    /// An answer was sent and the backend has not said what came of it.
    waiting: bool,

    /// Files asked of the backend, and those this frame found missing.
    asked_files: HashSet<FileId>,
    want: Vec<FileId>,
    /// Files the picker's thread (or a paste) hands back.
    picked: (Sender<Vec<String>>, Receiver<Vec<String>>),
}

impl ChatModule {
    /// A chat app over `backend`, whose events arrive on `events`. `waker`
    /// wakes the interface from the threads the app starts.
    #[must_use]
    pub fn new(backend: Box<dyn Backend>, events: Receiver<Event>, waker: Waker) -> Self {
        let now = Instant::now();
        Self {
            backend,
            events,
            waker,
            store: Store::default(),
            current: None,
            forum: None,
            focus: Focus::Draft,
            today: Today::default(),
            now,
            list_scroll: Scroll::default(),
            strip: 0.0,
            search: Editor::default(),
            search_in: None,
            searched: (String::new(), None),
            search_edited: now,
            search_token: 0,
            asked_chats: (List::Main, 0),
            scroll: Scroll::default(),
            pin_end: false,
            jump: None,
            flash: None,
            selected: Vec::new(),
            marked: None,
            playing: None,
            revealed: HashSet::new(),
            asked_asides: HashSet::new(),
            draft: Editor::default(),
            draft_view: None,
            reply_to: None,
            editing: None,
            attached: Vec::new(),
            drafts: HashMap::new(),
            typed: None,
            caret: None,
            overlay: Overlay::None,
            overlay_shown: false,
            behind: None,
            opened: now,
            fields: Fields::lines(0),
            asked: Prompt::default(),
            waiting: false,
            asked_files: HashSet::new(),
            want: Vec::new(),
            picked: mpsc::channel(),
        }
    }

    /// Takes in one event of the backend's.
    fn apply(&mut self, event: Event, cx: &mut Context<'_>) {
        self.store.clock = platform::unix_now() as i64;
        match self.store.apply(event, Instant::now()) {
            Effect::None => {}
            Effect::News { chat, title, body } => {
                cx.requests.push(Request::Attention);
                cx.requests.push(Request::Notify { title, body });
                // In the conversation on screen it is seen arriving: heard only when the window is elsewhere.
                let in_view = self.current.is_some_and(|key| key.0 == chat);
                cx.requests.push(Request::Sound { sound: Sound::Arrived, in_view });
            }
            Effect::Sent => cx.requests.push(Request::Sound { sound: Sound::Sent, in_view: false }),
            Effect::Problem(text) => cx.requests.push(Request::Notice(text)),
            Effect::Notice(text) => cx.requests.push(Request::Info(text)),
            Effect::Goto(key, message) => self.goto(key, message),
            Effect::Browse(url) => browse(&url),
            Effect::Session => {
                self.waiting = false;
                let prompt = match &self.store.session {
                    Session::Asking(prompt) => prompt.clone(),
                    _ => Prompt::default(),
                };
                // The same step asked again (the answer was refused) keeps what was typed.
                if prompt.fields != self.asked.fields || prompt.title != self.asked.title {
                    self.fields = Fields::lines(prompt.fields.len());
                    if let Some(secret) = prompt.fields.iter().position(|field| field.secret) {
                        self.fields = std::mem::replace(&mut self.fields, Fields::lines(0)).secret(secret);
                    }
                }
                self.asked = prompt;
                if self.store.session != Session::Ready {
                    // Nothing of the account stays: not its drafts, nor what was open over it.
                    self.drafts.clear();
                    self.draft = Editor::default();
                    self.leave();
                    (self.forum, self.behind, self.overlay) = (None, None, Overlay::None);
                    self.search = Editor::default();
                    self.search_in = None;
                    self.asked_files.clear();
                    self.asked_asides.clear();
                    self.asked_chats = (List::Main, 0);
                }
            }
        }
    }

    /// The chat `id` was picked in the list: a forum shows its topics, any
    /// other chat its conversation.
    fn pick(&mut self, id: ChatId) {
        if self.store.chat(id).is_some_and(|chat| chat.forum) {
            self.forum = Some(id);
            self.list_scroll = Scroll::default();
            self.backend.send(Command::LoadTopics(id));
        } else {
            self.open((id, 0), None);
        }
    }

    /// Shows a conversation on the canvas; with `at`, at that message.
    /// Without, where the user left off: the first unread message, or the end.
    fn open(&mut self, key: Convo, at: Option<MessageId>) {
        if self.current != Some(key) {
            self.leave();
            self.current = Some(key);
            self.backend.send(Command::Open(key.0));
            let synced = self.store.chat(key.0).filter(|_| key.1 == 0).map(|chat| chat.draft.clone());
            if let Some(words) = self.drafts.get(&key).cloned().or(synced).filter(|words| !words.is_empty()) {
                self.draft.insert(&words);
            }
            self.scroll = Scroll::default();
            self.focus = Focus::Draft;
            self.opened = Instant::now();
            // Another chat than the forum whose topics are listed: the list is the chats again.
            if self.forum.is_some_and(|forum| forum != key.0) {
                self.forum = None;
            }
        }
        // What was read before the chat was opened; the messages after it are new.
        let (unread, read) = match self.store.topic(key.0, key.1) {
            Some(topic) => (topic.unread, topic.read_inbox),
            None if key.1 == 0 => self.store.chat(key.0).map_or((0, 0), |chat| (chat.unread, chat.read_inbox)),
            // The comments under a post: nothing says how far they were read.
            None => (0, 0),
        };
        let thread = self.store.thread_mut(key);
        if at.is_none() {
            thread.mark(if unread > 0 { read } else { 0 });
        }
        let target = at.or((thread.divider > 0 && unread > 0).then_some(thread.divider));
        let loaded = thread.loaded || thread.loading;
        let here = target.is_some_and(|message| thread.find(message).is_some());
        (self.jump, self.pin_end) = match (at, target) {
            (Some(message), _) => (Some(Jump::Message(message)), false),
            (None, Some(_)) => (Some(Jump::New), false),
            (None, None) => (None, true),
        };
        if let Some(from) = target.filter(|_| !here) {
            thread.loading = true;
            self.backend.send(Command::LoadHistory { chat: key.0, topic: key.1, from, page: Page::Around });
        } else if !loaded {
            thread.loading = true;
            self.backend.send(Command::LoadHistory { chat: key.0, topic: key.1, from: 0, page: Page::Older });
        }
    }

    /// Shows a conversation the backend points at ([`Event::Goto`]).
    fn goto(&mut self, key: Convo, message: MessageId) {
        (self.behind, self.overlay) = (None, Overlay::None);
        if key.1 == 0 && message == 0 {
            self.pick(key.0);
        } else {
            self.open(key, (message != 0).then_some(message));
        }
    }

    /// Takes the open conversation off the canvas, keeping what was typed for when it is opened again.
    fn leave(&mut self) {
        if let Some(key) = self.current.take() {
            let words = if self.editing.is_some() { String::new() } else { self.draft.text().trim().to_owned() };
            let synced = self.store.chat(key.0).filter(|_| key.1 == 0).map_or("", |chat| chat.draft.as_str());
            if words != self.drafts.get(&key).map_or(synced, String::as_str) {
                self.backend.send(Command::Draft { chat: key.0, topic: key.1, text: words.clone() });
            }
            if words.is_empty() {
                self.drafts.remove(&key);
            } else {
                self.drafts.insert(key, words);
            }
            self.backend.send(Command::Close(key.0));
            self.store.leave(key);
        }
        self.draft = Editor::default();
        (self.reply_to, self.editing, self.jump, self.flash, self.typed) = (None, None, None, None, None);
        self.attached.clear();
        self.selected.clear();
        self.marked = None;
        self.revealed.clear();
        self.playing = None;
    }

    /// Brings the open conversation to its latest message: at once when it
    /// is loaded up to there, by loading its end afresh when it is not.
    fn show_end(&mut self) {
        let Some(key) = self.current else { return };
        match self.store.thread(key) {
            Some(thread) if thread.live => return self.scroll.follow_end(),
            // Its first page is on its way: the end is where to be when it comes.
            Some(thread) if !thread.loaded => return (self.jump, self.pin_end) = (None, true),
            _ => {}
        }
        self.store.threads.remove(&key);
        self.store.thread_mut(key).loading = true;
        self.backend.send(Command::LoadHistory { chat: key.0, topic: key.1, from: 0, page: Page::Older });
        (self.jump, self.pin_end) = (None, true);
    }

    /// Sends what the message box holds.
    fn submit(&mut self) {
        let Some((chat, topic)) = self.current else { return };
        let text = self.draft.text().trim().to_owned();
        if let Some(message) = self.editing.take() {
            let caption = self.store.message((chat, topic), message).is_some_and(|m| !matches!(m.body, Body::Text(_)));
            if !text.is_empty() || caption {
                self.backend.send(Command::Edit { chat, message, text, caption });
            }
        } else if !self.attached.is_empty() {
            let paths = std::mem::take(&mut self.attached);
            self.backend.send(Command::SendFiles { chat, topic, paths, caption: text, reply_to: self.reply_to });
        } else if !text.is_empty() {
            self.backend.send(Command::Send { chat, topic, text, reply_to: self.reply_to });
        } else {
            return;
        }
        self.draft = Editor::default();
        (self.reply_to, self.typed) = (None, None);
        self.drafts.remove(&(chat, topic));
        self.show_end();
    }

    /// Starts editing the message `id` of the open conversation in the message box.
    fn edit(&mut self, id: MessageId) {
        let Some(text) = self.current.and_then(|key| self.store.message(key, id)).map(|m| m.body.text().to_owned())
        else {
            return;
        };
        self.draft = Editor::default();
        self.draft.insert(&text);
        (self.editing, self.reply_to, self.focus) = (Some(id), None, Focus::Draft);
    }

    /// Opens the file picker on a thread of its own; what is chosen comes back through `poll`.
    fn pick_files(&self) {
        self.on_thread("adelite-picker", || platform::pick_any("Choose files to send").unwrap_or_default());
    }

    /// Attaches what the clipboard holds that is not words: copied files, or a copied picture.
    fn paste_files(&self) {
        let picture = std::env::temp_dir().join(format!("adelite-paste-{}.png", platform::unix_now()));
        self.on_thread("adelite-paste", move || platform::clipboard_files(&picture).unwrap_or_default());
    }

    /// Runs `find` on a thread of its own and attaches the files it comes back with.
    fn on_thread(&self, name: &str, find: impl FnOnce() -> Vec<PathBuf> + Send + 'static) {
        let (picked, waker) = (self.picked.0.clone(), self.waker.clone());
        let _ = std::thread::Builder::new().name(name.into()).spawn(move || {
            let files: Vec<String> = find().iter().map(|path| path.display().to_string()).collect();
            if !files.is_empty() {
                let _ = picked.send(files);
                waker();
            }
        });
    }

    /// Looks through the open chat's messages, or everywhere when no chat is open.
    fn search_here(&mut self, here: bool) {
        self.search_in = self.current.map(|key| key.0).filter(|_| here);
        self.focus = Focus::Search;
        self.search_edited = self.now;
    }

    /// Leaves the search: the list is the chats again.
    fn end_search(&mut self) {
        self.search = Editor::default();
        self.search_in = None;
        self.focus = Focus::Draft;
    }

    /// Whether the sidebar shows what a search found rather than the chats.
    fn searching(&self) -> bool {
        !self.search.text().trim().is_empty()
    }

    /// The editor typing goes to.
    fn editor(&mut self) -> &mut Editor {
        match (&mut self.overlay, self.focus) {
            (Overlay::Picker(picker), _) => picker.editor(),
            (_, Focus::Search) => &mut self.search,
            (_, Focus::Draft) if self.current.is_none() => &mut self.search,
            (_, Focus::Draft) => &mut self.draft,
        }
    }

    /// A key while signed in.
    fn chat_key(&mut self, event: &KeyEvent, mods: ModifiersState, cx: &mut Context<'_>) {
        let primary = if cfg!(target_os = "macos") { mods.super_key() } else { mods.control_key() };
        if !matches!(self.overlay, Overlay::None) {
            return self.overlay_key(event, mods, cx);
        }
        let searching = self.focus == Focus::Search || self.current.is_none();
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.escape(),
            // An emptied search through one chat looks everywhere again.
            Key::Named(NamedKey::Backspace)
                if self.focus == Focus::Search && self.search.text().is_empty() && self.search_in.is_some() =>
            {
                self.search_in = None;
            }
            // Words picked in the messages are copied, unless the box being typed in has its own.
            Key::Character(c)
                if primary
                    && c.eq_ignore_ascii_case("c")
                    && self.editor().selection().is_empty()
                    && self.marked_text().is_some() =>
            {
                ui::copy(cx.clipboard, &self.marked_text().unwrap_or_default());
            }
            Key::Character(c) if primary && c.eq_ignore_ascii_case("k") => self.search_here(false),
            Key::Character(c) if primary && c.eq_ignore_ascii_case("f") => self.search_here(true),
            Key::Named(NamedKey::Enter) if searching => {
                if let Some(id) = self.first_hit() {
                    self.end_search();
                    self.pick(id);
                }
            }
            Key::Named(NamedKey::Enter) if mods.shift_key() => self.draft.insert("\n"),
            Key::Named(NamedKey::Enter) => self.submit(),
            Key::Named(NamedKey::ArrowUp) if !searching && self.draft.text().is_empty() => {
                if let Some(id) = self.current.and_then(|key| self.store.last_own_text(key)).map(|m| m.id) {
                    self.edit(id);
                }
            }
            Key::Named(key @ (NamedKey::ArrowUp | NamedKey::ArrowDown)) if !searching => {
                if let Some((layout, _)) = &self.draft_view {
                    move_line(&mut self.draft, layout, *key == NamedKey::ArrowUp, mods.shift_key());
                }
            }
            Key::Named(NamedKey::PageUp) => self.scroll.nudge(-400.0),
            Key::Named(NamedKey::PageDown) => self.scroll.nudge(400.0),
            // A paste that is not words: files copied elsewhere, or a picture.
            Key::Character(c) if primary && c.eq_ignore_ascii_case("v") && !searching && !has_text(cx.clipboard) => {
                self.paste_files();
            }
            _ => {
                let before = self.editor().text().len();
                edit_key(self.editor(), event, mods, cx.clipboard);
                if searching {
                    self.search_edited = self.now;
                } else if self.draft.text().len() != before {
                    self.typing();
                }
            }
        }
    }

    /// Escape takes back the nearest thing: what is picked, what is being
    /// answered or edited, the search, the forum's topics.
    fn escape(&mut self) {
        if self.marked.take().is_some() {
        } else if !self.selected.is_empty() {
            self.selected.clear();
        } else if self.editing.take().is_some() {
            self.draft = Editor::default();
        } else if self.reply_to.take().is_some() {
        } else if self.searching() || self.focus == Focus::Search || self.search_in.is_some() {
            self.end_search();
        } else {
            self.forum = None;
        }
    }

    /// The words picked out of the open conversation's messages, if any are.
    fn marked_text(&self) -> Option<String> {
        let thread = self.store.thread(self.current?)?;
        let (from, to) = self.marked?.ordered(thread)?;
        Some(marked::text(thread, from, to)).filter(|text| !text.is_empty())
    }

    /// Closes what floats over the page. A chat's card that waited behind it comes back.
    fn close_overlay(&mut self) {
        self.overlay = match self.behind.take() {
            Some(card) => Overlay::Info(card),
            None => Overlay::None,
        };
    }

    /// Tells the other side the user is typing, now and then.
    fn typing(&mut self) {
        let Some((chat, topic)) = self.current else { return };
        let due = self.typed.is_none_or(|at| self.now.duration_since(at) >= TYPING_EVERY);
        if due && self.editing.is_none() && !self.draft.text().is_empty() {
            self.typed = Some(self.now);
            self.backend.send(Command::Typing { chat, topic });
        }
    }

    /// A key while the sign-in card is shown.
    fn session_key(&mut self, event: &KeyEvent, mods: ModifiersState, cx: &mut Context<'_>) {
        if self.fields.is_empty() || self.waiting {
            return;
        }
        let handled = self.fields.key(event, mods, cx.clipboard);
        if !handled && event.logical_key == Key::Named(NamedKey::Enter) {
            self.answer();
        }
    }

    /// Sends the sign-in fields to the backend.
    fn answer(&mut self) {
        let values: Vec<String> = (0..self.fields.len()).map(|index| self.fields.text(index).to_owned()).collect();
        if values.iter().any(|value| value.trim().is_empty()) {
            return;
        }
        self.waiting = true;
        self.backend.send(Command::Answer(values));
    }
}

impl Module for ChatModule {
    fn name(&self) -> &'static str {
        "Chat"
    }

    fn icon(&self) -> char {
        icon::MESSAGES
    }

    fn badge(&self) -> u32 {
        self.store.unread_now()
    }

    fn poll(&mut self, cx: &mut Context<'_>) {
        while let Ok(event) = self.events.try_recv() {
            self.apply(event, cx);
        }
        self.store.refresh();
        while let Ok(files) = self.picked.1.try_recv() {
            if self.current.is_some() {
                self.attached.extend(files);
            }
        }
    }

    fn draw(&mut self, p: &mut Painter<'_>, ui: &mut Ui, area: Rect, cx: &mut Context<'_>) {
        self.now = Instant::now();
        self.today = Today::at(platform::unix_now() as i64);
        self.caret = None;
        if self.store.session == Session::Ready {
            self.draw_chats(p, ui, area, cx);
        } else {
            self.draw_session(p, ui, area);
        }
        // What this frame wanted to show and did not have is fetched, once.
        for file in self.want.drain(..) {
            if self.asked_files.insert(file) {
                self.backend.send(Command::Download(file));
            }
        }
    }

    fn key(&mut self, event: &KeyEvent, mods: ModifiersState, cx: &mut Context<'_>) {
        if self.store.session == Session::Ready {
            self.chat_key(event, mods, cx);
        } else {
            self.session_key(event, mods, cx);
        }
    }

    fn ime_commit(&mut self, text: &str) {
        if let (Session::Ready, Overlay::Info(card)) = (&self.store.session, &mut self.overlay) {
            card.insert(text);
        } else if self.store.session == Session::Ready {
            self.editor().insert(text);
            self.search_edited = self.now;
        } else if !self.fields.is_empty() {
            self.fields.insert(text);
        }
    }

    fn ime_area(&self) -> Option<Rect> {
        self.caret
    }

    fn files_dropped(&mut self, files: Vec<PathBuf>) {
        if self.current.is_some() {
            self.attached.extend(files.iter().map(|path| path.display().to_string()));
        }
    }
}

impl ChatModule {
    /// The list beside the conversation, and what floats over them.
    fn draw_chats(&mut self, p: &mut Painter<'_>, ui: &mut Ui, area: Rect, cx: &mut Context<'_>) {
        let t = p.theme;
        // A video or a recording that was asked for plays once it has landed.
        if let Some(path) = self.playing.and_then(|file| self.store.files.get(&file)) {
            let _ = platform::open_url(path);
            self.playing = None;
        }
        // Under a menu or a card, the page does not answer the mouse.
        let outer = ui.blocker;
        if !matches!(self.overlay, Overlay::None) {
            ui.blocker = outer.or(Some(area));
        }
        let sidebar = Rect::new(area.x, area.y, theme::SIDEBAR_WIDTH.min(area.w * 0.4), area.h);
        let canvas = Rect::new(sidebar.right(), area.y, area.w - sidebar.w, area.h - theme::GAP);
        self.draw_list(p, ui, sidebar);
        canvas_under(p, canvas);
        let clip = p.push_clip(canvas);
        if self.current.is_some() {
            self.draw_thread(p, ui, canvas, cx);
        } else {
            let mid = (canvas.x + canvas.w * 0.5, canvas.y + canvas.h * 0.5);
            icon::draw(p, icon::MESSAGES, (mid.0, mid.1 - 34.0), 34.0, t.text_faint);
            let line = Rect::new(canvas.x, mid.1, canvas.w, 22.0);
            p.label_centered("Pick a chat to start reading", theme::HEADING, line, t.text_muted);
            let hint = Rect::new(canvas.x, mid.1 + 26.0, canvas.w, 18.0);
            p.label_centered("Or type to search your chats", theme::TINY, hint, t.text_faint);
        }
        p.set_clip(clip);
        ui.blocker = outer;
        self.draw_overlay(p, ui, area, cx);
    }

    /// What floats over the page.
    fn draw_overlay(&mut self, p: &mut Painter<'_>, ui: &mut Ui, area: Rect, cx: &mut Context<'_>) {
        if matches!(self.overlay, Overlay::None) {
            self.overlay_shown = false;
            return;
        }
        // Opened by this frame's click, it must not take that same click for one outside it.
        let fresh = !std::mem::replace(&mut self.overlay_shown, true);
        let (released, context) = (ui.released, ui.context);
        (ui.released, ui.context) = (released && !fresh, context && !fresh);
        match &self.overlay {
            Overlay::None => {}
            Overlay::Menu(_) => self.draw_menu(p, ui, area, cx),
            Overlay::Info(_) => self.draw_card(p, ui, area, cx),
            Overlay::Picker(_) => self.draw_picker(p, ui, area, cx),
            Overlay::Viewer(_) => self.draw_viewer(p, ui, area),
        }
        (ui.released, ui.context) = (released, context);
    }

    /// A key while something floats over the page: it is the overlay's.
    fn overlay_key(&mut self, event: &KeyEvent, mods: ModifiersState, cx: &mut Context<'_>) {
        if matches!(self.overlay, Overlay::Info(_)) {
            return self.card_key(event, mods, cx);
        }
        if event.logical_key == Key::Named(NamedKey::Escape) {
            return self.close_overlay();
        }
        match &mut self.overlay {
            Overlay::Viewer(viewer) => match &event.logical_key {
                Key::Named(NamedKey::ArrowLeft) => viewer.step(false),
                Key::Named(NamedKey::ArrowRight) => viewer.step(true),
                _ => {}
            },
            Overlay::Picker(picker) => {
                if event.logical_key == Key::Named(NamedKey::Enter) {
                    self.picker_enter(cx);
                } else if event.logical_key == Key::Named(NamedKey::Tab) {
                    picker.next_field();
                } else {
                    edit_key(picker.editor(), event, mods, cx.clipboard);
                }
            }
            Overlay::Menu(_) | Overlay::Info(_) | Overlay::None => {}
        }
    }
}

/// Opens `url` outside the app. Only web and mail addresses: what a
/// message says must never launch local programs.
fn browse(url: &str) {
    if ["https://", "http://", "mailto:"].iter().any(|scheme| url.starts_with(scheme)) {
        let _ = platform::open_url(url);
    }
}

/// Whether the clipboard holds words.
fn has_text(clipboard: &mut Option<Clipboard>) -> bool {
    if clipboard.is_none() {
        *clipboard = Clipboard::new().ok();
    }
    clipboard.as_mut().and_then(|clipboard| clipboard.get_text().ok()).is_some_and(|text| !text.is_empty())
}

/// The rounded canvas a page floats on, with its soft shadow.
fn canvas_under(p: &mut Painter<'_>, canvas: Rect) {
    let t = p.theme;
    let lifted = Rect::new(canvas.x, canvas.y + 2.0, canvas.w, canvas.h);
    p.shadow(lifted, adelite_ui::paint::fade(t.shadow, 0.35), theme::CANVAS_RADIUS, 18.0);
    p.rect(canvas, t.bg, theme::CANVAS_RADIUS);
}

/// A face: the picture `photo` once it is on disk, the initials of `name`
/// on its colour until then. A picture not yet fetched is noted in `want`.
fn avatar(
    p: &mut Painter<'_>,
    (files, want): (&HashMap<FileId, String>, &mut Vec<FileId>),
    photo: Option<FileId>,
    name: &str,
    center: (f32, f32),
    size: f32,
) {
    let rect = Rect::new(center.0 - size * 0.5, center.1 - size * 0.5, size, size);
    let coming = match photo.map(|file| (file, files.get(&file))) {
        Some((_, Some(path))) => match p.image(path, rect, size * 0.5) {
            Lookup::Ready(_) => return,
            Lookup::Loading => true,
            Lookup::Failed => false,
        },
        Some((file, None)) => {
            want.push(file);
            true
        }
        None => false,
    };
    let t = p.theme;
    let color = theme::name_color(name);
    p.rect(rect, mix(t.surface, color, 0.22), size * 0.5);
    // A picture that is on its way takes the place of nothing: letters that come and go would flicker.
    if !coming {
        let style = adelite_ui::text::Style::semibold((size * 0.36).round());
        p.label_centered(&label::initials(name), style, rect, color);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use adelite_chat::{Chat, Kind, Message, Place, Text};

    use super::*;

    /// A backend that only remembers what it was asked.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<Command>>>);

    impl Backend for Recorder {
        fn name(&self) -> &'static str {
            "Test"
        }

        fn send(&self, command: Command) {
            self.0.lock().unwrap_or_else(PoisonError::into_inner).push(command);
        }
    }

    impl Recorder {
        /// What was asked since the last look.
        fn take(&self) -> Vec<Command> {
            std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
        }
    }

    fn feed(module: &mut ChatModule, event: Event) -> Vec<Request> {
        let (mut clipboard, mut requests) = (None, Vec::new());
        module.apply(event, &mut Context { clipboard: &mut clipboard, requests: &mut requests });
        requests
    }

    fn chat(id: ChatId, unread: u32, read_inbox: MessageId) -> Chat {
        let places = vec![Place { list: List::Main, order: 1, pinned: false }];
        Chat {
            id,
            title: format!("chat {id}"),
            places,
            unread,
            read_inbox,
            can_send: true,
            joined: true,
            ..Chat::default()
        }
    }

    /// A signed-in app with three chats: one read, one with unread messages, one a forum.
    fn module() -> (ChatModule, Recorder) {
        let recorder = Recorder::default();
        let (_, events) = mpsc::channel();
        let mut module = ChatModule::new(Box::new(recorder.clone()), events, Arc::new(|| {}));
        feed(&mut module, Event::Session(Session::Ready));
        feed(&mut module, Event::Chat(chat(1, 0, 90)));
        feed(&mut module, Event::Chat(chat(2, 3, 40)));
        feed(&mut module, Event::Chat(Chat { forum: true, kind: Kind::Group, ..chat(3, 5, 0) }));
        recorder.take();
        (module, recorder)
    }

    fn history(chat: ChatId, from: MessageId, page: Page) -> Command {
        Command::LoadHistory { chat, topic: 0, from, page }
    }

    #[test]
    fn a_chat_opens_where_the_user_left_off() {
        let (mut module, asked) = module();
        // Nothing unread: its newest messages, shown at their end.
        module.pick(1);
        assert_eq!(asked.take(), [Command::Open(1), history(1, 0, Page::Older)]);
        assert!(module.pin_end && module.jump.is_none());
        // Something unread: the messages around what was read last, shown at the first new one.
        module.pick(2);
        assert_eq!(asked.take(), [Command::Close(1), Command::Open(2), history(2, 40, Page::Around)]);
        assert_eq!((module.jump, module.pin_end), (Some(Jump::New), false));
        assert_eq!(module.store.thread((2, 0)).map(|thread| thread.divider), Some(40));
        // A forum shows its topics, and no conversation until one is picked.
        module.pick(3);
        assert_eq!((asked.take(), module.forum, module.current), (vec![Command::LoadTopics(3)], Some(3), Some((2, 0))));
        // Where the backend points: that message, with what is around it.
        let shown = feed(&mut module, Event::Goto { chat: 1, topic: 0, message: 77 });
        assert!(shown.is_empty());
        assert_eq!(asked.take(), [Command::Close(2), Command::Open(1), history(1, 77, Page::Around)]);
        assert_eq!(module.jump, Some(Jump::Message(77)));
        // It is there already: no asking again, only the way to it.
        let around = (70..80).map(|id| Message { id, chat: 1, ..Message::default() }).collect();
        feed(&mut module, Event::History { chat: 1, topic: 0, page: Page::Around, messages: around });
        module.open((1, 0), Some(75));
        assert!(asked.take().is_empty());
        assert_eq!(module.jump, Some(Jump::Message(75)));
    }

    #[test]
    fn what_is_typed_stays_with_its_chat() {
        let (mut module, asked) = module();
        module.pick(1);
        module.draft.insert("half a thought");
        module.pick(2);
        let draft = Command::Draft { chat: 1, topic: 0, text: "half a thought".into() };
        assert!(asked.take().contains(&draft), "the other devices are told");
        assert_eq!(module.draft.text(), "", "the next chat starts empty");
        module.pick(1);
        assert_eq!(module.draft.text(), "half a thought");
        // Unchanged, it is not told again; sent, it is gone.
        module.pick(2);
        assert!(!asked.take().iter().any(|command| matches!(command, Command::Draft { .. })));
        module.pick(1);
        asked.take();
        module.submit();
        let sent = Command::Send { chat: 1, topic: 0, text: "half a thought".into(), reply_to: None };
        assert_eq!(asked.take(), [sent]);
        assert!(module.drafts.is_empty() && module.draft.text().is_empty());
        // Words begun on another device are there when the chat is opened.
        feed(&mut module, Event::Chat(Chat { draft: "from my phone".into(), ..chat(2, 0, 40) }));
        module.pick(2);
        assert_eq!(module.draft.text(), "from my phone");
    }

    #[test]
    fn editing_knows_words_from_captions_and_news_is_said_outside() {
        let (mut module, asked) = module();
        module.pick(1);
        let own = |id, body| Message { id, chat: 1, outgoing: true, body, ..Message::default() };
        let photo = Body::Photo { file: None, full: None, width: 4, height: 3, caption: Text::plain("the roof") };
        let messages = vec![own(10, Body::Text(Text::plain("hello"))), own(11, photo)];
        feed(&mut module, Event::History { chat: 1, topic: 0, page: Page::Older, messages });
        asked.take();
        module.edit(11);
        assert_eq!(module.draft.text(), "the roof");
        module.draft.insert(" at dusk");
        module.submit();
        let edit = Command::Edit { chat: 1, message: 11, text: "the roof at dusk".into(), caption: true };
        assert_eq!(asked.take(), [edit]);
        module.edit(10);
        module.submit();
        assert!(matches!(asked.take().as_slice(), [Command::Edit { message: 10, caption: false, .. }]));
        // A message that just arrived asks for attention, in the window and outside it.
        let now = platform::unix_now() as i64;
        let news = Message { id: 12, chat: 2, date: now, sender_name: "Mara".into(), ..own(12, Body::default()) };
        let said = feed(&mut module, Event::Message(Message { outgoing: false, ..news }));
        assert!(matches!(said.as_slice(),
            [Request::Attention, Request::Notify { title, .. }, Request::Sound { sound: Sound::Arrived, in_view: false }]
            if title == "chat 2"));
        // In the open conversation it is seen arriving; one's own that has left has its own sound.
        let here = Message { id: 13, chat: 1, outgoing: false, date: now, body: Body::default(), ..Message::default() };
        let said = feed(&mut module, Event::Message(here));
        assert!(matches!(said.last(), Some(Request::Sound { sound: Sound::Arrived, in_view: true })));
        let left = feed(&mut module, Event::Sent { was: 99, message: own(14, Body::default()) });
        assert_eq!(left, [Request::Sound { sound: Sound::Sent, in_view: false }]);
    }
}
