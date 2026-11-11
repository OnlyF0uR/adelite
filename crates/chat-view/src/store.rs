//! Everything the view knows: the chats and the lists they are in, the
//! messages of the conversations that were opened, the topics of forums,
//! what a search found, and where files landed. Events go in
//! ([`Store::apply`]); the drawing code reads. No drawing here, so all of
//! it is tested without a window.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use adelite_chat::{
    Body, Chat, ChatId, Delivery, Event, FileId, Info, Kind, List, ListInfo, Member, Message, MessageId, Motion, Page,
    Session, Shared, Topic, TopicId,
};
use adelite_ui::platform::local_time;

/// How long "typing…" stays without being said again.
const TYPING: Duration = Duration::from_secs(6);
/// Messages kept of a conversation the user left.
const KEPT: usize = 100;
/// A message older than this many seconds is no news: it came with a sync.
const FRESH: i64 = 90;
/// Messages kept for showing outside their conversation (answered ones) before the oldest go.
const ASIDES: usize = 400;

/// A conversation: a chat, or one topic of it (zero: the chat itself).
pub type Key = (ChatId, TopicId);

/// A row of a conversation: a message, with the others of its album, and
/// what is worked out once about it.
#[derive(Debug)]
pub struct Item {
    /// The message; of an album, its first.
    pub message: Message,
    /// The other messages of its album, oldest first.
    pub more: Vec<Message>,
    /// The height of its row (with the space and date above it) at the
    /// thread's width. Zero until measured; negative (the old height,
    /// negated) when it may have changed and is to be measured again.
    pub height: f32,
    /// The calendar day it was sent on ([`adelite_ui::platform::LocalTime::date`]).
    pub day: i32,
    /// When it was sent, as its bubble says it: `14:32`.
    pub time: String,
}

impl Item {
    fn new(message: Message) -> Self {
        let at = local_time(message.date);
        Self {
            message,
            more: Vec::new(),
            height: 0.0,
            day: at.date(),
            time: format!("{:02}:{:02}", at.hour, at.minute),
        }
    }

    /// Every message of the row, oldest first.
    pub fn messages(&self) -> impl DoubleEndedIterator<Item = &Message> {
        std::iter::once(&self.message).chain(&self.more)
    }

    /// The newest id in the row.
    #[must_use]
    pub fn last_id(&self) -> MessageId {
        self.more.last().map_or(self.message.id, |message| message.id)
    }
}

/// Whether two messages are shown as one row: pictures and videos sent together.
fn together(a: &Message, b: &Message) -> bool {
    let visual = |message: &Message| {
        matches!(message.body, Body::Photo { .. } | Body::Video { motion: Motion::Video | Motion::Gif, .. })
    };
    a.album != 0 && a.album == b.album && a.sender == b.sender && visual(a) && visual(b)
}

/// The conversations of a chat that a message of `topic` is part of: the
/// chat itself, and its topic when it has one.
fn places(topic: TopicId) -> impl Iterator<Item = TopicId> {
    std::iter::once(0).chain((topic != 0).then_some(topic))
}

/// The messages of one conversation, oldest first.
#[derive(Debug, Default)]
pub struct Thread {
    /// The rows, by ascending id.
    pub items: Vec<Item>,
    /// A page of older messages was asked for and has not come.
    pub loading: bool,
    /// A page of newer messages was asked for and has not come.
    pub loading_newer: bool,
    /// There are no older messages than the first.
    pub complete: bool,
    /// The first page came: an empty thread is an empty chat.
    pub loaded: bool,
    /// Its last row is the conversation's latest message: what arrives is
    /// added. Not so after a jump far back, until the way down was loaded.
    pub live: bool,
    /// How many rows at the front arrived since the view last looked:
    /// their height moves what is on screen down.
    pub prepended: usize,
    /// The width the heights were measured at.
    pub width: f32,
    /// The newest message told to the backend as seen.
    pub seen: MessageId,
    /// What was read when the conversation was opened: the messages after
    /// it are marked as new. Zero for no mark.
    pub divider: MessageId,
}

impl Thread {
    /// The row holding the message `id`.
    #[must_use]
    pub fn find(&self, id: MessageId) -> Option<usize> {
        let at = self.items.partition_point(|item| item.message.id <= id).checked_sub(1)?;
        self.items.get(at).filter(|item| item.messages().any(|message| message.id == id)).map(|_| at)
    }

    /// The message `id`.
    #[must_use]
    pub fn message(&self, id: MessageId) -> Option<&Message> {
        self.items.get(self.find(id)?)?.messages().find(|message| message.id == id)
    }

    fn message_mut(&mut self, id: MessageId) -> Option<&mut Message> {
        let at = self.find(id)?;
        let item = self.items.get_mut(at)?;
        item.height = -item.height.abs();
        if item.message.id == id { Some(&mut item.message) } else { item.more.iter_mut().find(|m| m.id == id) }
    }

