//! What a chat is to Adelite, whatever network carries it.
//!
//! The chat view draws [`Chat`]s and [`Message`]s and speaks to a
//! [`Backend`] in [`Command`]s; the backend answers in [`Event`]s. Nothing
//! here knows Telegram: a backend for another network (or one of our own)
//! is a crate that implements [`Backend`], and the view never changes.
//!
//! Signing in is the backend's business. It says what it needs as a
//! [`Prompt`] (a title, a few fields, a button) and the view shows it, so a
//! network with another way in needs no screen of its own.
//!
//! A network that has less than this model (no folders, no topics, no
//! reactions) simply never says so: every list, flag and count here is
//! empty by default, and the view shows nothing for what is empty.

use std::ops::Range;

/// Identifies a chat within its backend.
pub type ChatId = i64;
/// Identifies a message within its chat. Later messages have larger ids.
pub type MessageId = i64;
/// Identifies whoever sent a message: a person, or a chat speaking as itself.
pub type SenderId = i64;
/// Identifies a file the backend can fetch ([`Command::Download`]).
pub type FileId = i64;
/// Identifies a part of a chat that has messages of its own: a topic of a
/// forum, the comments under a post. Zero is the chat itself.
pub type TopicId = i64;

/// Where the backend is on its way to being signed in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Session {
    /// Starting, or connecting.
    #[default]
    Starting,
    /// It cannot run on this machine: what is wrong, and what would help.
    Unavailable {
        /// The problem, in a few words.
        title: String,
        /// What to do about it.
        detail: String,
    },
    /// It needs something from the user.
    Asking(Prompt),
    /// Signed in: chats and messages flow.
    Ready,
}

/// A step of signing in: what the backend asks the user for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Prompt {
    /// What is asked, in a few words.
    pub title: String,
    /// A sentence or two of explanation.
    pub detail: String,
    /// What to type; answered in this order ([`Command::Answer`]).
    pub fields: Vec<Field>,
    /// The button's label.
    pub action: String,
    /// A page that helps (its label and address), if one does.
    pub link: Option<(String, String)>,
    /// Why the last answer was refused; empty when it was not.
    pub problem: String,
    /// Whether the user can go a step back ([`Command::Back`]).
    pub back: bool,
    /// Another way to get past this step, as a button's label ("Send the
    /// code by SMS"); empty when there is none ([`Command::Alternative`]).
    pub alternative: String,
}

/// One thing a [`Prompt`] asks for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Field {
    /// What it is.
    pub label: String,
    /// Shown in the box while it is empty.
    pub hint: String,
    /// Whether what is typed is hidden.
    pub secret: bool,
}

/// What kind of conversation a chat is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// With one person.
    #[default]
    Private,
    /// Among several.
    Group,
    /// One side speaks, the rest read.
    Channel,
}

/// One of the lists the chats are kept in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum List {
    /// Every chat that is not put away.
    #[default]
    Main,
    /// The chats put away.
    Archive,
    /// A folder of the user's own.
    Folder(i64),
    /// The chats of a community: groups and channels that belong together.
    Community(i64),
}

/// A list as the strip over the chats names it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListInfo {
    /// Which list.
    pub list: List,
    /// Its name: "All", "Work", "Archive".
    pub name: String,
    /// How many of its chats have something unread and are not muted.
    pub unread: u32,
}

/// A chat's place in one list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Place {
    /// The list.
    pub list: List,
    /// Its place there: larger comes first, zero is not listed.
    pub order: i64,
    /// Kept at the top of that list.
    pub pinned: bool,
}

