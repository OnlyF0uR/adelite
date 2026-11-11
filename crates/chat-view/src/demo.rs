//! A chat app filled with made-up conversations, for snapshots of the
//! design: no network, no account. One function per surface: the list, a
//! conversation, its menus, a search, a forum's topics, a chat's panel, the
//! card to pick chats on, a picture looked at alone.

use std::sync::Arc;
use std::sync::mpsc;

use adelite_chat::{
    Backend, Body, Button, Chat, ChatId, Command, Delivery, Event, Field, Info, Kind, LinkPreview, List, ListInfo,
    Member, Message, Motion, Page, Place, Poll, PollOption, Press, Preview, Prompt, Reaction, Role, Session, Span,
    Style, Text, Topic,
};
use adelite_ui::platform::unix_now;
use adelite_ui::{Context, Module};

use crate::menu::{Menu, Target};
use crate::picker::{Picker, Purpose};
use crate::viewer::Viewer;
use crate::{ChatModule, Jump, Overlay};

/// A backend that hears nothing and says nothing.
struct Quiet;

impl Backend for Quiet {
    fn name(&self) -> &'static str {
        "Telegram"
    }

    fn send(&self, _: Command) {}
}

/// The demo's group chat, its chat with one person, its channel and its forum.
const DESIGN: ChatId = -60;
const MARA: ChatId = 7;
const NEWS: ChatId = -1_000_000_000_050;
const PRODUCT: ChatId = -63;
const WORK: List = List::Folder(1);

fn feed(module: &mut ChatModule, events: Vec<Event>) {
    let (mut clipboard, mut requests) = (None, Vec::new());
    let mut cx = Context { clipboard: &mut clipboard, requests: &mut requests };
    for event in events {
        module.apply(event, &mut cx);
    }
    module.poll(&mut cx);
}

/// A chat app whose backend is in the state `session`.
#[must_use]
pub fn at(session: Session) -> ChatModule {
    let (_, events) = mpsc::channel();
    let mut module = ChatModule::new(Box::new(Quiet), events, Arc::new(|| {}));
    feed(&mut module, vec![Event::Session(session), Event::Online(true)]);
    module
}

/// The prompt for a sign-in code, its last answer refused.
#[must_use]
pub fn code_prompt() -> Prompt {
    Prompt {
        title: "Enter the code".into(),
        detail: "Telegram sent a code to the Telegram app on your other devices.".into(),
        fields: vec![Field { label: "Code".into(), hint: "12345".into(), secret: false }],
        action: "Continue".into(),
        link: None,
        problem: "That code is not right.".into(),
        back: true,
        alternative: "Send the code by SMS instead".into(),
    }
}

/// The prompt for the API id and hash.
#[must_use]
pub fn app_prompt() -> Prompt {
    Prompt {
        title: "Connect Telegram".into(),
        detail: "This copy of Adelite was built without a Telegram API id. Create one under “API development \
                 tools” and paste it here to try it; a release is built with its own and never asks."
            .into(),
        fields: vec![
            Field { label: "API id".into(), hint: "1234567".into(), secret: false },
            Field { label: "API hash".into(), hint: "32 letters and digits".into(), secret: true },
        ],
        action: "Continue".into(),
        link: Some(("my.telegram.org".into(), "https://my.telegram.org/apps".into())),
        problem: String::new(),
        back: false,
        alternative: String::new(),
    }
}

fn chat(id: ChatId, title: &str, kind: Kind, order: i64, last: (&str, &str, i64, bool)) -> Chat {
    Chat {
        id,
        title: title.into(),
        kind,
        places: vec![Place { list: List::Main, order, pinned: false }],
        can_send: kind != Kind::Channel,
        joined: true,
        last: Some(Preview {
            id: 900,
            sender: last.0.into(),
            text: last.1.into(),
            date: unix_now() as i64 - last.2,
            outgoing: last.3,
        }),
        ..Chat::default()
    }
}

fn message(chat: ChatId, id: i64, who: (i64, &str), ago: i64, body: Body) -> Message {
    Message {
        id,
        chat,
        sender: who.0,
        sender_name: who.1.into(),
        outgoing: who.0 == 1,
        date: unix_now() as i64 - ago,
        body,
        ..Message::default()
    }
}