    /// Puts `message` in its place; one with its id is replaced. Returns
    /// whether it is new.
    fn insert(&mut self, message: Message) -> bool {
        let at = self.items.partition_point(|item| item.message.id <= message.id);
        if let Some(before) = at.checked_sub(1).and_then(|at| self.items.get_mut(at)) {
            // Told again, or one more of the album before it: that row is measured again.
            let known = before.message.id == message.id || before.more.iter().any(|m| m.id == message.id);
            if known || together(&before.message, &message) {
                before.height = -before.height.abs();
                if before.message.id == message.id {
                    let more = std::mem::take(&mut before.more);
                    *before = Item { more, height: before.height, ..Item::new(message) };
                } else if let Some(slot) = before.more.iter_mut().find(|m| m.id == message.id) {
                    *slot = message;
                } else {
                    let place = before.more.partition_point(|m| m.id < message.id);
                    before.more.insert(place, message);
                }
                return !known;
            }
        }
        if let Some(after) = self.items.get_mut(at).filter(|after| together(&message, &after.message)) {
            // The album's first message came after its others: it leads the row from now on.
            let height = -after.height.abs();
            let mut more = std::mem::take(&mut after.more);
            more.insert(0, std::mem::take(&mut after.message));
            *after = Item { more, height, ..Item::new(message) };
            return true;
        }
        self.items.insert(at, Item::new(message));
        self.stale(at + 1);
        true
    }

    fn remove(&mut self, id: MessageId) {
        let Some(at) = self.find(id) else { return };
        let Some(item) = self.items.get_mut(at) else { return };
        item.height = -item.height.abs();
        if item.message.id != id {
            item.more.retain(|message| message.id != id);
        } else if item.more.is_empty() {
            self.items.remove(at);
            self.stale(at);
        } else {
            let (more, height) = (item.more.split_off(1), item.height);
            if let Some(first) = item.more.pop() {
                *item = Item { more, height, ..Item::new(first) };
            }
        }
    }

    /// The row at `at` depends on the one before it (the date over it, the
    /// space between): measure it again.
    fn stale(&mut self, at: usize) {
        if let Some(item) = self.items.get_mut(at) {
            item.height = -item.height.abs();
        }
    }

    /// Forgets all but the newest messages: the user left the chat.
    fn trim(&mut self) {
        if self.items.len() > KEPT {
            self.items.drain(..self.items.len() - KEPT);
            self.stale(0);
            self.complete = false;
        }
        (self.loading, self.loading_newer, self.prepended) = (false, false, 0);
        self.mark(0);
    }

    /// Sets what was read when the conversation was opened: the messages
    /// after it are marked as new. The mark takes room over a row, so the
    /// rows are measured again when it moves.
    pub fn mark(&mut self, divider: MessageId) {
        if divider != self.divider {
            self.divider = divider;
            self.items.iter_mut().for_each(|item| item.height = -item.height.abs());
        }
    }

    /// The oldest message's id; zero for none.
    #[must_use]
    pub fn oldest(&self) -> MessageId {
        self.items.first().map_or(0, |item| item.message.id)
    }

    /// The newest message's id that the server knows; zero for none.
    #[must_use]
    pub fn newest(&self) -> MessageId {
        let sent = |message: &&Message| message.delivery == Delivery::Sent;
        self.items.iter().rev().flat_map(|item| item.messages().rev()).find(sent).map_or(0, |message| message.id)
    }

    /// The ids of the received messages up to `upto` not yet told as
    /// seen, which are from now on.
    pub fn take_unseen(&mut self, upto: MessageId) -> Vec<MessageId> {
        let seen = self.seen;
        if upto <= seen {
            return Vec::new();
        }
        let fresh = self.items.iter().rev().flat_map(|item| item.messages().rev());
        let unseen: Vec<MessageId> = fresh
            .take_while(|message| message.id > seen)
            .filter(|message| message.id <= upto && !message.outgoing && message.delivery == Delivery::Sent)
            .map(|message| message.id)
            .collect();
        // A message still being sent has an id past every real one: it does not count.
        self.seen = upto.min(self.newest()).max(seen);
        unseen
    }

    /// The first row after the mark of what was read, if it is loaded.
    #[must_use]
    pub fn first_new(&self) -> Option<usize> {
        if self.divider == 0 {
            return None;
        }
        let at = self.items.partition_point(|item| item.message.id <= self.divider);
        // Only what others wrote is new; and nothing is, where the mark is the end.
        self.items.get(at).filter(|item| !item.message.outgoing).map(|_| at)
    }
}

/// What an event asks of the module beyond keeping it.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    /// Nothing.
    None,
    /// A message arrived that wants the user's attention.
    News {
        /// The chat it arrived in.
        chat: ChatId,
        /// That chat's name.
        title: String,
        /// What it says.
        body: String,
    },
    /// Tell the user this went wrong.
    Problem(String),
    /// Tell the user this.
    Notice(String),
    /// A message of the user's own has left.
    Sent,
    /// Signing in moved on: the prompt, or its absence, changed.
    Session,
    /// Show this conversation, and this message of it (zero: none).
    Goto(Key, MessageId),
    /// Open this address outside the app.
    Browse(String),
}

/// A part of what was sent in a chat (its pictures, its files, its links), as far as it was fetched.
#[derive(Debug, Default)]
pub struct Sent {
    /// The messages, newest first.
    pub messages: Vec<Message>,
    /// A page was asked for and has not come.
    pub loading: bool,
    /// There is no more before the last.
    pub complete: bool,
}

/// What a search found so far.
#[derive(Debug, Default)]
pub struct Found {
    /// The search it belongs to.
    pub token: u64,
    /// Chats and people, best first.
    pub chats: Vec<ChatId>,
    /// Messages, newest first.
    pub messages: Vec<Message>,
}