/// A conversation in the list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Chat {
    /// Its id.
    pub id: ChatId,
    /// Its name.
    pub title: String,
    /// What kind it is.
    pub kind: Kind,
    /// Its picture, if it has one.
    pub photo: Option<FileId>,
    /// Where it is in each list it is in.
    pub places: Vec<Place>,
    /// Its messages do not ask for attention.
    pub muted: bool,
    /// How many of its messages are unread.
    pub unread: u32,
    /// One of the unread messages mentions the user.
    pub mentioned: bool,
    /// The user marked it as unread, whatever was read.
    pub marked_unread: bool,
    /// Its latest message, as the list shows it.
    pub last: Option<Preview>,
    /// The newest received message the user has read.
    pub read_inbox: MessageId,
    /// The newest message of the user's own that the other side has read.
    pub read_outbox: MessageId,
    /// Whether the user may write here.
    pub can_send: bool,
    /// The line under its name: "online", "12 members".
    pub status: String,
    /// When the person it is with was last here, in Unix seconds; zero
    /// when that is unknown or [`Chat::status`] says it all.
    pub last_seen: i64,
    /// Words begun here and not sent, on this device or another.
    pub draft: String,
    /// Its messages are kept in topics ([`Command::LoadTopics`]).
    pub forum: bool,
    /// The user is in it. A chat found by searching may be one they are not in.
    pub joined: bool,
    /// The user runs it: may pin, remove what others wrote, change what it is called.
    pub admin: bool,
}

impl Chat {
    /// Its place in `list`, if it is listed there.
    #[must_use]
    pub fn place(&self, list: List) -> Option<&Place> {
        self.places.iter().find(|place| place.list == list && place.order != 0)
    }

    /// Whether it wears an unread mark: something unread, or marked so.
    #[must_use]
    pub fn waiting(&self) -> bool {
        self.unread > 0 || self.mentioned || self.marked_unread
    }
}

/// A chat's latest message, as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preview {
    /// The message's id.
    pub id: MessageId,
    /// Who sent it, where the list names them (in groups); empty otherwise.
    pub sender: String,
    /// What it says, on one line.
    pub text: String,
    /// When it was sent, in Unix seconds.
    pub date: i64,
    /// Whether the user sent it.
    pub outgoing: bool,
}

/// A part of a chat with messages of its own: a topic of a forum.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Topic {
    /// The chat it is in.
    pub chat: ChatId,
    /// Its id there.
    pub id: TopicId,
    /// Its name.
    pub name: String,
    /// Its colour as `0xRRGGBB`; zero for none of its own.
    pub color: u32,
    /// Its place among the chat's topics: larger comes first.
    pub order: i64,
    /// Kept at the top.
    pub pinned: bool,
    /// Nobody may write in it any more.
    pub closed: bool,
    /// Its messages do not ask for attention.
    pub muted: bool,
    /// How many of its messages are unread.
    pub unread: u32,
    /// One of the unread messages mentions the user.
    pub mentioned: bool,
    /// Its latest message.
    pub last: Option<Preview>,
    /// The newest received message the user has read.
    pub read_inbox: MessageId,
    /// The newest message of the user's own that was read.
    pub read_outbox: MessageId,
}

/// Whether a message of the user's own has left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Delivery {
    /// On the server (every received message is).
    #[default]
    Sent,
    /// On its way.
    Sending,
    /// It could not be sent.
    Failed,
}

/// A message.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Message {
    /// Its id.
    pub id: MessageId,
    /// The chat it is in.
    pub chat: ChatId,
    /// The topic it is in; zero for none.
    pub topic: TopicId,
    /// Who sent it.
    pub sender: SenderId,
    /// The sender's name.
    pub sender_name: String,
    /// The sender's picture, if they have one.
    pub sender_photo: Option<FileId>,
    /// Whether the user sent it.
    pub outgoing: bool,
    /// When it was sent, in Unix seconds.
    pub date: i64,
    /// Whether it was changed after it was sent.
    pub edited: bool,
    /// Whether it has left (the user's own).
    pub delivery: Delivery,
    /// The message it answers, if any.
    pub reply_to: Option<MessageId>,
    /// Whose message it passes on; empty when it is the sender's own.
    pub forwarded_from: String,
    /// What it holds.
    pub body: Body,
    /// The page its first link leads to, shown under the words.
    pub link: Option<LinkPreview>,
    /// What people answered it with, most used first.
    pub reactions: Vec<Reaction>,
    /// Buttons under it, in rows.
    pub buttons: Vec<Vec<Button>>,
    /// Kept at the top of its chat.
    pub pinned: bool,
    /// Messages sent together share one; zero for a message on its own.
    pub album: i64,
    /// How often it was seen (a channel's post); zero where that is not counted.
    pub views: u32,
    /// How many comments it has, where it can be commented on ([`Command::Comments`]).
    pub comments: Option<u32>,
    /// It mentions the user, who has not read it yet.
    pub mentions_me: bool,
}

