//! The view's commands as TDLib's requests.
//!
//! Nothing here talks to TDLib: a command and what is remembered go in,
//! JSON comes out, so every request can be looked at in a test. A request
//! whose answer matters carries an `@extra`, which TDLib hands back with
//! the answer: an object whose `for` says what the answer is for, with
//! whatever is needed to make an event of it. A request without one is
//! something the user asked to be done: only its failing is told.

use std::path::Path;

use adelite_chat::{ChatId, Command, List, MessageId, Page, SenderId, Shared, TopicId};
use serde_json::{Value, json};

use crate::map::{Cache, Peer};

/// How many chats one [`Command::LoadChats`] asks for.
const CHATS_PAGE: i64 = 40;
/// How many messages one [`Command::LoadHistory`] asks for.
const HISTORY_PAGE: i64 = 50;
/// How many topics of a forum are asked for at once.
pub const TOPICS_PAGE: i64 = 100;
/// How many topics of a forum are asked for at most: a list longer than that is searched, not read.
pub const TOPICS_MOST: i64 = 300;
/// How many chats, messages and people a search asks for.
const FOUND_CHATS: i64 = 30;
const FOUND_MESSAGES: i64 = 30;
const FOUND_IN_CHAT: i64 = 50;
const FOUND_PEOPLE: i64 = 200;
/// How many messages one page of a chat's pictures, files or links holds.
const SHARED_PAGE: i64 = 60;
/// How many files go into one album.
const ALBUM: usize = 10;
/// Muted for longer than a year is muted for good.
const FOREVER: i64 = i32::MAX as i64;

/// The `@extra` of a request whose answer, and whose failing, nobody waits for.
pub fn quiet() -> Value {
    json!({"for": "quiet"})
}

/// A topic as the `MessageTopic` TDLib knows it by; null for the chat itself.
/// The other way round is `map::topic`.
pub fn topic(topic: TopicId) -> Value {
    match topic {
        0 => Value::Null,
        1.. => json!({"@type": "messageTopicForum", "forum_topic_id": topic}),
        _ => json!({"@type": "messageTopicThread", "message_thread_id": -topic}),
    }
}

/// A list as TDLib's `ChatList`. A community is none of TDLib's lists.
fn chat_list(list: List) -> Option<Value> {
    match list {
        List::Main => Some(json!({"@type": "chatListMain"})),
        List::Archive => Some(json!({"@type": "chatListArchive"})),
        List::Folder(id) => Some(json!({"@type": "chatListFolder", "chat_folder_id": id})),
        List::Community(_) => None,
    }
}

/// A page as the word an `@extra` carries it by.
fn page_name(page: Page) -> &'static str {
    match page {
        Page::Older => "older",
        Page::Newer => "newer",
        Page::Around => "around",
    }
}

/// The page an `@extra` carries.
pub fn page(name: &str) -> Page {
    match name {
        "newer" => Page::Newer,
        "around" => Page::Around,
        _ => Page::Older,
    }
}

/// The `offset` and `limit` that make TDLib answer with a page seen from
/// `from`. It counts from the message asked from towards older ones; a
/// negative offset starts that many messages later, the first of which is
/// the message itself.
pub fn window(from: MessageId, page: Page) -> (i64, i64) {
    match page {
        // The newest messages have nothing after them.
        _ if from == 0 => (0, HISTORY_PAGE),
        Page::Older => (0, HISTORY_PAGE),
        Page::Newer => (-HISTORY_PAGE - 1, HISTORY_PAGE + 1),
        Page::Around => (-HISTORY_PAGE / 2, HISTORY_PAGE),
    }
}

/// The request for a page of a chat's messages, or of a topic's.
fn history(chat: ChatId, of: TopicId, from: MessageId, page: Page) -> Value {
    let (offset, limit) = window(from, page);
    let mut request = match of {
        0 => json!({"@type": "getChatHistory", "chat_id": chat, "only_local": false}),
        1.. => json!({"@type": "getForumTopicHistory", "chat_id": chat, "forum_topic_id": of}),
        _ => json!({"@type": "getMessageThreadHistory", "chat_id": chat, "message_id": -of}),
    };
    request["from_message_id"] = json!(from);
    request["offset"] = json!(offset);
    request["limit"] = json!(limit);
    request["@extra"] = json!({"for": "history", "chat": chat, "topic": of, "page": page_name(page), "from": from});
    request
}

/// The request for a chat's newest pinned message.
pub fn top(chat: ChatId) -> Value {
    json!({"@type": "getChatPinnedMessage", "chat_id": chat, "@extra": {"for": "top", "chat": chat}})
}

/// The request for the topics of a forum that come after the ones seen:
/// `after` is what the last answer said to go on from (date, message, topic).
pub fn topics(chat: ChatId, after: (i64, i64, i64), seen: i64) -> Value {
    json!({
        "@type": "getForumTopics", "chat_id": chat, "query": "", "offset_date": after.0,
        "offset_message_id": after.1, "offset_forum_topic_id": after.2, "limit": TOPICS_PAGE,
        "@extra": {"for": "topics", "chat": chat, "seen": seen, "after": [after.0, after.1, after.2]},
    })
}