/// Everything the view knows.
#[derive(Debug, Default)]
pub struct Store {
    /// Where signing in stands.
    pub session: Session,
    /// Whether the network is reachable.
    pub online: bool,
    /// Now, in Unix seconds, as the module last said: what is news is measured against it.
    pub clock: i64,
    pub(crate) chats: HashMap<ChatId, Chat>,
    /// The lists, as the strip over the chats shows them.
    pub lists: Vec<ListInfo>,
    /// The list that is shown.
    list: List,
    /// The chats of the shown list, first to last; sorted again when `unsorted`.
    order: Vec<ChatId>,
    unsorted: bool,
    /// Unread messages in chats that are not muted.
    unread: u32,
    pub(crate) threads: HashMap<Key, Thread>,
    /// The topics of each forum, first to last.
    topics: HashMap<ChatId, Vec<Topic>>,
    /// Where each fetched file is.
    pub files: HashMap<FileId, String>,
    /// Who is typing in a chat, and until when that is believed.
    typing: HashMap<ChatId, (String, Instant)>,
    /// Messages shown outside their conversation: answered ones not loaded there.
    pub(crate) asides: HashMap<(ChatId, MessageId), Message>,
    /// The message each chat shows at its top.
    tops: HashMap<ChatId, Message>,
    /// What is known about a chat beyond its row.
    infos: HashMap<ChatId, Info>,
    /// What was sent in a chat, by kind.
    sent: HashMap<(ChatId, Shared), Sent>,
    /// The people of the chat last asked about.
    pub members: (ChatId, Vec<Member>),
    /// People the user knows, as last asked for.
    pub people: Vec<Member>,
    /// What the running search found.
    pub found: Found,
}