/// What a message holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// Words.
    Text(Text),
    /// A picture.
    Photo {
        /// The picture at the size a bubble shows it, if the backend has one.
        file: Option<FileId>,
        /// The picture at its largest, for looking at it alone.
        full: Option<FileId>,
        /// Its width in pixels, at its largest.
        width: u32,
        /// Its height in pixels, at its largest.
        height: u32,
        /// What is written under it.
        caption: Text,
    },
    /// Something that moves: a video, a GIF, a round video message.
    Video {
        /// What it is.
        motion: Motion,
        /// A still of it, to show before it is fetched.
        still: Option<FileId>,
        /// The video itself.
        file: Option<FileId>,
        /// Its width in pixels.
        width: u32,
        /// Its height in pixels.
        height: u32,
        /// How long it runs, in seconds.
        seconds: u32,
        /// What is written under it.
        caption: Text,
    },
    /// A sticker: a picture without a bubble.
    Sticker {
        /// The picture (a still, where the sticker moves).
        file: Option<FileId>,
        /// The emoji it stands for.
        emoji: String,
        /// Its width in pixels.
        width: u32,
        /// Its height in pixels.
        height: u32,
    },
    /// Something to listen to: a voice message, a piece of music.
    Audio {
        /// The recording.
        file: Option<FileId>,
        /// A voice message, not a file with a title.
        voice: bool,
        /// What it is called: the title and who performs it; empty for a voice message.
        title: String,
        /// How long it runs, in seconds.
        seconds: u32,
        /// What is written under it.
        caption: Text,
    },
    /// A question with answers to pick from.
    Poll(Poll),
    /// A file to keep.
    File {
        /// The file.
        file: Option<FileId>,
        /// Its name.
        name: String,
        /// Its size in bytes.
        size: u64,
        /// What is written under it.
        caption: Text,
    },
    /// Something the app does not show yet, named: "Location", "Contact".
    Other {
        /// What it is.
        label: String,
        /// What is written under it.
        caption: Text,
    },
    /// Said by the chat itself: "Mara joined the group".
    Service(String),
}

/// What kind of moving picture a [`Body::Video`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Motion {
    /// A video.
    #[default]
    Video,
    /// A short loop without sound.
    Gif,
    /// A round video message.
    Round,
}

/// A question with answers to pick from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Poll {
    /// What is asked.
    pub question: String,
    /// The answers, in order.
    pub options: Vec<PollOption>,
    /// How many people answered.
    pub voters: u32,
    /// Nobody can answer any more.
    pub closed: bool,
    /// Several answers may be picked.
    pub multiple: bool,
    /// One answer is right.
    pub quiz: bool,
}

impl Poll {
    /// Whether the user has answered it.
    #[must_use]
    pub fn answered(&self) -> bool {
        self.options.iter().any(|option| option.chosen)
    }
}

/// One answer of a [`Poll`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PollOption {
    /// The answer.
    pub text: String,
    /// How many picked it.
    pub votes: u32,
    /// Its share of the voters, in percent.
    pub percent: u8,
    /// The user picked it.
    pub chosen: bool,
}

/// The page a link leads to, as a card under the message.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkPreview {
    /// The address.
    pub url: String,
    /// The site's name.
    pub site: String,
    /// The page's title.
    pub title: String,
    /// A few words about it.
    pub description: String,
    /// Its picture, if it has one.
    pub photo: Option<FileId>,
    /// The picture's width in pixels.
    pub width: u32,
    /// The picture's height in pixels.
    pub height: u32,
}