/// The request for one topic of a forum, in full.
pub fn one_topic(chat: ChatId, id: TopicId) -> Value {
    json!({"@type": "getForumTopic", "chat_id": chat, "forum_topic_id": id, "@extra": {"for": "topic", "chat": chat}})
}

/// The request for a chat's full information, by what the chat is with;
/// `link` is an invitation just made, which the answer may not know yet.
pub fn full_info(cache: &Cache, chat: ChatId, purpose: &str, link: &str) -> Value {
    let mut request = match cache.peer(chat) {
        Peer::User(id) => json!({"@type": "getUserFullInfo", "user_id": id}),
        Peer::Group(id) => json!({"@type": "getBasicGroupFullInfo", "basic_group_id": id}),
        Peer::Supergroup(id) => json!({"@type": "getSupergroupFullInfo", "supergroup_id": id}),
    };
    request["@extra"] = json!({"for": purpose, "chat": chat, "link": link});
    request
}

/// The request for a community's chats.
pub fn community(id: i64) -> Value {
    json!({"@type": "getCommunityFullInfo", "community_id": id, "@extra": {"for": "community", "id": id}})
}

/// The request for the chat with one person; its answer is `purpose`'s.
pub fn private_chat(user: SenderId, purpose: &Value) -> Value {
    json!({"@type": "createPrivateChat", "user_id": user, "force": false, "@extra": purpose})
}

/// The request for whoever goes by a name; the chat is gone to.
pub fn named(name: &str) -> Value {
    json!({"@type": "searchPublicChat", "username": name, "@extra": {"for": "goto"}})
}

/// The request for an unread message that mentions the user: the oldest
/// (asked from the first message there can be, towards newer ones; TDLib
/// wants a limit above what the offset skips), or the newest where it
/// will not answer that.
pub fn mention(chat: ChatId, oldest: bool) -> Value {
    let (from, offset, limit) = if oldest { (1 << 20, -1, 2) } else { (0, 0, 1) };
    json!({
        "@type": "searchChatMessages", "chat_id": chat, "query": "", "from_message_id": from, "offset": offset,
        "limit": limit, "filter": {"@type": "searchMessagesFilterUnreadMention"},
        "@extra": {"for": "mention", "chat": chat, "oldest": oldest},
    })
}

/// What the administrators this app makes may do: the everyday running of
/// a chat, without making more of themselves or hiding who they are.
fn administrator(channel: bool) -> Value {
    json!({
        "@type": "chatMemberStatusAdministrator",
        "rights": {
            "@type": "chatAdministratorRights", "can_manage_chat": true, "can_change_info": true,
            // Posting is what running a channel is; elsewhere these two mean nothing.
            "can_post_messages": channel, "can_edit_messages": channel, "can_delete_messages": true,
            "can_invite_users": true, "can_restrict_members": true, "can_pin_messages": true,
            "can_manage_topics": true, "can_manage_video_chats": true, "can_promote_members": false,
            "is_anonymous": false,
        },
    })
}

/// Whether a file is sent as a photo: by its name.
fn is_picture(path: &str) -> bool {
    let extension = Path::new(path).extension().and_then(|extension| extension.to_str());
    extension.is_some_and(|extension| ["png", "jpg", "jpeg", "webp"].contains(&extension.to_lowercase().as_str()))
}

/// The files as they are sent: pictures together and other files
/// together (Telegram mixes them in no album), ten at most at once. The
/// kind the first file is of goes first. With each group, whether it is pictures.
pub fn albums(paths: &[String]) -> Vec<(bool, Vec<&str>)> {
    let first = paths.first().is_some_and(|path| is_picture(path));
    let mut groups = Vec::new();
    for pictures in [first, !first] {
        let of_kind: Vec<&str> = paths.iter().map(String::as_str).filter(|path| is_picture(path) == pictures).collect();
        groups.extend(of_kind.chunks(ALBUM).map(|group| (pictures, group.to_vec())));
    }
    groups
}

/// Where a link or a mention leads ([`Command::Follow`]).
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// Whoever goes by this name (without its sign).
    Name(String),
    /// The user with this id.
    User(SenderId),
    /// An address of Telegram's own, which TDLib reads.
    Telegram(String),
    /// A page outside.
    Outside(String),
    /// Nowhere.
    Nothing,
}

/// Reads where a [`Command::Follow`] leads from what it was given.
pub fn target(address: &str) -> Target {
    let address = address.trim();
    if let Some(name) = address.strip_prefix('@') {
        let fits = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        return if fits { Target::Name(name.to_owned()) } else { Target::Nothing };
    }
    if let Some(id) = address.strip_prefix("user:") {
        return id.parse().ok().filter(|id| *id > 0).map_or(Target::Nothing, Target::User);
    }
    let lower = address.to_ascii_lowercase();
    if address.is_empty() {
        Target::Nothing
    } else if lower.starts_with("tg:") {
        Target::Telegram(address.to_owned())
    } else if let Some(rest) = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")) {
        if telegrams(rest) { Target::Telegram(address.to_owned()) } else { Target::Outside(address.to_owned()) }
    } else if !lower.contains(':') && telegrams(&lower) {
        // "t.me/adelite", as people write it.
        Target::Telegram(format!("https://{address}"))
    } else {
        Target::Outside(address.to_owned())
    }
}