impl Store {
    /// Takes in an event.
    pub fn apply(&mut self, event: Event, now: Instant) -> Effect {
        match event {
            Event::Session(session) => {
                if self.session == Session::Ready && session != Session::Ready {
                    // Signed out: nothing of the account stays on screen.
                    *self = Self { files: std::mem::take(&mut self.files), ..Self::default() };
                }
                self.session = session;
                return Effect::Session;
            }
            Event::Online(online) => self.online = online,
            Event::Lists(lists) => {
                // A list that is gone leaves the user in the main one.
                if !lists.iter().any(|info| info.list == self.list) {
                    self.list = List::Main;
                }
                self.lists = lists;
                self.unsorted = true;
            }
            Event::Chat(chat) => {
                self.chats.insert(chat.id, chat);
                self.unsorted = true;
            }
            Event::Topics { chat, topics } => {
                let known = self.topics.entry(chat).or_default();
                for topic in topics {
                    match known.iter_mut().find(|known| known.id == topic.id) {
                        Some(slot) => *slot = topic,
                        None => known.push(topic),
                    }
                }
                known.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.order.cmp(&a.order)).then(b.id.cmp(&a.id)));
            }
            Event::History { chat, topic, page, messages } => self.page((chat, topic), page, messages),
            Event::Message(message) => {
                // Whoever was typing has sent it.
                self.typing.remove(&message.chat);
                let news = self.news(&message);
                for topic in places(message.topic) {
                    // A conversation never opened has no thread: its history is fetched when it is.
                    let open = self.threads.get_mut(&(message.chat, topic));
                    if let Some(thread) = open.filter(|thread| thread.loaded && thread.live) {
                        thread.insert(message.clone());
                    }
                }
                if let Some((title, body)) = news {
                    return Effect::News { chat: message.chat, title, body };
                }
            }
            Event::Sent { was, message } => {
                for topic in places(message.topic) {
                    if let Some(thread) = self.threads.get_mut(&(message.chat, topic)) {
                        thread.remove(was);
                        if thread.live {
                            thread.insert(message.clone());
                        }
                    }
                }
                if message.delivery == Delivery::Sent {
                    return Effect::Sent;
                }
            }
            Event::Body { chat, message, body } => self.change(chat, message, |m| m.body = body.clone()),
            Event::Edited { chat, message } => self.change(chat, message, |m| m.edited = true),
            Event::Counts { chat, message, reactions, views, comments } => self.change(chat, message, |m| {
                m.reactions.clone_from(&reactions);
                (m.views, m.comments) = (views, comments);
            }),
            Event::Pinned { chat, message, on } => self.change(chat, message, |m| m.pinned = on),
            Event::Top { chat, message } => match message {
                Some(message) => drop(self.tops.insert(chat, *message)),
                None => drop(self.tops.remove(&chat)),
            },
            Event::Deleted { chat, messages } => {
                for (_, thread) in self.threads.iter_mut().filter(|(key, _)| key.0 == chat) {
                    for id in &messages {
                        thread.remove(*id);
                    }
                }
                self.found.messages.retain(|found| found.chat != chat || !messages.contains(&found.id));
                for (_, sent) in self.sent.iter_mut().filter(|(key, _)| key.0 == chat) {
                    sent.messages.retain(|message| !messages.contains(&message.id));
                }
                if self.tops.get(&chat).is_some_and(|top| messages.contains(&top.id)) {
                    self.tops.remove(&chat);
                }
            }
            Event::Fetched { chat, messages } => {
                if self.asides.len() + messages.len() > ASIDES {
                    self.asides.clear();
                }
                self.asides.extend(messages.into_iter().map(|message| ((chat, message.id), message)));
                // The rows that answer them show more now.
                for (_, thread) in self.threads.iter_mut().filter(|(key, _)| key.0 == chat) {
                    for item in thread.items.iter_mut().filter(|item| item.message.reply_to.is_some()) {
                        item.height = -item.height.abs();
                    }
                }
            }
            Event::Shared { chat, kind, before, messages } => {
                let sent = self.sent.entry((chat, kind)).or_default();
                (sent.loading, sent.complete) = (false, messages.is_empty());
                if before == 0 {
                    // From the start: what was there is replaced by what is there now.
                    sent.messages = messages;
                } else {
                    for message in messages {
                        if !sent.messages.iter().any(|known| known.id == message.id) {
                            sent.messages.push(message);
                        }
                    }
                }
            }
            Event::File { file, path } => {
                self.files.insert(file, path);
            }
            Event::Typing { chat, who, on } => {
                if on {
                    self.typing.insert(chat, (who, now + TYPING));
                } else {
                    self.typing.remove(&chat);
                }
            }
            Event::Found { token, chats, messages } => {
                if token == self.found.token {
                    for chat in chats {
                        if !self.found.chats.contains(&chat) {
                            self.found.chats.push(chat);
                        }
                    }
                    for message in messages {
                        let known = |found: &Message| found.chat == message.chat && found.id == message.id;
                        if !self.found.messages.iter().any(known) {
                            self.found.messages.push(message);
                        }
                    }
                    self.found.messages.sort_by_key(|found| std::cmp::Reverse(found.date));
                }
            }
            Event::Goto { chat, topic, message } => return Effect::Goto((chat, topic), message),
            Event::Browse(url) => return Effect::Browse(url),
            Event::Info(info) => {
                self.infos.insert(info.chat, info);
            }
            Event::Members { chat, members, .. } => self.members = (chat, members),
            Event::People { people, .. } => self.people = people,
            Event::Notice(text) => return Effect::Notice(text),
            Event::Problem(text) => return Effect::Problem(text),
        }
        Effect::None
    }

    /// Takes in a page of a conversation's messages.
    fn page(&mut self, key: Key, page: Page, messages: Vec<Message>) {
        let latest = self.latest(key);
        let thread = self.threads.entry(key).or_default();
        // A page nobody waits for was asked before a jump elsewhere: it belongs to what was left behind.
        let waited = match page {
            Page::Older => thread.loading || !thread.loaded,
            Page::Newer => thread.loading_newer,
            Page::Around => true,
        };
        if !waited {
            return;
        }
        let known = messages.iter().any(|message| thread.find(message.id).is_some());
        if page == Page::Around && !known && !messages.is_empty() {
            // Somewhere else in the history: what was loaded is left behind.
            *thread = Thread { width: thread.width, seen: thread.seen, divider: thread.divider, ..Thread::default() };
        }
        let front = thread.oldest();
        let fresh = messages.into_iter().filter_map(|message| thread.insert(message).then_some(())).count();
        if front != 0 {
            // Rows in front of what was there: what is on screen moves down by them.
            thread.prepended += thread.items.partition_point(|item| item.last_id() < front);
        }
        thread.loaded = true;
        match page {
            Page::Older => {
                thread.loading = false;
                // A page with nothing new (a backend may repeat the message
                // asked from) is the start of the chat: asking again would
                // only bring the same page.
                thread.complete |= fresh == 0;
                thread.live |= front == 0;
            }
            Page::Newer => {
                thread.loading_newer = false;
                thread.live |= fresh == 0;
            }
            Page::Around => (thread.loading, thread.loading_newer) = (false, false),
        }
        thread.live |= latest != 0 && thread.newest() >= latest;
    }

    /// The id of the latest message of a conversation, as its row in the list knows it.
    fn latest(&self, (chat, topic): Key) -> MessageId {
        if topic == 0 {
            return self.chats.get(&chat).and_then(|chat| chat.last.as_ref()).map_or(0, |last| last.id);
        }
        self.topic(chat, topic).and_then(|topic| topic.last.as_ref()).map_or(0, |last| last.id)
    }

    /// What to say about `message` outside the window, when it is news.
    fn news(&self, message: &Message) -> Option<(String, String)> {
        let chat = self.chats.get(&message.chat)?;
        let fresh = self.clock - message.date < FRESH && message.delivery == Delivery::Sent;
        if message.outgoing
            || !fresh
            || (chat.muted && !message.mentions_me)
            || matches!(message.body, Body::Service(_))
        {
            return None;
        }
        let words = message.body.preview();
        let body = if chat.kind == Kind::Group { format!("{}: {words}", message.sender_name) } else { words };
        Some((chat.title.clone(), body))
    }

    fn change(&mut self, chat: ChatId, message: MessageId, change: impl Fn(&mut Message)) {
        for (_, thread) in self.threads.iter_mut().filter(|(key, _)| key.0 == chat) {
            if let Some(found) = thread.message_mut(message) {
                change(found);
            }
        }
        if let Some(top) = self.tops.get_mut(&chat).filter(|top| top.id == message) {
            change(top);
        }
    }

    /// The chat `id`.
    #[must_use]
    pub fn chat(&self, id: ChatId) -> Option<&Chat> {
        self.chats.get(&id)
    }

    /// The list that is shown.
    #[must_use]
    pub fn list(&self) -> List {
        self.list
    }

    /// Shows another list.
    pub fn show(&mut self, list: List) {
        self.unsorted |= list != self.list;
        self.list = list;
    }

    /// The chats of the shown list, first to last, as of the last [`Store::refresh`].
    #[must_use]
    pub fn listed(&self) -> &[ChatId] {
        &self.order
    }

    /// Every chat the user is in, the most recently active first: where a message can go.
    #[must_use]
    pub fn recent(&self) -> Vec<ChatId> {
        let when = |chat: &Chat| chat.last.as_ref().map_or(0, |last| last.date);
        let listed = |chat: &&Chat| chat.places.iter().any(|place| place.order != 0);
        let mut chats: Vec<&Chat> = self.chats.values().filter(listed).collect();
        chats.sort_unstable_by(|a, b| when(b).cmp(&when(a)).then(b.id.cmp(&a.id)));
        chats.into_iter().map(|chat| chat.id).collect()
    }

    /// Unread messages in chats that are not muted, as of the last [`Store::refresh`].
    #[must_use]
    pub fn unread_now(&self) -> u32 {
        self.unread
    }

    /// The listed chats, first to last.
    #[cfg(test)]
    fn order(&mut self) -> &[ChatId] {
        self.refresh();
        &self.order
    }

    /// Unread messages in chats that are not muted.
    #[cfg(test)]
    fn unread(&mut self) -> u32 {
        self.refresh();
        self.unread
    }

    /// Brings the order and the unread count up to date with the chats.
    pub fn refresh(&mut self) {
        if !std::mem::take(&mut self.unsorted) {
            return;
        }
        let (chats, list) = (&self.chats, self.list);
        self.order.clear();
        self.order.extend(chats.values().filter(|chat| chat.place(list).is_some()).map(|chat| chat.id));
        let place = |id: &ChatId| chats.get(id).and_then(|chat| chat.place(list)).map_or(0, |place| place.order);
        self.order.sort_unstable_by(|a, b| place(b).cmp(&place(a)).then(b.cmp(a)));
        let counts = |chat: &&Chat| chat.place(List::Main).is_some() && !chat.muted;
        self.unread = chats.values().filter(counts).map(|chat| chat.unread).sum();
    }

    /// The messages of a conversation, once it was opened.
    #[must_use]
    pub fn thread(&self, key: Key) -> Option<&Thread> {
        self.threads.get(&key)
    }

    /// The messages of a conversation, there from now on.
    pub fn thread_mut(&mut self, key: Key) -> &mut Thread {
        self.threads.entry(key).or_default()
    }

    /// The user left a conversation: only its newest messages are kept,
    /// and none of one that was not at its end.
    pub fn leave(&mut self, key: Key) {
        if self.threads.get(&key).is_some_and(|thread| !thread.live) {
            self.threads.remove(&key);
        } else if let Some(thread) = self.threads.get_mut(&key) {
            thread.trim();
        }
    }

    /// The topics of the forum `chat`, first to last.
    #[must_use]
    pub fn topics(&self, chat: ChatId) -> &[Topic] {
        self.topics.get(&chat).map_or(&[], Vec::as_slice)
    }

    /// The topic `id` of `chat`.
    #[must_use]
    pub fn topic(&self, chat: ChatId, id: TopicId) -> Option<&Topic> {
        self.topics.get(&chat)?.iter().find(|topic| topic.id == id)
    }

    /// Who is typing in chat `id` right now.
    #[must_use]
    pub fn typing(&self, id: ChatId, now: Instant) -> Option<&str> {
        self.typing.get(&id).filter(|(_, until)| *until > now).map(|(who, _)| who.as_str())
    }

    /// The message `id` of a conversation, if it is loaded there.
    #[must_use]
    pub fn message(&self, key: Key, id: MessageId) -> Option<&Message> {
        self.threads.get(&key)?.message(id)
    }

    /// The message `id` of `chat` wherever it is known: in the conversation
    /// `key`, or fetched to be shown outside it.
    #[must_use]
    pub fn known(&self, key: Key, id: MessageId) -> Option<&Message> {
        self.message(key, id).or_else(|| self.asides.get(&(key.0, id)))
    }

    /// The message a chat shows at its top.
    #[must_use]
    pub fn top(&self, chat: ChatId) -> Option<&Message> {
        self.tops.get(&chat)
    }

    /// What is known about a chat beyond its row.
    #[must_use]
    pub fn info(&self, chat: ChatId) -> Option<&Info> {
        self.infos.get(&chat)
    }

    /// What was sent in a chat, of one kind, as far as it was fetched.
    #[must_use]
    pub fn sent(&self, chat: ChatId, kind: Shared) -> Option<&Sent> {
        self.sent.get(&(chat, kind))
    }

    /// What was sent in a chat, of one kind, there from now on.
    pub fn sent_mut(&mut self, chat: ChatId, kind: Shared) -> &mut Sent {
        self.sent.entry((chat, kind)).or_default()
    }

    /// Starts a search: what the last one found is forgotten.
    pub fn search(&mut self, token: u64) {
        self.found = Found { token, ..Found::default() };
    }

    /// The user's own newest message in a conversation whose words can be edited.
    #[must_use]
    pub fn last_own_text(&self, key: Key) -> Option<&Message> {
        let own = |message: &&Message| {
            message.outgoing && message.delivery == Delivery::Sent && matches!(message.body, Body::Text(_))
        };
        self.threads.get(&key)?.items.iter().rev().map(|item| &item.message).find(own)
    }
}