/// What people answered a message with, and how many did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reaction {
    /// The emoji.
    pub emoji: String,
    /// How many answered with it.
    pub count: u32,
    /// The user is one of them.
    pub chosen: bool,
}

/// A button under a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    /// What it says.
    pub label: String,
    /// What a click does.
    pub press: Press,
}

/// What a click on a [`Button`] does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Press {
    /// Opens this address.
    Link(String),
    /// Tells whoever sent the message ([`Command::Press`], with this back).
    Data(String),
    /// Nothing this app can do.
    Nothing,
}

impl Default for Body {
    fn default() -> Self {
        Self::Text(Text::default())
    }
}

impl Body {
    /// The body on one line, as a chat's latest message or an answered
    /// message is shown: its words, or what it is.
    #[must_use]
    pub fn preview(&self) -> String {
        let named = |what: &str, caption: &Text| {
            if caption.text.is_empty() { what.to_owned() } else { format!("{what} · {}", one_line(&caption.text)) }
        };
        match self {
            Self::Text(text) => one_line(&text.text),
            Self::Photo { caption, .. } => named("Photo", caption),
            Self::Video { motion, caption, .. } => named(motion.label(), caption),
            Self::Sticker { emoji, .. } => format!("{emoji} Sticker").trim().to_owned(),
            Self::Audio { voice: true, caption, .. } => named("Voice message", caption),
            Self::Audio { title, caption, .. } => named(if title.is_empty() { "Audio" } else { title }, caption),
            Self::Poll(poll) => format!("Poll · {}", one_line(&poll.question)),
            Self::File { name, caption, .. } => named(name, caption),
            Self::Other { label, caption } => named(label, caption),
            Self::Service(text) => one_line(text),
        }
    }

    /// The styled words: the text, or the caption. None where there are none to style.
    #[must_use]
    pub fn words(&self) -> Option<&Text> {
        match self {
            Self::Text(text) => Some(text),
            Self::Photo { caption, .. }
            | Self::Video { caption, .. }
            | Self::Audio { caption, .. }
            | Self::File { caption, .. }
            | Self::Other { caption, .. } => Some(caption),
            Self::Sticker { .. } | Self::Poll(_) | Self::Service(_) => None,
        }
    }

    /// The words that can be copied or edited: the text, or the caption.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Service(text) => text,
            Self::Poll(poll) => &poll.question,
            _ => self.words().map_or("", |words| words.text.as_str()),
        }
    }

    /// Whether the user can change its words: text, or what has a caption.
    #[must_use]
    pub fn editable(&self) -> bool {
        matches!(self, Self::Text(_) | Self::Photo { .. } | Self::Video { .. } | Self::Audio { .. } | Self::File { .. })
    }
}

impl Motion {
    /// What it is called.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Gif => "GIF",
            Self::Round => "Video message",
        }
    }
}

/// At most the first 200 characters of `text`, its line breaks as spaces.
fn one_line(text: &str) -> String {
    text.chars().take(200).map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// Words, some of them styled.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Text {
    /// The words.
    pub text: String,
    /// What is styled: byte ranges of `text`, sorted, not overlapping.
    pub spans: Vec<Span>,
}

impl Text {
    /// Words without styles.
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), spans: Vec::new() }
    }
}

/// A styled part of a [`Text`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// The bytes of the text it covers.
    pub range: Range<usize>,
    /// How they look.
    pub style: Style,
}

/// How a [`Span`] looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Style {
    /// Heavier.
    Bold,
    /// Slanted.
    Italic,
    /// Code: monospaced.
    Code,
    /// Struck through.
    Strike,
    /// Underlined.
    Underline,
    /// A link to this address.
    Link(String),
    /// Someone's name, as a mention. What it holds leads to them
    /// ([`Command::Follow`]); empty when it leads nowhere.
    Mention(String),
    /// A word to search for: a hashtag.
    Tag,
    /// Hidden until the user asks to see it.
    Spoiler,
    /// Said by someone else: a quotation.
    Quote,
}