/// Whether an address, lower-cased and without its scheme, is on one of Telegram's own hosts.
fn telegrams(address: &str) -> bool {
    let host = address.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.strip_prefix("www.").unwrap_or(host);
    let on = |own: &str| host.strip_suffix(own).is_some_and(|sub| sub.is_empty() || sub.ends_with('.'));
    // A name before an @ is there to look like the host, which is what comes after it.
    !host.contains('@') && ["t.me", "telegram.me", "telegram.dog"].iter().any(|own| on(own))
}

/// A command as the requests that carry it out, in the order they are
/// sent. `marked_up` makes a `formattedText` of what the user typed.
/// Signing in and out are not asked here: they are the client's own.
pub fn requests(command: Command, cache: &Cache, marked_up: &dyn Fn(&str) -> Value) -> Vec<Value> {
    let reply =
        |to: Option<MessageId>| to.map(|message| json!({"@type": "inputMessageReplyToMessage", "message_id": message}));
    let person = |user: SenderId| json!({"@type": "messageSenderUser", "user_id": user});
    let one = |request: Value| vec![request];
    match command {
        Command::Answer(_) | Command::Back | Command::Alternative | Command::LogOut => Vec::new(),
        Command::LoadChats(List::Community(id)) => one(community(id)),
        Command::LoadChats(list) => one(json!({
            "@type": "loadChats", "chat_list": chat_list(list), "limit": CHATS_PAGE, "@extra": quiet(),
        })),
        Command::Open(chat) => vec![json!({"@type": "openChat", "chat_id": chat, "@extra": quiet()}), top(chat)],
        Command::Close(chat) => one(json!({"@type": "closeChat", "chat_id": chat, "@extra": quiet()})),
        Command::LoadHistory { chat, topic, from, page } => one(history(chat, topic, from, page)),
        Command::LoadTopics(chat) => one(topics(chat, (0, 0, 0), 0)),
        Command::Send { chat, topic: to, text, reply_to } => one(json!({
            "@type": "sendMessage", "chat_id": chat, "topic_id": topic(to), "reply_to": reply(reply_to),
            "input_message_content": {"@type": "inputMessageText", "text": marked_up(&text), "clear_draft": true},
        })),
        Command::SendFiles { chat, topic: to, paths, caption, reply_to } => {
            // The words go under the first file of the first group.
            let mut caption = (!caption.is_empty()).then(|| marked_up(&caption));
            let mut content = |path: &str, picture: bool| {
                let file = json!({"@type": "inputFileLocal", "path": path});
                let caption = caption.take();
                if picture {
                    json!({"@type": "inputMessagePhoto", "photo": {"@type": "inputPhoto", "photo": file}, "caption": caption})
                } else {
                    json!({"@type": "inputMessageDocument", "document": {"@type": "inputDocument", "document": file}, "caption": caption})
                }
            };
            let send = |(pictures, group): (bool, Vec<&str>)| {
                let mut contents: Vec<Value> = group.iter().map(|path| content(path, pictures)).collect();
                let mut request = json!({"chat_id": chat, "topic_id": topic(to), "reply_to": reply(reply_to)});
                if contents.len() == 1 {
                    request["@type"] = json!("sendMessage");
                    request["input_message_content"] = contents.remove(0);
                } else {
                    request["@type"] = json!("sendMessageAlbum");
                    request["input_message_contents"] = Value::Array(contents);
                }
                request
            };
            albums(&paths).into_iter().map(send).collect()
        }
        Command::Edit { chat, message, text, caption: true } => one(json!({
            "@type": "editMessageCaption", "chat_id": chat, "message_id": message, "caption": marked_up(&text),
        })),
        Command::Edit { chat, message, text, caption: false } => one(json!({
            "@type": "editMessageText", "chat_id": chat, "message_id": message,
            "input_message_content": {"@type": "inputMessageText", "text": marked_up(&text)},
        })),
        Command::Delete { chat, messages, for_all } => {
            one(json!({"@type": "deleteMessages", "chat_id": chat, "message_ids": messages, "revoke": for_all}))
        }
        Command::Forward { from, mut messages, to } => {
            // TDLib takes them oldest first, each once.
            messages.sort_unstable();
            messages.dedup();
            one(json!({
                "@type": "forwardMessages", "chat_id": to, "from_chat_id": from, "message_ids": messages,
                "send_copy": false, "remove_caption": false,
            }))
        }
        Command::Pin { chat, message, on: true } => one(json!({
            "@type": "pinChatMessage", "chat_id": chat, "message_id": message, "disable_notification": true,
            "only_for_self": false,
        })),
        Command::Pin { chat, message, on: false } => {
            one(json!({"@type": "unpinChatMessage", "chat_id": chat, "message_id": message}))
        }
        Command::React { chat, message, emoji, on } => {
            let mut request = json!({
                "chat_id": chat, "message_id": message, "reaction_type": {"@type": "reactionTypeEmoji", "emoji": emoji},
            });
            if on {
                request["@type"] = json!("addMessageReaction");
                request["is_big"] = json!(false);
                request["update_recent_reactions"] = json!(true);
            } else {
                request["@type"] = json!("removeMessageReaction");
            }
            one(request)
        }
        Command::Vote { chat, message, options } => {
            one(json!({"@type": "setPollAnswer", "chat_id": chat, "message_id": message, "option_ids": options}))
        }
        Command::Press { chat, message, data } => one(json!({
            "@type": "getCallbackQueryAnswer", "chat_id": chat, "message_id": message,
            "payload": {"@type": "callbackQueryPayloadData", "data": data}, "@extra": {"for": "press"},
        })),
        Command::Read { chat, messages } => one(json!({
            "@type": "viewMessages", "chat_id": chat, "message_ids": messages, "force_read": false, "@extra": quiet(),
        })),
        Command::MarkUnread { chat, on: true } => {
            one(json!({"@type": "toggleChatIsMarkedAsUnread", "chat_id": chat, "is_marked_as_unread": true}))
        }
        Command::MarkUnread { chat, on: false } => {
            let mut requests = one(json!({
                "@type": "toggleChatIsMarkedAsUnread", "chat_id": chat, "is_marked_as_unread": false, "@extra": quiet(),
            }));
            // Reading its latest message reads all before it, wherever the chat is shown.
            requests.extend(cache.last_message(chat).map(|last| {
                json!({"@type": "viewMessages", "chat_id": chat, "message_ids": [last], "force_read": true, "@extra": quiet()})
            }));
            requests.push(json!({"@type": "readAllChatMentions", "chat_id": chat, "@extra": quiet()}));
            requests.push(json!({"@type": "readAllChatReactions", "chat_id": chat, "@extra": quiet()}));
            requests
        }
        Command::Mute { chat, on } => {
            // Its other settings stay as they are; only muting stops following the default.
            let mut settings = cache.notify(chat);
            settings["use_default_mute_for"] = json!(false);
            settings["mute_for"] = json!(if on { FOREVER } else { 0 });
            one(json!({"@type": "setChatNotificationSettings", "chat_id": chat, "notification_settings": settings}))
        }
        Command::PinChat { chat, list, on } => chat_list(list)
            .map(|list| json!({"@type": "toggleChatIsPinned", "chat_list": list, "chat_id": chat, "is_pinned": on}))
            .into_iter()
            .collect(),
        Command::Move { chat, to } => chat_list(to)
            .map(|list| json!({"@type": "addChatToList", "chat_id": chat, "chat_list": list}))
            .into_iter()
            .collect(),
        Command::Join(chat) => one(json!({"@type": "joinChat", "chat_id": chat, "@extra": {"for": "joined"}})),
        Command::Leave(chat) => one(match cache.peer(chat) {
            // A conversation with a person is dropped on this side only.
            Peer::User(_) => forget(chat, Value::Null),
            // A basic group stays in the list after leaving it: it is dropped once that is done.
            Peer::Group(_) => json!({"@type": "leaveChat", "chat_id": chat, "@extra": {"for": "left", "chat": chat}}),
            Peer::Supergroup(_) => json!({"@type": "leaveChat", "chat_id": chat}),
        }),
        Command::Draft { chat, topic: of, text } => {
            let draft = (!text.is_empty()).then(|| {
                json!({
                    "@type": "draftMessage",
                    "content": {"@type": "draftMessageContentText", "text": {"@type": "formattedText", "text": text}},
                })
            });
            one(json!({
                "@type": "setChatDraftMessage", "chat_id": chat, "topic_id": topic(of), "draft_message": draft,
                "@extra": quiet(),
            }))
        }
        Command::Typing { chat, topic: of } => one(json!({
            "@type": "sendChatAction", "chat_id": chat, "topic_id": topic(of), "action": {"@type": "chatActionTyping"},
            "@extra": quiet(),
        })),
        Command::Search { query, chat, token } => search(query.trim(), chat, token),
        Command::Follow(address) => match target(&address) {
            Target::Name(name) => one(named(&name)),
            Target::User(user) => one(private_chat(user, &json!({"for": "goto"}))),
            Target::Telegram(url) => {
                one(json!({"@type": "getInternalLinkType", "link": url, "@extra": {"for": "link", "url": url}}))
            }
            // Answered without asking: the client says where to go.
            Target::Outside(_) | Target::Nothing => Vec::new(),
        },
        Command::Fetch { chat, messages } => one(json!({
            "@type": "getMessages", "chat_id": chat, "message_ids": messages, "@extra": {"for": "fetched", "chat": chat},
        })),
        Command::Shared { chat, kind, before } => one(json!({
            "@type": "searchChatMessages", "chat_id": chat, "query": "", "from_message_id": before, "offset": 0,
            "limit": SHARED_PAGE, "filter": {"@type": shared_filter(kind)},
            "@extra": {"for": "shared", "chat": chat, "kind": shared_name(kind), "before": before},
        })),
        Command::NextMention(chat) => one(mention(chat, true)),
        Command::Comments { chat, message } => one(json!({
            "@type": "getMessageThread", "chat_id": chat, "message_id": message, "@extra": {"for": "comments"},
        })),
        Command::Download(file) => one(json!({
            "@type": "downloadFile", "file_id": file, "priority": 1, "offset": 0, "limit": 0,
            "synchronous": false, "@extra": quiet(),
        })),
        Command::Info(chat) => one(full_info(cache, chat, "info", "")),
        Command::Invite(chat) => one(full_info(cache, chat, "invite", "")),
        Command::Members { chat, query } => one(json!({
            "@type": "searchChatMembers", "chat_id": chat, "query": query, "limit": FOUND_PEOPLE,
            "@extra": {"for": "members", "chat": chat, "query": query},
        })),
        Command::People(query) => {
            let extra = json!({"for": "people", "query": query});
            one(if query.trim().is_empty() {
                json!({"@type": "getContacts", "@extra": extra})
            } else {
                json!({"@type": "searchContacts", "query": query, "limit": FOUND_PEOPLE, "@extra": extra})
            })
        }
        Command::AddMember { chat, user } => one(json!({
            // A basic group shows someone new what was said before, as Telegram's own apps do.
            "@type": "addChatMember", "chat_id": chat, "user_id": user, "forward_limit": 100, "@extra": {"for": "added"},
        })),
        // Out of the chat, and free to come back: a ban is another thing.
        Command::RemoveMember { chat, user } => one(json!({
            "@type": "setChatMemberStatus", "chat_id": chat, "member_id": person(user),
            "status": {"@type": "chatMemberStatusLeft"},
        })),
        Command::Promote { chat, user, on } => {
            let status =
                if on { administrator(cache.is_channel(chat)) } else { json!({"@type": "chatMemberStatusMember"}) };
            one(json!({"@type": "setChatMemberStatus", "chat_id": chat, "member_id": person(user), "status": status}))
        }
        Command::Rename { chat, title } => one(json!({"@type": "setChatTitle", "chat_id": chat, "title": title})),
        Command::Describe { chat, about } => {
            one(json!({"@type": "setChatDescription", "chat_id": chat, "description": about}))
        }
        Command::NewGroup { title, members } => one(json!({
            "@type": "createNewBasicGroupChat", "user_ids": members, "title": title, "@extra": {"for": "goto"},
        })),
        Command::Talk(user) => one(private_chat(user, &json!({"for": "goto"}))),
    }
}