#[cfg(test)]
mod tests {
    use adelite_chat::{Place, Text};

    use super::*;

    fn message(chat: ChatId, id: MessageId, outgoing: bool) -> Message {
        Message {
            id,
            chat,
            outgoing,
            date: 1_794_300_000 + id,
            body: Body::Text(Text::plain("hi")),
            ..Message::default()
        }
    }

    fn photo(chat: ChatId, id: MessageId, album: i64) -> Message {
        let body = Body::Photo { file: None, full: None, width: 4, height: 3, caption: Text::default() };
        Message { album, body, ..message(chat, id, false) }
    }

    fn chat(id: ChatId, order: i64, unread: u32, muted: bool) -> Chat {
        let places = vec![Place { list: List::Main, order, pinned: false }];
        Chat { id, places, unread, muted, title: format!("chat {id}"), ..Chat::default() }
    }

    fn page(chat: ChatId, page: Page, messages: Vec<Message>) -> Event {
        Event::History { chat, topic: 0, page, messages }
    }

    fn ids(store: &Store, key: Key) -> Vec<MessageId> {
        store.thread(key).map(|t| t.items.iter().map(|i| i.message.id).collect()).unwrap_or_default()
    }

    #[test]
    fn chats_are_listed_by_their_order_and_counted() {
        let (mut store, now) = (Store::default(), Instant::now());
        for chat in [chat(1, 50, 2, false), chat(2, 90, 5, true), chat(3, 0, 9, false), chat(4, 70, 1, false)] {
            store.apply(Event::Chat(chat), now);
        }
        assert_eq!(store.order(), [2, 4, 1], "largest order first; order zero is not listed");
        assert_eq!(store.unread(), 3, "muted and unlisted chats do not count");
        store.apply(Event::Chat(chat(1, 99, 0, false)), now);
        assert_eq!((store.order().to_vec(), store.unread()), (vec![1, 2, 4], 1));
        // Another list shows its own chats, in its own order; one that goes leaves the main one.
        let mut filed = chat(4, 70, 1, false);
        filed.places.push(Place { list: List::Folder(3), order: 5, pinned: false });
        store.apply(Event::Chat(filed), now);
        let folder = ListInfo { list: List::Folder(3), name: "Work".into(), unread: 1 };
        store.apply(Event::Lists(vec![ListInfo::default(), folder]), now);
        store.show(List::Folder(3));
        assert_eq!((store.order().to_vec(), store.unread()), (vec![4], 1), "the count stays the main list's");
        store.apply(Event::Lists(vec![ListInfo::default()]), now);
        assert_eq!((store.list(), store.order().len()), (List::Main, 3));
        assert_eq!(store.recent(), [4, 2, 1], "without messages, by id; the unlisted one is not offered");
    }