/// Which messages a page of history holds, seen from the message asked from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    /// The ones before it.
    #[default]
    Older,
    /// The ones after it.
    Newer,
    /// The ones on both sides of it, and itself.
    Around,
}

/// A part of what was sent in a chat that is worth looking through by
/// itself ([`Command::Shared`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Shared {
    /// Pictures and videos.
    #[default]
    Media,
    /// Files.
    Files,
    /// Messages with links in them.
    Links,
}

/// Someone in a chat, or someone to put in one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Member {
    /// Who.
    pub id: SenderId,
    /// Their name.
    pub name: String,
    /// Their picture, if they have one.
    pub photo: Option<FileId>,
    /// The line under their name: "online", "last seen recently".
    pub status: String,
    /// What they are in the chat.
    pub role: Role,
}

/// What a [`Member`] is in a chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    /// One of many.
    #[default]
    Member,
    /// Helps run it.
    Admin,
    /// It is theirs.
    Owner,
}

/// What there is to know about a chat beyond its row ([`Command::Info`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Info {
    /// The chat.
    pub chat: ChatId,
    /// What it says about itself: a person's bio, a group's description.
    pub about: String,
    /// The name it is found under, with its sign: "@adelite".
    pub handle: String,
    /// The person's phone number, where the user may see it.
    pub phone: String,
    /// The address that leads here: its public page, or an invitation.
    pub link: String,
    /// How many are in it; zero for a person.
    pub members: u32,
    /// The user may put others in it.
    pub can_invite: bool,
    /// The user may change its name and what it says about itself.
    pub can_edit: bool,
}