fn words(text: &str) -> Body {
    Body::Text(Text::plain(text))
}

fn photo(file: Option<i64>, caption: &str) -> Body {
    Body::Photo { file, full: file, width: 1280, height: 800, caption: Text::plain(caption) }
}

fn reaction(emoji: &str, count: u32, chosen: bool) -> Reaction {
    Reaction { emoji: emoji.into(), count, chosen }
}

const ME: (i64, &str) = (1, "You");
const MARA_LIND: (i64, &str) = (7, "Mara Lind");
const JONAS: (i64, &str) = (8, "Jonas Weber");
const LI: (i64, &str) = (11, "Li Wei");

/// The chats of the demo's account, and the lists they are kept in.
fn account(picture: Option<&str>) -> Vec<Event> {
    let (hour, day) = (3_600, 86_400);
    let filed = |mut chat: Chat, order| {
        chat.places.push(Place { list: WORK, order, pinned: false });
        chat
    };
    let design = Chat {
        places: vec![
            Place { list: List::Main, order: 99, pinned: true },
            Place { list: WORK, order: 9, pinned: false },
        ],
        status: "4 members".into(),
        read_outbox: 60,
        read_inbox: 61,
        unread: 2,
        admin: true,
        last: Some(Preview {
            id: 71,
            sender: "Li".into(),
            text: "@jerry can you check the copy before Thursday?".into(),
            date: unix_now() as i64 - 90,
            outgoing: false,
        }),
        ..chat(DESIGN, "Design", Kind::Group, 99, ("", "", 0, false))
    };
    let chats = vec![
        design,
        Chat {
            unread: 3,
            status: "online".into(),
            photo: picture.map(|_| 501),
            ..chat(
                MARA,
                "Mara Lind",
                Kind::Private,
                90,
                ("", "Can you look at the contract before noon?", 14 * 60, false),
            )
        },
        Chat {
            unread: 12,
            muted: true,
            status: "1204 subscribers".into(),
            ..chat(
                NEWS,
                "Company News",
                Kind::Channel,
                80,
                ("", "Photo · The new office opens Monday", 2 * hour, false),
            )
        },
        Chat {
            read_outbox: 900,
            draft: "I'll send the invoice tomorrow, it still needs".into(),
            ..chat(8, "Jonas Weber", Kind::Private, 70, ("", "Thanks, got it", 5 * hour, true))
        },
        filed(
            Chat {
                mentioned: true,
                unread: 1,
                ..chat(-61, "Backend", Kind::Group, 60, ("Li", "@jerry the migration is green", day + hour, false))
            },
            8,
        ),
        filed(
            Chat {
                forum: true,
                unread: 5,
                status: "38 members".into(),
                ..chat(PRODUCT, "Product", Kind::Group, 55, ("Ana", "The roadmap for Q1 is up", day + 2 * hour, false))
            },
            7,
        ),
        chat(1, "Saved Messages", Kind::Private, 50, ("", "q3-forecast.xlsx", 2 * day, true)),
        Chat { marked_unread: true, ..chat(9, "Ana Sousa", Kind::Private, 40, ("", "Voice message", 4 * day, false)) },
        chat(-62, "Support rota", Kind::Group, 30, ("You", "I can take Friday", 9 * day, true)),
        chat(10, "Tomás Ortega", Kind::Private, 20, ("", "See you at the offsite", 40 * day, false)),
    ];
    let list = |list, name: &str, unread| ListInfo { list, name: name.into(), unread };
    let mut events = vec![Event::Lists(vec![
        list(List::Main, "All", 6),
        list(WORK, "Work", 3),
        list(List::Folder(2), "Friends", 0),
        list(List::Community(1), "Adelite HQ", 0),
        list(List::Archive, "Archive", 0),
    ])];
    events.extend(chats.into_iter().map(Event::Chat));
    for file in [501, 502, 504] {
        events.extend(picture.map(|path| Event::File { file, path: path.to_owned() }));
    }
    events.push(Event::Typing { chat: MARA, who: "Mara".into(), on: true });
    events
}