    #[test]
    fn a_thread_keeps_its_messages_in_order_through_every_event() {
        let (mut store, now) = (Store::default(), Instant::now());
        store.clock = 1_794_300_040;
        store.apply(Event::Chat(chat(1, 5, 0, false)), now);
        // Before the chat is opened, a message is only news.
        let news = store.apply(Event::Message(message(1, 30, false)), now);
        assert_eq!(news, Effect::News { chat: 1, title: "chat 1".into(), body: "hi".into() });
        assert!(store.thread((1, 0)).is_none());
        store.thread_mut((1, 0)).loading = true;
        store.apply(page(1, Page::Older, vec![message(1, 30, false), message(1, 20, true)]), now);
        assert_eq!(ids(&store, (1, 0)), [20, 30]);
        let state = |store: &Store| store.thread((1, 0)).map(|t| (t.loading, t.loaded, t.live, t.prepended));
        assert_eq!(state(&store), Some((false, true, true, 0)));
        // An older page lands in front and is counted, a repeat is not.
        store.thread_mut((1, 0)).loading = true;
        store.apply(page(1, Page::Older, vec![message(1, 20, true), message(1, 10, false)]), now);
        assert_eq!((ids(&store, (1, 0)), store.thread((1, 0)).map(|t| t.prepended)), (vec![10, 20, 30], Some(1)));
        // The user's own message: sending under one id, sent under another.
        let mut sending = message(1, 9_000, true);
        sending.delivery = Delivery::Sending;
        assert_eq!(store.apply(Event::Message(sending), now), Effect::None);
        assert_eq!(store.apply(Event::Sent { was: 9_000, message: message(1, 40, true) }, now), Effect::Sent);
        assert_eq!(ids(&store, (1, 0)), [10, 20, 30, 40]);
        // One that could not be sent is no cause for the sound of one that left.
        let failed = Message { delivery: Delivery::Failed, ..message(1, 9_001, true) };
        assert_eq!(store.apply(Event::Sent { was: 9_001, message: failed }, now), Effect::None);
        store.apply(Event::Deleted { chat: 1, messages: vec![9_001] }, now);
        assert_eq!(store.last_own_text((1, 0)).map(|m| m.id), Some(40));
        // Changes and deletions find their message; the row after a gap is measured again.
        store.thread_mut((1, 0)).items.iter_mut().for_each(|item| item.height = 50.0);
        store.apply(Event::Edited { chat: 1, message: 20 }, now);
        store.apply(Event::Body { chat: 1, message: 99, body: Body::Service("x".into()) }, now);
        store.apply(Event::Pinned { chat: 1, message: 30, on: true }, now);
        store.apply(Event::Deleted { chat: 1, messages: vec![10, 77] }, now);
        assert_eq!(ids(&store, (1, 0)), [20, 30, 40]);
        let thread = store.thread((1, 0)).expect("the thread");
        assert!(thread.items[0].message.edited && thread.items[0].height < 0.0 && thread.items[2].height > 0.0);
        assert!(thread.items[1].message.pinned);
        // A page that only repeats what is there, or an empty one: the start was reached.
        assert!(store.thread((1, 0)).is_some_and(|t| !t.complete));
        store.thread_mut((1, 0)).loading = true;
        store.apply(page(1, Page::Older, vec![message(1, 20, true)]), now);
        assert!(store.thread((1, 0)).is_some_and(|t| t.complete && t.items[0].height < 0.0 && t.prepended == 1));
        // A page nobody waits for (asked before a jump elsewhere) is dropped.
        store.apply(page(1, Page::Older, vec![message(1, 1, true)]), now);
        assert_eq!(ids(&store, (1, 0)), [20, 30, 40]);
    }