/// The request that drops a chat from the lists with its messages, on this side only.
pub fn forget(chat: ChatId, extra: Value) -> Value {
    let mut request =
        json!({"@type": "deleteChatHistory", "chat_id": chat, "remove_from_chat_list": true, "revoke": false});
    if !extra.is_null() {
        request["@extra"] = extra;
    }
    request
}

/// TDLib's filter for one kind of what was sent in a chat.
fn shared_filter(kind: Shared) -> &'static str {
    match kind {
        Shared::Media => "searchMessagesFilterPhotoAndVideo",
        Shared::Files => "searchMessagesFilterDocument",
        Shared::Links => "searchMessagesFilterUrl",
    }
}

/// A kind of what was sent in a chat, as a request remembers it.
fn shared_name(kind: Shared) -> &'static str {
    match kind {
        Shared::Media => "media",
        Shared::Files => "files",
        Shared::Links => "links",
    }
}

/// The kind a request remembered ([`shared_name`]).
pub fn shared(name: &str) -> Shared {
    match name {
        "files" => Shared::Files,
        "links" => Shared::Links,
        _ => Shared::Media,
    }
}

/// The requests of a search: in one chat its messages; everywhere the
/// chats known here, the ones Telegram knows, the public ones, the
/// user's contacts and messages, each answering on its own.
fn search(query: &str, chat: Option<ChatId>, token: u64) -> Vec<Value> {
    let found = json!({"for": "found", "token": token});
    if query.is_empty() {
        return Vec::new();
    }
    if let Some(chat) = chat {
        return vec![json!({
            "@type": "searchChatMessages", "chat_id": chat, "query": query, "from_message_id": 0, "offset": 0,
            "limit": FOUND_IN_CHAT, "@extra": found,
        })];
    }
    vec![
        json!({"@type": "searchChats", "query": query, "limit": FOUND_CHATS, "@extra": found}),
        json!({"@type": "searchChatsOnServer", "query": query, "limit": FOUND_CHATS, "@extra": found}),
        json!({"@type": "searchPublicChats", "query": query, "@extra": found}),
        json!({"@type": "searchContacts", "query": query, "limit": FOUND_CHATS, "@extra": found}),
        json!({"@type": "searchMessages", "query": query, "offset": "", "limit": FOUND_MESSAGES, "@extra": found}),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Value {
        json!({"@type": "formattedText", "text": text})
    }

    fn asked(command: Command) -> Vec<Value> {
        requests(command, &Cache::default(), &plain)
    }

    fn only(command: Command) -> Value {
        let mut requests = asked(command);
        assert_eq!(requests.len(), 1, "{requests:?}");
        requests.remove(0)
    }

    #[test]
    fn a_topic_is_a_forums_or_a_thread_both_ways() {
        for id in [0, 5, -4096, i64::from(i32::MAX), -(1 << 40)] {
            assert_eq!(crate::map::topic(&topic(id)), id);
        }
        assert_eq!(topic(0), Value::Null);
        assert_eq!(topic(5), json!({"@type": "messageTopicForum", "forum_topic_id": 5}));
        assert_eq!(topic(-4096), json!({"@type": "messageTopicThread", "message_thread_id": 4096}));
    }

    #[test]
    fn a_page_of_history_is_asked_from_the_right_side() {
        assert_eq!(window(0, Page::Older), (0, 50));
        assert_eq!(window(900, Page::Older), (0, 50));
        assert_eq!(window(900, Page::Newer), (-51, 51), "the message itself, then fifty after it");
        assert_eq!(window(900, Page::Around), (-25, 50));
        assert_eq!(
            (window(0, Page::Newer), window(0, Page::Around)),
            ((0, 50), (0, 50)),
            "nothing is newer than the newest"
        );
        for (_, page) in [(0, Page::Older), (1, Page::Newer), (2, Page::Around)] {
            let (offset, limit) = window(900, page);
            assert!((-99..=0).contains(&offset) && limit >= -offset && limit <= 100, "what TDLib takes");
            assert_eq!(super::page(page_name(page)), page);
        }
        let chat = only(Command::LoadHistory { chat: 7, topic: 0, from: 900, page: Page::Around });
        assert_eq!(
            (chat["@type"].as_str(), chat["from_message_id"].as_i64(), chat["offset"].as_i64()),
            (Some("getChatHistory"), Some(900), Some(-25))
        );
        assert_eq!(chat["@extra"], json!({"for": "history", "chat": 7, "topic": 0, "page": "around", "from": 900}));
        let forum = only(Command::LoadHistory { chat: 7, topic: 5, from: 0, page: Page::Older });
        assert_eq!(
            (forum["@type"].as_str(), forum["forum_topic_id"].as_i64()),
            (Some("getForumTopicHistory"), Some(5))
        );
        let thread = only(Command::LoadHistory { chat: 7, topic: -4096, from: 0, page: Page::Older });
        assert_eq!(
            (thread["@type"].as_str(), thread["message_id"].as_i64()),
            (Some("getMessageThreadHistory"), Some(4096))
        );
    }

    #[test]
    fn files_go_as_albums_of_one_kind() {
        let paths: Vec<String> =
            ["a.pdf", "b.JPG", "c.png", "d.txt", "e", "f.webp"].iter().map(|path| (*path).to_owned()).collect();
        assert_eq!(albums(&paths), [(false, vec!["a.pdf", "d.txt", "e"]), (true, vec!["b.JPG", "c.png", "f.webp"])]);
        let many: Vec<String> = (0..23).map(|index| format!("{index}.jpeg")).collect();
        let sizes: Vec<(bool, usize)> =
            albums(&many).iter().map(|(pictures, group)| (*pictures, group.len())).collect();
        assert_eq!(sizes, [(true, 10), (true, 10), (true, 3)]);
        assert!(albums(&[]).is_empty());
        // One file is a message; several of a kind an album; the words go under the first.
        let sent =
            asked(Command::SendFiles { chat: 7, topic: 5, paths, caption: "the plan".into(), reply_to: Some(9) });
        assert_eq!(sent.len(), 2);
        let (documents, pictures) = (&sent[0], &sent[1]);
        assert_eq!(
            (documents["@type"].as_str(), pictures["@type"].as_str()),
            (Some("sendMessageAlbum"), Some("sendMessageAlbum"))
        );
        let contents = documents["input_message_contents"].as_array().expect("contents");
        assert_eq!(contents[0]["caption"], plain("the plan"));
        assert_eq!(
            (contents[1]["caption"].is_null(), contents[0]["@type"].as_str()),
            (true, Some("inputMessageDocument"))
        );
        assert_eq!(pictures["input_message_contents"][0]["photo"]["photo"]["path"], "b.JPG");
        assert!(pictures["input_message_contents"][0]["caption"].is_null());
        assert_eq!(
            (pictures["topic_id"]["forum_topic_id"].as_i64(), pictures["reply_to"]["message_id"].as_i64()),
            (Some(5), Some(9))
        );
        let one = only(Command::SendFiles {
            chat: 7,
            topic: 0,
            paths: vec!["x.png".into()],
            caption: String::new(),
            reply_to: None,
        });
        assert_eq!(
            (one["@type"].as_str(), one["input_message_content"]["@type"].as_str()),
            (Some("sendMessage"), Some("inputMessagePhoto"))
        );
        assert!(
            one["topic_id"].is_null() && one["reply_to"].is_null() && one["input_message_content"]["caption"].is_null()
        );
    }

    #[test]
    fn a_link_leads_into_telegram_or_outside() {
        let telegram = |url: &str| Target::Telegram(url.to_owned());
        let outside = |url: &str| Target::Outside(url.to_owned());
        assert_eq!(target("@adelite"), Target::Name("adelite".into()));
        assert_eq!(target(" user:42 "), Target::User(42));
        for nowhere in ["", "  ", "@", "@two words", "user:", "user:x", "user:-4"] {
            assert_eq!(target(nowhere), Target::Nothing, "{nowhere:?}");
        }
        for own in [
            "https://t.me/adelite/12",
            "http://telegram.me/+abc",
            "HTTPS://T.ME/c/1/2",
            "https://www.telegram.dog/x",
            "https://adelite.t.me/",
            "tg://resolve?domain=adelite",
            "TG:resolve",
            "https://t.me",
        ] {
            assert_eq!(target(own), telegram(own), "{own:?}");
        }
        assert_eq!(target("t.me/adelite"), telegram("https://t.me/adelite"));
        // Hosts that only look like Telegram's, and everything else, are pages outside.
        for other in [
            "https://t.me.example/x",
            "https://not-t.me/x",
            "https://t.me@evil.example/",
            "https://example.com/t.me/x",
            "mailto:mara@t.me",
            "ftp://t.me/x",
            "adelite.example",
            "https://xt.me/",
        ] {
            assert_eq!(target(other), outside(other), "{other:?}");
        }
        assert!(asked(Command::Follow("https://adelite.example".into())).is_empty(), "nothing to ask TDLib");
        let link = only(Command::Follow("https://t.me/adelite/12".into()));
        assert_eq!(
            (link["@type"].as_str(), link["link"].as_str()),
            (Some("getInternalLinkType"), Some("https://t.me/adelite/12"))
        );
        assert_eq!(only(Command::Follow("@adelite".into()))["username"], "adelite");
        assert_eq!(only(Command::Follow("user:42".into()))["@type"], "createPrivateChat");
    }

    #[test]
    fn a_search_asks_everywhere_at_once_or_in_one_chat() {
        let everywhere = asked(Command::Search { query: " roof ".into(), chat: None, token: 3 });
        let kinds: Vec<&str> = everywhere.iter().map(|request| request["@type"].as_str().unwrap_or_default()).collect();
        assert_eq!(
            kinds,
            ["searchChats", "searchChatsOnServer", "searchPublicChats", "searchContacts", "searchMessages"]
        );
        assert!(everywhere.iter().all(|request| request["query"] == "roof" && request["@extra"]["token"] == 3));
        let inside = only(Command::Search { query: "roof".into(), chat: Some(7), token: u64::MAX });
        assert_eq!(
            (inside["@type"].as_str(), inside["chat_id"].as_i64(), inside["@extra"]["token"].as_u64()),
            (Some("searchChatMessages"), Some(7), Some(u64::MAX))
        );
        assert!(asked(Command::Search { query: "  ".into(), chat: None, token: 1 }).is_empty());
        // What was sent in a chat, one kind at a time, a page from where the last one ended.
        let files = only(Command::Shared { chat: 7, kind: Shared::Files, before: 90 });
        assert_eq!(
            (files["filter"]["@type"].as_str(), files["from_message_id"].as_i64(), files["query"].as_str()),
            (Some("searchMessagesFilterDocument"), Some(90), Some(""))
        );
        assert_eq!(files["@extra"], json!({"for": "shared", "chat": 7, "kind": "files", "before": 90}));
        for kind in [Shared::Media, Shared::Files, Shared::Links] {
            assert_eq!(shared(shared_name(kind)), kind);
        }
    }

    #[test]
    fn commands_become_the_requests_that_carry_them_out() {
        let kind = |command: Command| only(command)["@type"].as_str().unwrap_or_default().to_owned();
        let send = only(Command::Send { chat: 7, topic: -4096, text: "hello".into(), reply_to: Some(9) });
        assert_eq!(send["topic_id"]["message_thread_id"], 4096);
        assert_eq!(
            (send["reply_to"]["message_id"].as_i64(), &send["input_message_content"]["text"]),
            (Some(9), &plain("hello"))
        );
        assert_eq!(kind(Command::Edit { chat: 7, message: 9, text: "x".into(), caption: true }), "editMessageCaption");
        assert_eq!(kind(Command::Edit { chat: 7, message: 9, text: "x".into(), caption: false }), "editMessageText");
        assert_eq!(only(Command::Delete { chat: 7, messages: vec![9], for_all: false })["revoke"], false);
        let forward = only(Command::Forward { from: 7, messages: vec![30, 10, 20, 10], to: 8 });
        assert_eq!((forward["chat_id"].as_i64(), &forward["message_ids"]), (Some(8), &json!([10, 20, 30])));
        assert_eq!(kind(Command::Pin { chat: 7, message: 9, on: true }), "pinChatMessage");
        assert_eq!(kind(Command::Pin { chat: 7, message: 9, on: false }), "unpinChatMessage");
        assert_eq!(kind(Command::React { chat: 7, message: 9, emoji: "👍".into(), on: true }), "addMessageReaction");
        let taken = only(Command::React { chat: 7, message: 9, emoji: "👍".into(), on: false });
        assert_eq!(
            (taken["@type"].as_str(), taken["reaction_type"]["emoji"].as_str(), taken.get("is_big")),
            (Some("removeMessageReaction"), Some("👍"), None)
        );
        assert_eq!(only(Command::Vote { chat: 7, message: 9, options: vec![0, 2] })["option_ids"], json!([0, 2]));
        assert_eq!(only(Command::Press { chat: 7, message: 9, data: "eWVz".into() })["payload"]["data"], "eWVz");
        // Lists: a community is none of TDLib's, so nothing is pinned or moved there.
        assert_eq!(only(Command::LoadChats(List::Folder(4)))["chat_list"]["chat_folder_id"], 4);
        assert_eq!(kind(Command::LoadChats(List::Community(3))), "getCommunityFullInfo");
        assert_eq!(only(Command::Move { chat: 7, to: List::Archive })["chat_list"]["@type"], "chatListArchive");
        assert!(asked(Command::PinChat { chat: 7, list: List::Community(3), on: true }).is_empty());
        assert_eq!(kind(Command::PinChat { chat: 7, list: List::Main, on: true }), "toggleChatIsPinned");
        // Leaving: a person's chat is dropped here, a group is left.
        let dropped = only(Command::Leave(7));
        assert_eq!(
            (dropped["@type"].as_str(), dropped["revoke"].as_bool(), dropped["remove_from_chat_list"].as_bool()),
            (Some("deleteChatHistory"), Some(false), Some(true))
        );
        assert_eq!(only(Command::Leave(-60))["@extra"], json!({"for": "left", "chat": -60}));
        assert_eq!(only(Command::Leave(-1_000_000_000_050)).get("@extra"), None);
        assert_eq!(kind(Command::Info(-60)), "getBasicGroupFullInfo");
        assert_eq!(only(Command::Info(-1_000_000_000_050))["supergroup_id"], 50);
        assert_eq!(only(Command::Invite(7))["@extra"]["for"], "invite");
        let draft = only(Command::Draft { chat: 7, topic: 0, text: "**hm".into() });
        assert_eq!(draft["draft_message"]["content"]["text"]["text"], "**hm", "kept as it was typed");
        assert!(only(Command::Draft { chat: 7, topic: 0, text: String::new() })["draft_message"].is_null());
        assert_eq!(kind(Command::People(" ".into())), "getContacts");
        assert_eq!(kind(Command::People("ma".into())), "searchContacts");
        let removed = only(Command::RemoveMember { chat: -60, user: 8 });
        assert_eq!(
            (removed["member_id"]["user_id"].as_i64(), removed["status"]["@type"].as_str()),
            (Some(8), Some("chatMemberStatusLeft"))
        );
        let promoted = only(Command::Promote { chat: -60, user: 8, on: true });
        let rights = &promoted["status"]["rights"];
        assert_eq!(
            (
                rights["can_pin_messages"].as_bool(),
                rights["can_promote_members"].as_bool(),
                rights["is_anonymous"].as_bool()
            ),
            (Some(true), Some(false), Some(false))
        );
        assert_eq!(rights["can_post_messages"], false, "posting is a channel's");
        assert_eq!(
            only(Command::Promote { chat: -60, user: 8, on: false })["status"]["@type"],
            "chatMemberStatusMember"
        );
        assert!(asked(Command::LogOut).is_empty() && asked(Command::Answer(Vec::new())).is_empty());
    }

    #[test]
    fn what_is_remembered_shapes_a_request() {
        let mut cache = Cache::default();
        let mut out = Vec::new();
        let last =
            json!({"id": 44, "chat_id": 7, "date": 5, "content": {"@type": "messageText", "text": {"text": "x"}}});
        cache.chat(&json!({"id": 7, "type": {"@type": "chatTypePrivate", "user_id": 7}, "last_message": last,
            "notification_settings": {"use_default_mute_for": true, "mute_for": 0, "use_default_sound": false, "sound_id": "55"}}), &mut out);
        cache.chat(&json!({"id": -1_000_000_000_050_i64, "type": {"@type": "chatTypeSupergroup", "supergroup_id": 50, "is_channel": true}}), &mut out);
        let ask = |command: Command| requests(command, &cache, &plain);
        // Muting changes that and nothing else about how the chat asks for attention.
        let muted = &ask(Command::Mute { chat: 7, on: true })[0]["notification_settings"];
        assert_eq!((muted["use_default_mute_for"].as_bool(), muted["mute_for"].as_i64()), (Some(false), Some(FOREVER)));
        assert_eq!((muted["use_default_sound"].as_bool(), muted["sound_id"].as_str()), (Some(false), Some("55")));
        let unmuted = &ask(Command::Mute { chat: 8, on: false })[0]["notification_settings"];
        assert_eq!(
            (
                unmuted["use_default_mute_for"].as_bool(),
                unmuted["mute_for"].as_i64(),
                unmuted["use_default_sound"].as_bool()
            ),
            (Some(false), Some(0), Some(true))
        );
        // Read through and through: unmarked, its latest message read, and what mentions or answers the user.
        let read = ask(Command::MarkUnread { chat: 7, on: false });
        let kinds: Vec<&str> = read.iter().map(|request| request["@type"].as_str().unwrap_or_default()).collect();
        assert_eq!(
            kinds,
            ["toggleChatIsMarkedAsUnread", "viewMessages", "readAllChatMentions", "readAllChatReactions"]
        );
        assert_eq!((&read[1]["message_ids"], read[1]["force_read"].as_bool()), (&json!([44]), Some(true)));
        assert_eq!(ask(Command::MarkUnread { chat: 8, on: false }).len(), 3, "no message to read");
        assert_eq!(ask(Command::MarkUnread { chat: 7, on: true }).len(), 1);
        let promoted = &ask(Command::Promote { chat: -1_000_000_000_050, user: 8, on: true })[0];
        assert_eq!(promoted["status"]["rights"]["can_post_messages"], true);
        let opened = ask(Command::Open(7));
        assert_eq!(
            (opened[0]["@type"].as_str(), opened[1]["@type"].as_str()),
            (Some("openChat"), Some("getChatPinnedMessage"))
        );
        assert_eq!(mention(7, true)["from_message_id"], 1 << 20);
        assert_eq!(
            (mention(7, false)["from_message_id"].as_i64(), mention(7, false)["offset"].as_i64()),
            (Some(0), Some(0))
        );
    }
}