/// The messages of the demo's group.
fn design_history(picture: Option<&str>) -> Vec<Message> {
    let (hour, day) = (3_600, 86_400);
    let shown = picture.map(|_| 502);
    let link = "https://adelite.example/specs/onboarding";
    let rich = Text {
        text: format!("The spec is at {link}, and the short version is: one screen, no tour."),
        spans: vec![
            Span { range: 15..15 + link.len(), style: Style::Link(link.into()) },
            Span { range: 55 + link.len() - 14..55 + link.len() + 2, style: Style::Bold },
        ],
    };
    let page = LinkPreview {
        url: link.into(),
        site: "Adelite".into(),
        title: "Onboarding: one screen, no tour".into(),
        description: "What a new teammate sees in their first minute, and everything we decided to leave out.".into(),
        photo: picture.map(|_| 504),
        width: 640,
        height: 280,
    };
    let code = Text {
        text: "Run cargo test --workspace before you push, it takes ten seconds now.".into(),
        spans: vec![Span { range: 4..26, style: Style::Code }],
    };
    let ask = Text {
        text: "@jerry can you check the copy before Thursday?".into(),
        spans: vec![Span { range: 0..6, style: Style::Mention("@jerry".into()) }],
    };
    let option = |text: &str, votes, percent, chosen| PollOption { text: text.into(), votes, percent, chosen };
    let poll = Poll {
        question: "When do we ship the new onboarding?".into(),
        options: vec![
            option("Thursday", 3, 60, true),
            option("Friday, after the migration", 2, 40, false),
            option("Next week", 0, 0, false),
        ],
        voters: 5,
        ..Poll::default()
    };
    let video = Body::Video {
        motion: Motion::Video,
        still: shown,
        file: Some(510),
        width: 1280,
        height: 720,
        seconds: 72,
        caption: Text::plain("The walkthrough from last week"),
    };
    let voice =
        Body::Audio { file: Some(511), voice: true, title: String::new(), seconds: 24, caption: Text::default() };
    let file = Body::File {
        file: Some(503),
        name: "onboarding-metrics.pdf".into(),
        size: 2_482_176,
        caption: Text::default(),
    };
    vec![
        message(DESIGN, 10, MARA_LIND, day + 3 * hour, Body::Service("Mara Lind added Li Wei".into())),
        message(
            DESIGN,
            20,
            MARA_LIND,
            day + 2 * hour,
            words("Morning! I put the onboarding flow in the shared folder."),
        ),
        Message {
            link: Some(page),
            pinned: true,
            reactions: vec![reaction("👍", 3, true), reaction("🔥", 1, false)],
            ..message(DESIGN, 21, MARA_LIND, day + 2 * hour - 20, Body::Text(rich))
        },
        Message {
            edited: true,
            ..message(
                DESIGN,
                30,
                ME,
                day + hour,
                words("Read it on the train. I like that the first screen asks for nothing."),
            )
        },
        Message {
            reply_to: Some(21),
            ..message(
                DESIGN,
                40,
                JONAS,
                3 * hour,
                words("One screen is brave. What happens for people who were invited by a teammate?"),
            )
        },
        Message {
            album: 5,
            ..message(DESIGN, 41, JONAS, 3 * hour - 30, photo(shown, "Here is how it looks with the invite banner."))
        },
        Message { album: 5, ..message(DESIGN, 42, JONAS, 3 * hour - 30, photo(shown, "")) },
        Message { album: 5, ..message(DESIGN, 43, JONAS, 3 * hour - 30, photo(shown, "")) },
        message(DESIGN, 50, LI, 2 * hour, Body::Text(code)),
        message(DESIGN, 51, LI, 2 * hour - 40, file),
        message(DESIGN, 52, LI, 2 * hour - 60, voice),
        message(DESIGN, 55, MARA_LIND, hour + 600, Body::Poll(poll)),
        Message {
            reply_to: Some(40),
            ..message(
                DESIGN,
                60,
                ME,
                hour,
                words("They skip it and land in the team's space. I'll add that to the spec."),
            )
        },
        Message { forwarded_from: "Company News".into(), ..message(DESIGN, 61, ME, hour - 30, video) },
        Message {
            reactions: vec![reaction("🎉", 2, false)],
            ..message(DESIGN, 70, JONAS, 130, words("Shipping it Thursday then 🚀"))
        },
        Message { mentions_me: true, ..message(DESIGN, 71, LI, 90, Body::Text(ask)) },
    ]
}