/// Something the view asks of the backend. None of them blocks; what comes
/// of them arrives as [`Event`]s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// The user's answer to the [`Prompt`]: a value per field.
    Answer(Vec<String>),
    /// A step back in signing in.
    Back,
    /// The other way the [`Prompt`] offers ([`Prompt::alternative`]).
    Alternative,
    /// More of the chats of a list.
    LoadChats(List),
    /// The user is looking at this chat.
    Open(ChatId),
    /// The user left this chat.
    Close(ChatId),
    /// A page of a chat's messages, seen from `from` (zero: the newest).
    LoadHistory {
        /// The chat.
        chat: ChatId,
        /// The topic; zero for the chat itself.
        topic: TopicId,
        /// The message the page is seen from.
        from: MessageId,
        /// Which side of it.
        page: Page,
    },
    /// The topics of a forum ([`Event::Topics`]).
    LoadTopics(ChatId),
    /// Send words to a chat.
    Send {
        /// The chat.
        chat: ChatId,
        /// The topic; zero for the chat itself.
        topic: TopicId,
        /// The words.
        text: String,
        /// The message they answer, if any.
        reply_to: Option<MessageId>,
    },
    /// Send files to a chat, together (pictures go as photos).
    SendFiles {
        /// The chat.
        chat: ChatId,
        /// The topic; zero for the chat itself.
        topic: TopicId,
        /// The files' paths.
        paths: Vec<String>,
        /// What to write under them.
        caption: String,
        /// The message they answer, if any.
        reply_to: Option<MessageId>,
    },
    /// Replace the words of a message of the user's own (its text, or its caption).
    Edit {
        /// The chat.
        chat: ChatId,
        /// The message.
        message: MessageId,
        /// Its new words.
        text: String,
        /// The words are written under something (a picture, a file), not the message itself.
        caption: bool,
    },
    /// Delete messages.
    Delete {
        /// The chat.
        chat: ChatId,
        /// The messages.
        messages: Vec<MessageId>,
        /// For everyone, where the network allows; otherwise only for the user.
        for_all: bool,
    },
    /// Pass messages on to another chat.
    Forward {
        /// The chat they are in.
        from: ChatId,
        /// The messages.
        messages: Vec<MessageId>,
        /// The chat they go to.
        to: ChatId,
    },
    /// Keep a message at the top of its chat, or stop keeping it there.
    Pin {
        /// The chat.
        chat: ChatId,
        /// The message.
        message: MessageId,
        /// Whether it is kept there.
        on: bool,
    },
    /// Answer a message with an emoji, or take that back.
    React {
        /// The chat.
        chat: ChatId,
        /// The message.
        message: MessageId,
        /// The emoji.
        emoji: String,
        /// Whether it is given or taken back.
        on: bool,
    },
    /// Answer a poll: the picked options by their place; none takes the answer back.
    Vote {
        /// The chat.
        chat: ChatId,
        /// The message.
        message: MessageId,
        /// The picked options.
        options: Vec<usize>,
    },
    /// A button under a message was clicked ([`Press::Data`]).
    Press {
        /// The chat.
        chat: ChatId,
        /// The message.
        message: MessageId,
        /// What the button carries.
        data: String,
    },
    /// The user has seen these messages.
    Read {
        /// The chat.
        chat: ChatId,
        /// The messages.
        messages: Vec<MessageId>,
    },
    /// Mark a chat as unread, or as read through and through.
    MarkUnread {
        /// The chat.
        chat: ChatId,
        /// Unread, or read.
        on: bool,
    },
    /// Stop a chat's messages from asking for attention, or let them again.
    Mute {
        /// The chat.
        chat: ChatId,
        /// Muted, or not.
        on: bool,
    },
    /// Keep a chat at the top of a list, or stop keeping it there.
    PinChat {
        /// The chat.
        chat: ChatId,
        /// The list.
        list: List,
        /// Whether it is kept there.
        on: bool,
    },
    /// Put a chat into the archive, or back into the main list.
    Move {
        /// The chat.
        chat: ChatId,
        /// [`List::Archive`] or [`List::Main`].
        to: List,
    },
    /// Become part of a group or channel.
    Join(ChatId),
    /// Leave a group or channel, or drop the conversation with a person: it
    /// goes from the list, with its messages.
    Leave(ChatId),
    /// Keep words begun in a chat and not sent; empty forgets them.
    Draft {
        /// The chat.
        chat: ChatId,
        /// The topic; zero for the chat itself.
        topic: TopicId,
        /// The words.
        text: String,
    },
    /// The user is typing in a chat.
    Typing {
        /// The chat.
        chat: ChatId,
        /// The topic; zero for the chat itself.
        topic: TopicId,
    },
    /// Look for chats, people and messages ([`Event::Found`]).
    Search {
        /// What to look for.
        query: String,
        /// Only the messages of this chat; everywhere when none.
        chat: Option<ChatId>,
        /// Given back with what is found, so answers to earlier questions can be told apart.
        token: u64,
    },
    /// Go where a link or a mention leads: a chat, a message
    /// ([`Event::Goto`]), or a page outside ([`Event::Browse`]).
    Follow(String),
    /// Messages of a chat by their ids, for showing them outside the
    /// conversation: an answered message, the pinned one ([`Event::Fetched`]).
    Fetch {
        /// The chat.
        chat: ChatId,
        /// The messages.
        messages: Vec<MessageId>,
    },
    /// A page of what was sent in a chat, of one kind, older than
    /// `before` (zero: the newest) ([`Event::Shared`]).
    Shared {
        /// The chat.
        chat: ChatId,
        /// Which kind.
        kind: Shared,
        /// The oldest message of that kind the view has, or zero.
        before: MessageId,
    },
    /// The oldest unread message that mentions the user ([`Event::Goto`]).
    NextMention(ChatId),
    /// The comments under a post ([`Event::Goto`]).
    Comments {
        /// The chat.
        chat: ChatId,
        /// The post.
        message: MessageId,
    },
    /// Fetch a file; an [`Event::File`] says where it landed.
    Download(FileId),
    /// What there is to know about a chat ([`Event::Info`]).
    Info(ChatId),
    /// The people in a chat whose names hold `query` ([`Event::Members`]).
    Members {
        /// The chat.
        chat: ChatId,
        /// Part of a name; empty for everyone.
        query: String,
    },
    /// People the user knows whose names hold `query` ([`Event::People`]).
    People(String),
    /// Put someone into a chat.
    AddMember {
        /// The chat.
        chat: ChatId,
        /// Who.
        user: SenderId,
    },
    /// Take someone out of a chat.
    RemoveMember {
        /// The chat.
        chat: ChatId,
        /// Who.
        user: SenderId,
    },
    /// Let someone help run a chat, or no longer.
    Promote {
        /// The chat.
        chat: ChatId,
        /// Who.
        user: SenderId,
        /// Whether they help run it.
        on: bool,
    },
    /// Change a chat's name.
    Rename {
        /// The chat.
        chat: ChatId,
        /// Its new name.
        title: String,
    },
    /// Change what a chat says about itself.
    Describe {
        /// The chat.
        chat: ChatId,
        /// The new words.
        about: String,
    },
    /// Make an address that invites into a chat; it comes as the chat's [`Info::link`].
    Invite(ChatId),
    /// Start a group with these people ([`Event::Goto`]).
    NewGroup {
        /// Its name.
        title: String,
        /// Who is in it besides the user.
        members: Vec<SenderId>,
    },
    /// The conversation with one person ([`Event::Goto`]).
    Talk(SenderId),
    /// Sign out.
    LogOut,
}