    #[test]
    fn a_jump_far_back_leaves_the_end_until_the_way_down_is_loaded() {
        let (mut store, now) = (Store::default(), Instant::now());
        let mut listed = chat(1, 5, 0, false);
        listed.last = Some(adelite_chat::Preview { id: 900, ..adelite_chat::Preview::default() });
        store.apply(Event::Chat(listed), now);
        store.apply(page(1, Page::Older, vec![message(1, 900, false), message(1, 890, false)]), now);
        assert!(store.thread((1, 0)).is_some_and(|t| t.live));
        // Around a message that is not loaded: only its surroundings are kept, and news is not added.
        store.apply(page(1, Page::Around, vec![message(1, 100, false), message(1, 110, false)]), now);
        assert_eq!(ids(&store, (1, 0)), [100, 110]);
        assert!(store.thread((1, 0)).is_some_and(|t| !t.live && !t.complete));
        store.apply(Event::Message(message(1, 910, false)), now);
        assert_eq!(ids(&store, (1, 0)), [100, 110]);
        // Around one that is: the page joins what is there.
        store.apply(page(1, Page::Around, vec![message(1, 110, false), message(1, 120, false)]), now);
        assert_eq!(ids(&store, (1, 0)), [100, 110, 120]);
        // The way down: newer pages until the chat's latest message is there.
        store.thread_mut((1, 0)).loading_newer = true;
        store.apply(page(1, Page::Newer, vec![message(1, 890, false), message(1, 900, false)]), now);
        assert!(store.thread((1, 0)).is_some_and(|t| t.live && !t.loading_newer));
        // Left before its end, a conversation is opened afresh.
        store.apply(page(1, Page::Around, vec![message(1, 5, false)]), now);
        store.leave((1, 0));
        assert!(store.thread((1, 0)).is_none());
    }

    #[test]
    fn pictures_sent_together_are_one_row() {
        let (mut store, now) = (Store::default(), Instant::now());
        // In any order of arrival, and across pages.
        store.apply(page(1, Page::Older, vec![photo(1, 12, 7), message(1, 20, false), photo(1, 11, 7)]), now);
        store.thread_mut((1, 0)).loading = true;
        store.apply(page(1, Page::Older, vec![photo(1, 10, 7), message(1, 5, false)]), now);
        assert_eq!(ids(&store, (1, 0)), [5, 10, 20]);
        assert_eq!(store.thread((1, 0)).map(|t| t.prepended), Some(1), "the album grew; one row is new in front");
        let thread = store.thread((1, 0)).expect("the thread");
        assert_eq!(thread.items[1].more.iter().map(|m| m.id).collect::<Vec<_>>(), [11, 12]);
        assert_eq!((thread.find(12), thread.find(13), thread.message(11).map(|m| m.id)), (Some(1), None, Some(11)));
        assert_eq!((thread.items[1].last_id(), thread.newest()), (12, 20));
        // One of them changes or goes: the row stays, led by the oldest left.
        store.apply(Event::Edited { chat: 1, message: 11 }, now);
        store.apply(Event::Deleted { chat: 1, messages: vec![10] }, now);
        let thread = store.thread((1, 0)).expect("the thread");
        assert_eq!((ids(&store, (1, 0)), thread.items[1].more.len()), (vec![5, 11, 20], 1));
        assert!(thread.items[1].message.edited);
        store.apply(Event::Deleted { chat: 1, messages: vec![11, 12] }, now);
        assert_eq!(ids(&store, (1, 0)), [5, 20]);
        // Files sent together stay rows of their own.
        let file = Body::File { file: None, name: "a".into(), size: 1, caption: Text::default() };
        let files = [30, 31].map(|id| Message { album: 9, body: file.clone(), ..message(1, id, false) });
        store.thread_mut((1, 0)).loading_newer = true;
        store.apply(page(1, Page::Newer, files.to_vec()), now);
        assert_eq!(ids(&store, (1, 0)), [5, 20, 30, 31]);
    }

    #[test]
    fn seen_messages_are_told_once() {
        let (mut store, now) = (Store::default(), Instant::now());
        let history = vec![message(1, 1, false), message(1, 2, true), message(1, 3, false), message(1, 4, false)];
        store.apply(page(1, Page::Older, history), now);
        // Only as far as the user has looked.
        assert_eq!(store.thread_mut((1, 0)).take_unseen(3), [3, 1]);
        assert!(store.thread_mut((1, 0)).take_unseen(3).is_empty());
        assert_eq!(store.thread_mut((1, 0)).take_unseen(i64::MAX), [4]);
        store.apply(Event::Message(message(1, 5, false)), now);
        assert_eq!(store.thread_mut((1, 0)).take_unseen(i64::MAX), [5]);
        // What was read when the chat was opened marks where the new ones start.
        let thread = store.thread_mut((1, 0));
        thread.items.iter_mut().for_each(|item| item.height = 40.0);
        thread.mark(2);
        assert_eq!(thread.first_new(), Some(2));
        assert!(thread.items.iter().all(|item| item.height < 0.0), "the mark takes room: rows are measured again");
        thread.mark(5);
        assert_eq!(thread.first_new(), None, "nothing after the mark");
    }