/// A signed-in chat app with a list of chats and, when `open`, the group
/// conversation on the canvas. `picture` is an image file to show as a
/// photo and a face, if there is one.
#[must_use]
pub fn signed_in(open: bool, picture: Option<&str>) -> ChatModule {
    let mut module = at(Session::Ready);
    feed(&mut module, account(picture));
    if !open {
        return module;
    }
    module.open((DESIGN, 0), None);
    let history = design_history(picture);
    let pinned = history.iter().find(|message| message.pinned).cloned().map(Box::new);
    feed(
        &mut module,
        vec![
            Event::History { chat: DESIGN, topic: 0, page: Page::Around, messages: history },
            Event::Top { chat: DESIGN, message: pinned },
            Event::File { file: 503, path: "C:\\files\\onboarding-metrics.pdf".into() },
            // One of the user's own, still on its way.
            Event::Message(Message {
                delivery: Delivery::Sending,
                ..message(DESIGN, 9_000, ME, 5, words("Thursday works."))
            }),
        ],
    );
    module
}

/// The conversation further up: a link's page, an album, a file, a
/// recording and a poll, with the mouse over a message.
#[must_use]
pub fn media(picture: Option<&str>) -> ChatModule {
    let mut module = signed_in(true, picture);
    module.jump = Some(Jump::Message(41));
    module
}

/// The conversation with a draft in its message box: an answer being
/// written, with a file to go along.
#[must_use]
pub fn writing(picture: Option<&str>) -> ChatModule {
    let mut module = signed_in(true, picture);
    module.reply_to = Some(70);
    module.attached.push("C:\\work\\release-plan.pdf".into());
    module.draft.insert(
        "Thursday works. I'll have the copy ready by Wednesday evening,\nand Li can run the migration after that.",
    );
    module
}

/// The list with a chat's menu open over it.
#[must_use]
pub fn chat_menu(picture: Option<&str>) -> ChatModule {
    let mut module = signed_in(true, picture);
    module.overlay = Overlay::Menu(Menu::new(Target::Chat(MARA), (150.0, 150.0)));
    module
}

/// The conversation with a message's menu open: what to answer it with, and what to do with it.
#[must_use]
pub fn message_menu(picture: Option<&str>) -> ChatModule {
    let mut module = signed_in(true, picture);
    module.overlay = Overlay::Menu(Menu::new(Target::Message(70), (720.0, 470.0)));
    module
}

/// Messages picked to be passed on together.
#[must_use]
pub fn picked(picture: Option<&str>) -> ChatModule {
    let mut module = signed_in(true, picture);
    module.selected = vec![60, 70];
    module
}

/// Words picked with the mouse, from the middle of one message into the next with words.
#[must_use]
pub fn marked(picture: Option<&str>) -> ChatModule {
    let mut module = signed_in(true, picture);
    module.marked = Some(crate::marked::Marked { anchor: (60, 9), focus: (70, 8), dragging: false, moved: true });
    module
}

/// What a search found: chats, then messages.
#[must_use]
pub fn search(picture: Option<&str>) -> ChatModule {
    let mut module = signed_in(true, picture);
    module.search.insert("ship");
    module.focus = crate::Focus::Search;
    module.searched = ("ship".into(), None);
    module.search_token = 1;
    module.store.search(1);
    let found = vec![
        message(DESIGN, 70, JONAS, 130, words("Shipping it Thursday then 🚀")),
        message(MARA, 300, MARA_LIND, 86_400, words("Did the laptops ship yet? Finance is asking.")),
        message(NEWS, 400, (50, "Company News"), 3 * 86_400, words("We shipped 4.2: here is what is in it.")),
        message(-61, 500, ME, 9 * 86_400, words("Ship it behind the flag and we watch the dashboards.")),
    ];
    feed(&mut module, vec![Event::Found { token: 1, chats: vec![-62], messages: found }]);
    module
}