/// Something the backend tells the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Where signing in stands.
    Session(Session),
    /// Whether the network is reachable.
    Online(bool),
    /// The lists the chats are kept in, in the order they are shown.
    Lists(Vec<ListInfo>),
    /// A chat is new, or something about it changed.
    Chat(Chat),
    /// Topics of a forum: new ones, or ones that changed.
    Topics {
        /// The chat.
        chat: ChatId,
        /// The topics.
        topics: Vec<Topic>,
    },
    /// A page of a chat's messages ([`Command::LoadHistory`]), in any
    /// order. None: there are no more on that side.
    History {
        /// The chat.
        chat: ChatId,
        /// The topic; zero for the chat itself.
        topic: TopicId,
        /// Which page was asked for.
        page: Page,
        /// The messages.
        messages: Vec<Message>,
    },
    /// A new message, received or the user's own.
    Message(Message),
    /// What a message holds changed.
    Body {
        /// The chat.
        chat: ChatId,
        /// The message.
        message: MessageId,
        /// What it now holds.
        body: Body,
    },
    /// A message was edited by its sender.
    Edited {
        /// The chat.
        chat: ChatId,
        /// The message.
        message: MessageId,
    },
    /// What people made of a message changed.
    Counts {
        /// The chat.
        chat: ChatId,
        /// The message.
        message: MessageId,
        /// What it was answered with.
        reactions: Vec<Reaction>,
        /// How often it was seen.
        views: u32,
        /// How many comments it has.
        comments: Option<u32>,
    },
    /// A message is now kept at the top of its chat, or no longer.
    Pinned {
        /// The chat.
        chat: ChatId,
        /// The message.
        message: MessageId,
        /// Whether it is kept there.
        on: bool,
    },
    /// The message a chat shows at its top: its newest pinned one, or none.
    Top {
        /// The chat.
        chat: ChatId,
        /// The message.
        message: Option<Box<Message>>,
    },
    /// A message of the user's own has left, or failed to
    /// ([`Message::delivery`]): `was` is the id it had while sending.
    Sent {
        /// The id it had while sending.
        was: MessageId,
        /// The message as it now is.
        message: Message,
    },
    /// Messages are gone.
    Deleted {
        /// The chat.
        chat: ChatId,
        /// The messages.
        messages: Vec<MessageId>,
    },
    /// Messages asked for by their ids ([`Command::Fetch`]).
    Fetched {
        /// The chat.
        chat: ChatId,
        /// The messages that exist.
        messages: Vec<Message>,
    },
    /// A page of what was sent in a chat, of one kind ([`Command::Shared`]),
    /// newest first. None: there is no more.
    Shared {
        /// The chat.
        chat: ChatId,
        /// Which kind.
        kind: Shared,
        /// The message the page was asked from: zero for the first page.
        before: MessageId,
        /// The messages.
        messages: Vec<Message>,
    },
    /// A file is on disk at `path`.
    File {
        /// The file.
        file: FileId,
        /// Where it is.
        path: String,
    },
    /// Someone started or stopped typing in a chat.
    Typing {
        /// The chat.
        chat: ChatId,
        /// Their name.
        who: String,
        /// Whether they are typing.
        on: bool,
    },
    /// What a search found ([`Command::Search`]); it may come in parts,
    /// each adding to the ones before it.
    Found {
        /// The search's token.
        token: u64,
        /// Chats and people, best first.
        chats: Vec<ChatId>,
        /// Messages, newest first.
        messages: Vec<Message>,
    },
    /// Show this chat, and this message of it when there is one.
    Goto {
        /// The chat.
        chat: ChatId,
        /// The topic; zero for the chat itself.
        topic: TopicId,
        /// The message; zero for none.
        message: MessageId,
    },
    /// Open this address outside the app.
    Browse(String),
    /// What there is to know about a chat ([`Command::Info`]).
    Info(Info),
    /// People in a chat ([`Command::Members`]).
    Members {
        /// The chat.
        chat: ChatId,
        /// What was asked for.
        query: String,
        /// Who was found.
        members: Vec<Member>,
    },
    /// People the user knows ([`Command::People`]).
    People {
        /// What was asked for.
        query: String,
        /// Who was found.
        people: Vec<Member>,
    },
    /// Something to tell the user that is no problem: what a button answered.
    Notice(String),
    /// Something went wrong: what to tell the user.
    Problem(String),
}