    #[test]
    fn what_is_news_and_what_is_not() {
        let (mut store, now) = (Store::default(), Instant::now());
        store.clock = 1_794_300_010;
        let group = Chat { kind: Kind::Group, ..chat(1, 5, 0, false) };
        store.apply(Event::Chat(group), now);
        store.apply(Event::Chat(chat(2, 4, 0, true)), now);
        let from = |chat, id, name: &str| Message { sender_name: name.into(), ..message(chat, id, false) };
        let said = store.apply(Event::Message(from(1, 5, "Mara")), now);
        assert_eq!(said, Effect::News { chat: 1, title: "chat 1".into(), body: "Mara: hi".into() });
        // Muted chats are quiet unless the user is mentioned; one's own and old messages are no news.
        assert_eq!(store.apply(Event::Message(from(2, 6, "Li")), now), Effect::None);
        let mention = Message { mentions_me: true, ..from(2, 7, "Li") };
        assert!(matches!(store.apply(Event::Message(mention), now), Effect::News { .. }));
        assert_eq!(store.apply(Event::Message(message(1, 8, true)), now), Effect::None);
        store.clock += 600;
        assert_eq!(store.apply(Event::Message(from(1, 9, "Mara")), now), Effect::None, "came with a sync");
    }

    #[test]
    fn topics_search_and_asides_are_kept() {
        let (mut store, now) = (Store::default(), Instant::now());
        let topic = |id, order, pinned| Topic { chat: 1, id, order, pinned, ..Topic::default() };
        store.apply(Event::Topics { chat: 1, topics: vec![topic(1, 5, false), topic(2, 9, false)] }, now);
        store.apply(Event::Topics { chat: 1, topics: vec![topic(3, 1, true), topic(1, 20, false)] }, now);
        assert_eq!(store.topics(1).iter().map(|t| t.id).collect::<Vec<_>>(), [3, 1, 2], "pinned, then by order");
        assert!(store.topic(1, 2).is_some() && store.topics(9).is_empty());
        // A message of a topic lands in its conversation, not in another's.
        store.apply(Event::History { chat: 1, topic: 2, page: Page::Older, messages: Vec::new() }, now);
        store.apply(Event::History { chat: 1, topic: 3, page: Page::Older, messages: Vec::new() }, now);
        store.apply(Event::Message(Message { topic: 2, ..message(1, 50, false) }), now);
        assert_eq!((ids(&store, (1, 2)), ids(&store, (1, 3))), (vec![50], vec![]));
        // Only the running search's answers count, each added to the ones before.
        store.search(4);
        store.apply(Event::Found { token: 3, chats: vec![9], messages: Vec::new() }, now);
        store.apply(Event::Found { token: 4, chats: vec![1, 2], messages: vec![message(1, 5, false)] }, now);
        store.apply(Event::Found { token: 4, chats: vec![2, 3], messages: vec![message(1, 5, false)] }, now);
        assert_eq!((store.found.chats.clone(), store.found.messages.len()), (vec![1, 2, 3], 1));
        store.apply(Event::Fetched { chat: 1, messages: vec![message(1, 77, false)] }, now);
        assert!(store.known((1, 2), 77).is_some() && store.known((1, 2), 50).is_some());
        store.apply(Event::Top { chat: 1, message: Some(Box::new(message(1, 77, false))) }, now);
        // What was sent in a chat comes a page at a time; its first page replaces what was there.
        let sent = |before, ids: &[i64]| Event::Shared {
            chat: 1,
            kind: Shared::Files,
            before,
            messages: ids.iter().map(|id| message(1, *id, false)).collect(),
        };
        store.sent_mut(1, Shared::Files).loading = true;
        store.apply(sent(0, &[90, 77]), now);
        store.apply(sent(77, &[77, 60]), now);
        let listed =
            |store: &Store| store.sent(1, Shared::Files).map(|s| s.messages.iter().map(|m| m.id).collect::<Vec<_>>());
        assert_eq!(listed(&store), Some(vec![90, 77, 60]));
        store.apply(sent(60, &[]), now);
        assert!(store.sent(1, Shared::Files).is_some_and(|sent| sent.complete && !sent.loading));
        store.apply(Event::Deleted { chat: 1, messages: vec![77] }, now);
        assert!(store.top(1).is_none());
        assert_eq!(listed(&store), Some(vec![90, 60]), "what is deleted is gone from there too");
        store.apply(sent(0, &[95]), now);
        assert_eq!(listed(&store), Some(vec![95]));
    }

    #[test]
    fn leaving_trims_and_signing_out_forgets() {
        let (mut store, now) = (Store::default(), Instant::now());
        store.apply(Event::Session(Session::Ready), now);
        store.apply(Event::Chat(chat(1, 5, 1, false)), now);
        let history = (1..=250).map(|id| message(1, id, false)).collect();
        store.apply(page(1, Page::Older, history), now);
        store.leave((1, 0));
        assert_eq!(store.thread((1, 0)).map(|t| (t.items.len(), t.oldest(), t.complete)), Some((KEPT, 151, false)));
        store.apply(Event::Typing { chat: 1, who: "Mara".into(), on: true }, now);
        assert_eq!(store.typing(1, now), Some("Mara"));
        assert_eq!(store.typing(1, now + TYPING), None, "typing is believed for a while only");
        assert_eq!(store.apply(Event::Session(Session::Starting), now), Effect::Session);
        assert!(store.order().is_empty() && store.thread((1, 0)).is_none() && store.unread() == 0);
    }
}