/// A forum: its topics in the sidebar, one of them on the canvas.
#[must_use]
pub fn topics(picture: Option<&str>) -> ChatModule {
    let mut module = signed_in(false, picture);
    let topic = |id, name: &str, order, last: (&str, &str, i64)| Topic {
        chat: PRODUCT,
        id,
        name: name.into(),
        order,
        last: Some(Preview {
            id: 900 + id,
            sender: last.0.into(),
            text: last.1.into(),
            date: unix_now() as i64 - last.2,
            outgoing: false,
        }),
        ..Topic::default()
    };
    let topics = vec![
        Topic { pinned: true, ..topic(1, "General", 50, ("Mara", "Welcome, Tomás!", 4_000)) },
        Topic { unread: 4, color: 0x6F_B9F0, ..topic(2, "Roadmap", 90, ("Ana", "The roadmap for Q1 is up", 90_000)) },
        Topic {
            unread: 1,
            mentioned: true,
            color: 0xFB_6F5F,
            ..topic(3, "Bugs", 80, ("Li", "@jerry is this yours?", 120_000))
        },
        Topic {
            closed: true,
            color: 0x8E_EE98,
            ..topic(4, "Release 4.1", 70, ("Jonas", "Shipped. Closing this.", 600_000))
        },
        Topic {
            muted: true,
            color: 0xFF_D67E,
            ..topic(5, "Ideas", 60, ("Tomás", "What if the rail could be on the left?", 900_000))
        },
    ];
    module.pick(PRODUCT);
    feed(&mut module, vec![Event::Topics { chat: PRODUCT, topics }]);
    module.open((PRODUCT, 2), None);
    let ana = (12, "Ana Sousa");
    let said = |id, who, ago, text: &str| Message { topic: 2, ..message(PRODUCT, id, who, ago, words(text)) };
    let history = vec![
        said(880, ana, 93_000, "The roadmap for Q1 is up. Three themes: speed, mail, and documents."),
        said(881, JONAS, 92_000, "Mail before documents? I thought we said the other way round."),
        said(882, ana, 91_000, "We did. Then half the team asked when they can stop switching windows for mail."),
        said(883, ME, 90_500, "Fair. Mail first, as long as the rail stays this quiet."),
        said(902, ana, 90_000, "The roadmap for Q1 is up"),
    ];
    feed(&mut module, vec![Event::History { chat: PRODUCT, topic: 2, page: Page::Around, messages: history }]);
    module
}

/// The group's panel beside its conversation: what it is, and who is in it.
#[must_use]
pub fn info(picture: Option<&str>) -> ChatModule {
    let mut module = signed_in(true, picture);
    module.show_info(DESIGN);
    let member = |who: (i64, &str), status: &str, role| Member {
        id: who.0,
        name: who.1.into(),
        photo: (who.0 == 7).then_some(501).filter(|_| picture.is_some()),
        status: status.into(),
        role,
    };
    let members = vec![
        member(MARA_LIND, "online", Role::Owner),
        member(JONAS, "last seen recently", Role::Admin),
        member(LI, "last seen at 09:12", Role::Member),
        member((1, "Jerry"), "online", Role::Member),
    ];
    let about = Info {
        chat: DESIGN,
        about: "Specs, reviews and what ships next. Decisions are written down in the pinned message.".into(),
        link: "https://t.me/+Zk3vQ9d1xYpmN2Fk".into(),
        members: 4,
        can_invite: true,
        can_edit: true,
        ..Info::default()
    };
    feed(&mut module, vec![Event::Info(about), Event::Members { chat: DESIGN, query: String::new(), members }]);
    module
}

/// The group's card on the pictures that were sent in it.
#[must_use]
pub fn info_media(picture: Option<&str>) -> ChatModule {
    let mut module = info(picture);
    module.show_tab(crate::info::Tab::Sent(adelite_chat::Shared::Media));
    let shown = picture.map(|_| 502);
    let clip = |id, seconds| Message {
        body: Body::Video {
            motion: Motion::Video,
            still: shown,
            file: Some(510),
            width: 1280,
            height: 720,
            seconds,
            caption: Text::default(),
        },
        ..message(DESIGN, id, JONAS, 86_400, words(""))
    };
    let mut messages: Vec<Message> =
        (0..14).map(|n| message(DESIGN, 300 - n, JONAS, 86_400, photo(shown, ""))).collect();
    messages.insert(2, clip(350, 72));
    messages.insert(7, clip(340, 9));
    let kind = adelite_chat::Shared::Media;
    feed(&mut module, vec![Event::Shared { chat: DESIGN, kind, before: 0, messages }]);
    module
}