/// A chat network. The view sends it [`Command`]s; it delivers [`Event`]s
/// through the channel it was started with, and wakes the interface.
pub trait Backend {
    /// The network's name, as the user knows it: "Telegram".
    fn name(&self) -> &'static str;

    /// Asks it to do something. Never blocks.
    fn send(&self, command: Command);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_are_one_line_and_name_what_is_not_words() {
        let caption = Text::plain("the plan\nfor Q3");
        assert_eq!(Body::Text(Text::plain("a\nb")).preview(), "a b");
        assert_eq!(
            Body::Photo { file: None, full: None, width: 1, height: 1, caption: caption.clone() }.preview(),
            "Photo · the plan for Q3"
        );
        assert_eq!(Body::Other { label: "Location".into(), caption: Text::default() }.preview(), "Location");
        assert_eq!(Body::File { file: None, name: "q3.pdf".into(), size: 9, caption }.text(), "the plan\nfor Q3");
        assert_eq!(Body::Text(Text::plain("é".repeat(500))).preview().chars().count(), 200);
        let voice = Body::Audio { file: None, voice: true, title: String::new(), seconds: 4, caption: Text::default() };
        assert_eq!(voice.preview(), "Voice message");
        let gif = Body::Video {
            motion: Motion::Gif,
            still: None,
            file: None,
            width: 1,
            height: 1,
            seconds: 2,
            caption: Text::default(),
        };
        assert_eq!((gif.preview().as_str(), gif.editable()), ("GIF", true));
        let sticker = Body::Sticker { file: None, emoji: "🎉".into(), width: 1, height: 1 };
        assert_eq!((sticker.preview().as_str(), sticker.text(), sticker.editable()), ("🎉 Sticker", "", false));
    }

    #[test]
    fn a_chat_is_listed_where_it_has_an_order() {
        let chat = Chat {
            places: vec![
                Place { list: List::Main, order: 7, pinned: true },
                Place { list: List::Folder(2), order: 0, pinned: false },
            ],
            marked_unread: true,
            ..Chat::default()
        };
        assert_eq!(chat.place(List::Main).map(|place| (place.order, place.pinned)), Some((7, true)));
        assert!(chat.place(List::Folder(2)).is_none() && chat.place(List::Archive).is_none());
        assert!(chat.waiting() && !Chat::default().waiting());
        let poll = Poll { options: vec![PollOption { chosen: true, ..PollOption::default() }], ..Poll::default() };
        assert!(poll.answered() && !Poll::default().answered());
    }
}