/// The group's card on the files that were sent in it.
#[must_use]
pub fn info_files(picture: Option<&str>) -> ChatModule {
    let mut module = info(picture);
    let kind = adelite_chat::Shared::Files;
    module.show_tab(crate::info::Tab::Sent(kind));
    let file = |id, name: &str, size, ago| {
        let body = Body::File { file: Some(id), name: name.into(), size, caption: Text::default() };
        message(DESIGN, id, LI, ago, body)
    };
    let messages = vec![
        file(503, "onboarding-metrics.pdf", 2_482_176, 7_000),
        file(520, "invite-banner-v3.fig", 18_400_000, 90_000),
        file(521, "copy-review.docx", 48_200, 200_000),
        file(522, "q3-forecast.xlsx", 310_000, 900_000),
    ];
    let path = Event::File { file: 503, path: r"C:\files\onboarding-metrics.pdf".into() };
    feed(&mut module, vec![Event::Shared { chat: DESIGN, kind, before: 0, messages }, path]);
    module
}

/// The group's card with its form open: its name and its words about itself, being changed.
#[must_use]
pub fn info_edit(picture: Option<&str>) -> ChatModule {
    let mut module = info(picture);
    module.edit_card();
    module
}

/// The card a message's next chat is picked on.
#[must_use]
pub fn forward(picture: Option<&str>) -> ChatModule {
    let mut module = signed_in(true, picture);
    module.overlay = Overlay::Picker(Picker::new(Purpose::Forward { from: DESIGN, messages: vec![70] }));
    module
}

/// A picture of an album, looked at alone.
#[must_use]
pub fn viewer(picture: Option<&str>) -> ChatModule {
    let mut module = signed_in(true, picture);
    let item = module.store.thread((DESIGN, 0)).and_then(|thread| thread.items.get(thread.find(42)?));
    if let Some(viewer) = item.and_then(|item| Viewer::of(item, 1)) {
        module.overlay = Overlay::Viewer(viewer);
    }
    module
}

/// A channel: posts with how often they were seen, their comments, and buttons under them.
#[must_use]
pub fn channel(picture: Option<&str>) -> ChatModule {
    let mut module = signed_in(false, picture);
    module.open((NEWS, 0), None);
    let news = (50, "Company News");
    let (hour, day) = (3_600, 86_400);
    let page = LinkPreview {
        url: "https://adelite.example/blog/4-2".into(),
        site: "Adelite Blog".into(),
        title: "What is new in 4.2".into(),
        description: "Folders, topics and a quieter rail: the release notes, with pictures.".into(),
        photo: None,
        width: 0,
        height: 0,
    };
    let button = |label: &str, press| Button { label: label.into(), press };
    let posts = vec![
        Message {
            views: 1_840,
            comments: Some(12),
            link: Some(page),
            reactions: vec![reaction("🎉", 48, true), reaction("❤", 19, false), reaction("👍", 7, false)],
            ..message(
                NEWS,
                380,
                news,
                3 * day,
                words("We shipped 4.2: here is what is in it.\nhttps://adelite.example/blog/4-2"),
            )
        },
        Message {
            views: 1_204,
            comments: Some(3),
            ..message(NEWS, 390, news, day, photo(picture.map(|_| 502), "The new office opens Monday. Bring a mug."))
        },
        Message {
            views: 312,
            comments: Some(0),
            buttons: vec![
                vec![
                    button("I'm coming", Press::Data("yes".into())),
                    button("Can't make it", Press::Data("no".into())),
                ],
                vec![button("Add to calendar", Press::Link("https://adelite.example/offsite.ics".into()))],
            ],
            ..message(
                NEWS,
                400,
                news,
                2 * hour,
                words("Offsite on the 14th: are you in? Trains leave at 8:10 from Central."),
            )
        },
    ];
    feed(&mut module, vec![Event::History { chat: NEWS, topic: 0, page: Page::Older, messages: posts }]);
    module
}
