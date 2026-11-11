//! What TDLib says, as events for the view: its updates, and the answers
//! to our requests, which the `@extra` they come back with tells apart
//! (`request`). Some of them lead to a request in turn (a link that turns
//! out to be a message's, the next page of a forum's topics): those are
//! handed back to be sent. Nothing here talks to TDLib, so all of it can
//! be looked at in a test. Signing in is the client's own.

use adelite_chat::{ChatId, Event, Message, Page, Topic, TopicId};
use serde_json::{Value, json};

use crate::map::{self, Cache, Peer, flag, int, items, kind, string};
use crate::request::{self, quiet};

/// Said when Telegram would not put someone into a chat.
const NOT_ADDED: &str = "Not everyone could be added: some people only let their contacts add them. Send them an \
                         invitation instead.";

/// Maps one update or answer of TDLib's into events (`out`) and the
/// requests that follow from it (`ask`). `now` is the time in Unix seconds.
pub fn take(cache: &mut Cache, said: &Value, now: i64, out: &mut Vec<Event>, ask: &mut Vec<Value>) {
    if !said["@extra"].is_object() {
        update(cache, said, out, ask);
    } else if kind(said) == "error" {
        failed(said, out, ask);
    } else {
        answered(cache, said, now, out, ask);
    }
    out.extend(cache.lists());
}

/// Maps one update of TDLib's into events.
fn update(cache: &mut Cache, update: &Value, out: &mut Vec<Event>, ask: &mut Vec<Value>) {
    let chat = int(&update["chat_id"]);
    match kind(update) {
        "updateOption" if string(&update["name"]) == "my_id" => cache.me = int(&update["value"]["value"]),
        "updateConnectionState" => out.push(Event::Online(kind(&update["state"]) == "connectionStateReady")),
        "updateUser" => {
            let chat = cache.user(&update["user"], out);
            out.extend(chat.map(Event::Chat));
        }
        "updateUserStatus" => out.extend(cache.user_status(update).map(Event::Chat)),
        // What a chat says about itself changed, here or elsewhere: whoever shows it shows it as it is now.
        "updateUserFullInfo" => {
            let info = cache.info_of(Peer::User(int(&update["user_id"])), &update["user_full_info"]);
            out.extend(info.map(Event::Info));
        }
        "updateBasicGroupFullInfo" => {
            let info = cache.info_of(Peer::Group(int(&update["basic_group_id"])), &update["basic_group_full_info"]);
            out.extend(info.map(Event::Info));
        }
        "updateSupergroupFullInfo" => {
            let info = cache.info_of(Peer::Supergroup(int(&update["supergroup_id"])), &update["supergroup_full_info"]);
            out.extend(info.map(Event::Info));
        }
        "updateBasicGroup" => out.extend(cache.group(&update["basic_group"], false).map(Event::Chat)),
        "updateSupergroup" => out.extend(cache.group(&update["supergroup"], true).map(Event::Chat)),
        "updateNewChat" => {
            let chat = cache.chat(&update["chat"], out);
            out.extend(chat.map(Event::Chat));
        }
        "updateChatFolders" => cache.folders(update),
        "updateUnreadChatCount" => cache.unread(update),
        "updateScopeNotificationSettings" => cache.scope(&update["scope"], &update["notification_settings"], out),
        "updateCommunity" => {
            if cache.community(&update["community"], out) {
                ask.push(request::community(int(&update["community"]["id"])));
            }
        }
        "updateCommunityFullInfo" => {
            cache.community_info(int(&update["community_id"]), &update["community_full_info"], out);
        }
        "updateNewMessage" => {
            let message = cache.message(&update["message"], out);
            let topic = cache.topic_message(&update["message"], &message);
            out.extend(topic.map(told));
            out.push(Event::Message(message));
        }
        "updateMessageSendSucceeded" | "updateMessageSendFailed" => {
            let message = cache.message(&update["message"], out);
            out.push(Event::Sent { was: int(&update["old_message_id"]), message });
            let error = string(&update["error"]["message"]);
            if !error.is_empty() {
                out.push(Event::Problem(format!("A message could not be sent: {}", friendly(error))));
            }
        }
        "updateMessageContent" => {
            let (message, content) = (int(&update["message_id"]), &update["new_content"]);
            cache.shows_poll(chat, message, content);
            // What a service message says of its sender is not said again here.
            let body = cache.body(content, "", out);
            out.push(Event::Body { chat, message, body });
        }
        "updateMessageEdited" => out.push(Event::Edited { chat, message: int(&update["message_id"]) }),
        "updateMessageInteractionInfo" => out.push(cache.counts(update)),
        "updateMessageIsPinned" => {
            out.push(Event::Pinned { chat, message: int(&update["message_id"]), on: flag(&update["is_pinned"]) });
            // Which message the chat now shows at its top is TDLib's to say.
            ask.push(request::top(chat));
        }
        "updatePoll" => cache.polled(&update["poll"], out),
        // Messages only dropped from TDLib's memory are still there.
        "updateDeleteMessages" if flag(&update["is_permanent"]) && !flag(&update["from_cache"]) => {
            let messages = items(&update["message_ids"]).map(int).collect();
            out.push(Event::Deleted { chat, messages });
        }
        "updateChatAction" => out.push(Event::Typing {
            chat,
            who: cache.first_name(&update["sender_id"]),
            on: kind(&update["action"]) == "chatActionTyping",
        }),
        "updateFile" => out.extend(map::downloaded(&update["file"])),
        "updateForumTopicInfo" => {
            let info = &update["info"];
            match cache.topic_info(info) {
                Some(topic) => out.push(told(topic)),
                None => ask.extend(news(cache, int(&info["chat_id"]), int(&info["forum_topic_id"]))),
            }
        }
        "updateForumTopic" => match cache.topic_update(update) {
            Some((topic, moved)) => {
                // How much of it is still unread, only the whole topic says.
                ask.extend(moved.then(|| request::one_topic(chat, topic.id)));
                out.push(told(topic));
            }
            None => ask.extend(news(cache, chat, int(&update["forum_topic_id"]))),
        },
        // A request without an `@extra` is something the user asked to be done.
        "error" => out.push(Event::Problem(friendly(string(&update["message"])))),
        _ => {
            let chat = cache.chat_update(update, out);
            out.extend(chat.map(Event::Chat));
        }
    }
}

/// The request for a topic that is news, where the view follows the chat's topics.
fn news(cache: &Cache, chat: ChatId, topic: TopicId) -> Option<Value> {
    (topic != 0 && cache.follows_topics(chat)).then(|| request::one_topic(chat, topic))
}

/// Maps the answer to a request of ours into events, or into the request
/// that follows from it.
fn answered(cache: &mut Cache, answer: &Value, now: i64, out: &mut Vec<Event>, ask: &mut Vec<Value>) {
    let extra = &answer["@extra"];
    let chat = int(&extra["chat"]);
    match string(&extra["for"]) {
        "history" => {
            let (page, from) = (request::page(string(&extra["page"])), int(&extra["from"]));
            let mut messages = messages(cache, answer, out);
            // TDLib starts the page after a message at the message itself.
            if page == Page::Newer {
                messages.retain(|message| message.id > from);
            }
            out.push(Event::History { chat, topic: int(&extra["topic"]), page, messages });
        }
        "top" => {
            let message = (kind(answer) == "message").then(|| Box::new(cache.message(answer, out)));
            out.push(Event::Top { chat, message });
        }
        "topics" => topics(cache, answer, out, ask),
        "topic" => {
            let topic = cache.topic(answer, out);
            out.extend(topic.map(told));
        }
        "found" => found(cache, answer, out, ask),
        "shared" => {
            let (kind, before) = (request::shared(string(&extra["kind"])), int(&extra["before"]));
            // TDLib starts the page at the message asked from.
            let mut messages = messages(cache, answer, out);
            messages.retain(|message| before == 0 || message.id < before);
            out.push(Event::Shared { chat, kind, before, messages });
        }
        "goto" => match kind(answer) {
            "chat" => {
                let chat = cache.tell(answer, out);
                out.extend((chat != 0).then_some(Event::Goto { chat, topic: 0, message: 0 }));
            }
            "createdBasicGroupChat" => {
                out.push(Event::Goto { chat: int(&answer["chat_id"]), topic: 0, message: 0 });
                out.extend(not_added(&answer["failed_to_add_members"]));
            }
            _ => {}
        },
        "link" => link(answer, out, ask),
        "message-link" => {
            let (chat, message) = (int(&answer["chat_id"]), &answer["message"]);
            let topic = match map::topic(&answer["topic_id"]) {
                0 => map::topic(&message["topic_id"]),
                topic => topic,
            };
            out.push(match chat {
                // A chat the user cannot look into: Telegram's page says what it is.
                0 => Event::Browse(string(&extra["url"]).to_owned()),
                _ => Event::Goto { chat, topic, message: int(&message["id"]) },
            });
        }
        "invite-link" => out.push(match int(&answer["chat_id"]) {
            // Following a link must not make the user a member of anything.
            0 => Event::Notice(format!("“{}” has to be joined from Telegram for now.", string(&answer["title"]))),
            chat => Event::Goto { chat, topic: 0, message: 0 },
        }),
        "fetched" => {
            let messages = messages(cache, answer, out);
            out.push(Event::Fetched { chat, messages });
        }
        "mention" => {
            // TDLib answers newest first: the oldest of them is the one to read next.
            let unread = items(&answer["messages"]).filter(|message| message.is_object());
            match unread.min_by_key(|message| int(&message["id"])) {
                Some(message) => {
                    let (topic, message) = (map::topic(&message["topic_id"]), int(&message["id"]));
                    out.push(Event::Goto { chat, topic, message });
                }
                None if flag(&extra["oldest"]) => ask.push(request::mention(chat, false)),
                None => {}
            }
        }
        "comments" => {
            let (chat, thread) = (int(&answer["chat_id"]), int(&answer["message_thread_id"]));
            if chat != 0 && thread > 0 {
                out.push(Event::Goto { chat, topic: -thread, message: 0 });
            }
        }
        "info" | "invite" => {
            let mut info = cache.info(chat, answer);
            if info.link.is_empty() {
                string(&extra["link"]).clone_into(&mut info.link);
            }
            let group = !matches!(cache.peer(chat), Peer::User(_));
            if group && info.link.is_empty() && string(&extra["for"]) == "invite" {
                // No address leads here yet. One that exists is left alone: replacing it revokes it.
                let made = json!({"for": "invited", "chat": chat});
                ask.push(json!({"@type": "replacePrimaryChatInviteLink", "chat_id": chat, "@extra": made}));
            } else {
                out.push(Event::Info(info));
            }
        }
        // The chat's full information may not have heard of the new address yet: it goes along.
        "invited" => ask.push(request::full_info(cache, chat, "info", string(&answer["invite_link"]))),
        "members" => {
            let (query, members) = (string(&extra["query"]).to_owned(), cache.members(answer, now));
            out.push(Event::Members { chat, query, members });
        }
        "people" => {
            let (query, people) = (string(&extra["query"]).to_owned(), cache.people(answer, now));
            out.push(Event::People { query, people });
        }
        "press" => {
            let (text, url) = (string(&answer["text"]), string(&answer["url"]));
            out.extend((!text.is_empty()).then(|| Event::Notice(text.to_owned())));
            out.extend((!url.is_empty()).then(|| Event::Browse(url.to_owned())));
        }
        "joined" => out.extend(match kind(answer) {
            "chatJoinResultRequestSent" => Some(Event::Notice("Your request to join was sent.".to_owned())),
            "chatJoinResultDeclined" => Some(Event::Problem("You were not let into this chat.".to_owned())),
            "chatJoinResultGuardBotApprovalRequired" => {
                Some(Event::Notice("A bot decides who gets into this chat. Join it from Telegram for now.".to_owned()))
            }
            _ => None,
        }),
        "added" => out.extend(not_added(answer)),
        // The basic group is left: now it can go from the list.
        "left" => ask.push(request::forget(chat, quiet())),
        "scope" => cache.scope(&extra["scope"], answer, out),
        "communities" => ask.extend(items(&answer["community_ids"]).map(|id| request::community(int(id)))),
        "community" => cache.community_info(int(&extra["id"]), answer, out),
        _ => {}
    }
}

/// A page of a forum's topics came: tells them, and asks for the next
/// while TDLib says where to go on from.
fn topics(cache: &mut Cache, answer: &Value, out: &mut Vec<Event>, ask: &mut Vec<Value>) {
    let extra = &answer["@extra"];
    let chat = int(&extra["chat"]);
    let topics: Vec<Topic> = items(&answer["topics"]).filter_map(|topic| cache.topic(topic, out)).collect();
    let seen = int(&extra["seen"]).saturating_add(topics.len() as i64);
    let next = |field: &str| int(&answer[field]);
    let after = (next("next_offset_date"), next("next_offset_message_id"), next("next_offset_forum_topic_id"));
    let before = (int(&extra["after"][0]), int(&extra["after"][1]), int(&extra["after"][2]));
    // An answer that leads nowhere new would be asked for again and again.
    let more = !topics.is_empty() && seen < request::TOPICS_MOST && after != (0, 0, 0) && after != before;
    out.push(Event::Topics { chat, topics });
    ask.extend(more.then(|| request::topics(chat, after, seen)));
}

/// Part of what a search found came. People are found as users: each is
/// asked for the chat with them, which answers here again.
fn found(cache: &mut Cache, answer: &Value, out: &mut Vec<Event>, ask: &mut Vec<Value>) {
    let extra = &answer["@extra"];
    let (chats, messages) = match kind(answer) {
        "chats" => (cache.known(answer), Vec::new()),
        "chat" => (Some(cache.tell(answer, out)).filter(|chat| *chat != 0).into_iter().collect(), Vec::new()),
        "foundMessages" | "foundChatMessages" => (Vec::new(), messages(cache, answer, out)),
        "users" => return ask.extend(items(&answer["user_ids"]).map(|user| request::private_chat(int(user), extra))),
        _ => return,
    };
    out.push(Event::Found { token: extra["token"].as_u64().unwrap_or(0), chats, messages });
}

/// TDLib read an address of Telegram's own: goes where it leads, as far as this app can.
fn link(answer: &Value, out: &mut Vec<Event>, ask: &mut Vec<Value>) {
    let url = string(&answer["@extra"]["url"]);
    match kind(answer) {
        "internalLinkTypeMessage" => ask.push(json!({
            "@type": "getMessageLinkInfo", "url": string(&answer["url"]),
            "@extra": {"for": "message-link", "url": url},
        })),
        "internalLinkTypePublicChat" => ask.push(request::named(string(&answer["chat_username"]))),
        "internalLinkTypeChatInvite" => ask.push(json!({
            "@type": "checkChatInviteLink", "invite_link": string(&answer["invite_link"]),
            "@extra": {"for": "invite-link"},
        })),
        _ => out.push(Event::Browse(url.to_owned())),
    }
}

/// A request of ours failed: what that means depends on what it was for.
fn failed(error: &Value, out: &mut Vec<Event>, ask: &mut Vec<Value>) {
    let extra = &error["@extra"];
    let chat = int(&extra["chat"]);
    let query = || string(&extra["query"]).to_owned();
    match string(&extra["for"]) {
        // The view stops waiting for what will not come.
        "history" => out.push(Event::History {
            chat,
            topic: int(&extra["topic"]),
            page: request::page(string(&extra["page"])),
            messages: Vec::new(),
        }),
        // No message is pinned.
        "top" => out.push(Event::Top { chat, message: None }),
        "topics" => out.push(Event::Topics { chat, topics: Vec::new() }),
        "fetched" => out.push(Event::Fetched { chat, messages: Vec::new() }),
        "shared" => out.push(Event::Shared {
            chat,
            kind: request::shared(string(&extra["kind"])),
            before: int(&extra["before"]),
            messages: Vec::new(),
        }),
        "members" => out.push(Event::Members { chat, query: query(), members: Vec::new() }),
        "people" => out.push(Event::People { query: query(), people: Vec::new() }),
        // Not Telegram's after all, or nothing TDLib can read: the page itself says more.
        "link" | "message-link" => out.push(Event::Browse(string(&extra["url"]).to_owned())),
        "mention" if flag(&extra["oldest"]) => ask.push(request::mention(chat, false)),
        // Nothing the user asked for by name: no more chats to load, a picture that is gone,
        // a part of a search that found nothing, a community that will not load.
        "quiet" | "found" | "topic" | "mention" | "scope" | "communities" | "community" => {}
        _ => out.push(Event::Problem(friendly(string(&error["message"])))),
    }
}

/// The messages of an answer that holds some; a message TDLib could not find is null there.
fn messages(cache: &mut Cache, answer: &Value, out: &mut Vec<Event>) -> Vec<Message> {
    let found = items(&answer["messages"]).filter(|message| message.is_object());
    found.map(|message| cache.message(message, out)).collect()
}

/// A topic that is new or changed, as the event that tells it.
fn told(topic: Topic) -> Event {
    Event::Topics { chat: topic.chat, topics: vec![topic] }
}

/// What to say when a `failedToAddMembers` names someone.
fn not_added(failed: &Value) -> Option<Event> {
    items(&failed["failed_to_add_members"]).next().map(|_| Event::Problem(NOT_ADDED.to_owned()))
}

/// Telegram's error codes, and what TDLib says itself, as sentences.
pub fn friendly(message: &str) -> String {
    let known = [
        ("PHONE_NUMBER_INVALID", "That phone number is not valid. Write it with its country code, like +31612345678."),
        ("PHONE_NUMBER_BANNED", "That phone number is banned from Telegram."),
        ("PHONE_CODE_INVALID", "That code is not right."),
        ("PHONE_CODE_EXPIRED", "That code has expired. Go back and ask for a new one."),
        ("PHONE_CODE_EMPTY", "Type the code first."),
        ("PASSWORD_HASH_INVALID", "That password is not right."),
        ("EMAIL_INVALID", "That email address is not valid."),
        ("SEND_CODE_UNAVAILABLE", "Telegram has no other way to send the code right now."),
        ("CODE_INVALID", "That code is not right."),
        ("MESSAGE_NOT_MODIFIED", "Nothing was changed."),
        ("CHAT_WRITE_FORBIDDEN", "You cannot write in this chat."),
        ("CHAT_SEND_", "You cannot send that in this chat."),
        ("CHAT_RESTRICTED", "You cannot write in this chat."),
        ("USER_BANNED_IN_CHANNEL", "Telegram does not let you write in groups and channels for now."),
        ("SLOWMODE_WAIT", "This chat is in slow mode. Wait a little before writing again."),
        ("CHAT_ADMIN_REQUIRED", "Only an admin of this chat can do that."),
        ("Not enough rights", "You are not allowed to do that in this chat."),
        ("RIGHT_FORBIDDEN", "You cannot give someone rights you do not have yourself."),
        ("USER_ADMIN_INVALID", "You cannot change what this person is in the chat."),
        ("USER_CREATOR", "The owner of a chat cannot be changed."),
        ("ADMINS_TOO_MUCH", "This chat has as many admins as it can have."),
        ("USER_PRIVACY_RESTRICTED", "Their privacy settings do not let you add them. Send them an invitation instead."),
        ("USER_NOT_MUTUAL_CONTACT", "They can only be added by someone in their contacts."),
        ("USER_ALREADY_PARTICIPANT", "Already in this chat."),
        ("USER_NOT_PARTICIPANT", "They are not in this chat."),
        ("USER_KICKED", "They were removed from this chat and cannot be added again."),
        ("USER_CHANNELS_TOO_MUCH", "They are in too many groups and channels already."),
        ("CHANNELS_TOO_MUCH", "You are in too many groups and channels. Leave one first."),
        ("USERS_TOO_MUCH", "This chat is full."),
        ("YOU_BLOCKED_USER", "You blocked them. Unblock them in Telegram first."),
        ("USER_IS_BLOCKED", "You cannot write to them."),
        ("PEER_FLOOD", "Telegram is holding you back for now. Try again later."),
        ("REACTION_INVALID", "That reaction is not allowed here."),
        ("REACTIONS_TOO_MANY", "This message has as many different reactions as it can have."),
        ("MESSAGE_EDIT_TIME_EXPIRED", "This message is too old to be changed."),
        ("MESSAGE_AUTHOR_REQUIRED", "Only whoever wrote this message can change it."),
        ("Message can't be edited", "This message cannot be changed."),
        ("can't be deleted for everyone", "This message can only be deleted for yourself."),
        ("MESSAGE_DELETE_FORBIDDEN", "You cannot delete this message."),
        ("Message can't be deleted", "You cannot delete this message."),
        ("MESSAGE_ID_INVALID", "That message is gone."),
        ("Message not found", "That message is gone."),
        ("can't be pinned", "That cannot be pinned."),
        ("PINNED_DIALOGS_TOO_MUCH", "No more chats can be kept at the top. Unpin one first."),
        ("maximum number of pinned chats", "No more chats can be kept at the top. Unpin one first."),
        ("CHAT_FORWARDS_RESTRICTED", "Messages from this chat cannot be passed on."),
        ("can't be forwarded", "This message cannot be passed on."),
        ("USERNAME_NOT_OCCUPIED", "Nobody goes by that name."),
        ("USERNAME_INVALID", "Nobody goes by that name."),
        ("INVITE_HASH_EXPIRED", "That invitation is no longer valid."),
        ("INVITE_HASH_INVALID", "That invitation is not valid."),
        ("INVITE_REQUEST_SENT", "Your request to join was sent."),
        ("BOT_RESPONSE_TIMEOUT", "The bot did not answer."),
        ("MESSAGE_POLL_CLOSED", "This poll is closed."),
        ("REVOTE_NOT_ALLOWED", "The answer to this poll cannot be changed."),
        ("CHAT_TITLE_EMPTY", "A chat needs a name."),
        ("CHAT_ABOUT_TOO_LONG", "That is too long for a description."),
        ("PREMIUM_ACCOUNT_REQUIRED", "That needs Telegram Premium."),
    ];
    if let Some((_, sentence)) = known.iter().find(|(code, _)| message.contains(code)) {
        return (*sentence).to_owned();
    }
    // "Too Many Requests: retry after 37", "FLOOD_WAIT_37".
    let flood = message.contains("Too Many Requests") || message.contains("FLOOD_WAIT");
    let seconds = message.rsplit(|c: char| !c.is_ascii_digit()).next().and_then(|digits| digits.parse::<u64>().ok());
    match (flood, seconds) {
        (true, Some(seconds)) if seconds >= 120 => format!("Too many attempts. Try again in {} minutes.", seconds / 60),
        (true, Some(seconds)) => format!("Too many attempts. Try again in {seconds} seconds."),
        (true, None) => "Too many attempts. Try again later.".to_owned(),
        _ => message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use adelite_chat::{Body, Info, List};

    use super::*;

    /// What came of one thing TDLib said: the events, and the kinds of the requests that follow.
    fn said(cache: &mut Cache, said: &Value) -> (Vec<Event>, Vec<Value>) {
        let (mut out, mut ask) = (Vec::new(), Vec::new());
        take(cache, said, 10_000, &mut out, &mut ask);
        (out, ask)
    }

    /// A cache that knows Mara (7), the chat with her and a group (-60) the user is in.
    fn cache() -> Cache {
        let mut cache = Cache::default();
        cache.me = 1;
        for update in [
            json!({"@type": "updateUser", "user": {"id": 7, "first_name": "Mara", "last_name": "Lind",
                "status": {"@type": "userStatusOnline"}}}),
            json!({"@type": "updateNewChat", "chat": {"id": 7, "type": {"@type": "chatTypePrivate", "user_id": 7}}}),
            json!({"@type": "updateBasicGroup", "basic_group": {"id": 60, "member_count": 3,
                "status": {"@type": "chatMemberStatusCreator"}}}),
            json!({"@type": "updateNewChat", "chat": {"id": -60, "title": "Design",
                "type": {"@type": "chatTypeBasicGroup", "basic_group_id": 60}}}),
        ] {
            said(&mut cache, &update);
        }
        cache
    }

    fn message(id: i64, chat: i64) -> Value {
        json!({"@type": "message", "id": id, "chat_id": chat, "date": 1000 + id,
            "sender_id": {"@type": "messageSenderUser", "user_id": 7},
            "content": {"@type": "messageText", "text": {"text": "hello"}}})
    }

    fn kinds(requests: &[Value]) -> Vec<&str> {
        requests.iter().map(|request| request["@type"].as_str().unwrap_or_default()).collect()
    }

    #[test]
    fn what_a_chat_says_about_itself_follows_telegram() {
        let mut cache = cache();
        // The group's description was changed, here or on another device.
        let group = json!({"@type": "updateBasicGroupFullInfo", "basic_group_id": 60,
            "basic_group_full_info": {"description": "Specs and reviews", "invite_link": {"invite_link": "https://t.me/+abc"}}});
        let (out, _) = said(&mut cache, &group);
        assert!(matches!(out.as_slice(),
            [Event::Info(info)] if info.chat == -60 && info.about == "Specs and reviews" && info.link == "https://t.me/+abc"));
        let person = json!({"@type": "updateUserFullInfo", "user_id": 7, "user_full_info": {"bio": {"text": "Roofs"}}});
        assert!(
            matches!(said(&mut cache, &person).0.as_slice(), [Event::Info(info)] if info.chat == 7 && info.about == "Roofs")
        );
        // A chat that was never told has no card to keep current.
        let unknown = json!({"@type": "updateSupergroupFullInfo", "supergroup_id": 99, "supergroup_full_info": {"description": "x"}});
        assert!(said(&mut cache, &unknown).0.is_empty());
    }

    #[test]
    fn updates_tell_what_changed() {
        let mut cache = Cache::default();
        let (out, ask) = said(
            &mut cache,
            &json!({"@type": "updateNewChat", "chat": {"id": 7,
            "type": {"@type": "chatTypePrivate", "user_id": 7}}}),
        );
        assert!(matches!(out.as_slice(), [Event::Chat(chat)] if chat.id == 7) && ask.is_empty());
        // The lists are told once something is known of them, and again only when they change.
        let folders = json!({"@type": "updateChatFolders", "main_chat_list_position": 0,
            "chat_folders": [{"id": 4, "name": {"text": {"text": "Work"}}}]});
        let (out, _) = said(&mut cache, &folders);
        assert!(
            matches!(out.as_slice(), [Event::Lists(lists)] if lists[0].list == List::Main && lists[1].name == "Work")
        );
        assert!(said(&mut cache, &folders).0.is_empty());
        let (out, ask) = said(&mut cache, &json!({"@type": "updateNewMessage", "message": message(12, 7)}));
        assert!(matches!(out.as_slice(), [Event::Message(message)] if message.id == 12 && message.chat == 7));
        assert!(ask.is_empty());
        // A pin changes which message the chat shows at its top: that is asked for.
        let pinned = json!({"@type": "updateMessageIsPinned", "chat_id": 7, "message_id": 12, "is_pinned": true});
        let (out, ask) = said(&mut cache, &pinned);
        assert_eq!(out, [Event::Pinned { chat: 7, message: 12, on: true }]);
        assert_eq!((kinds(&ask), &ask[0]["@extra"]), (vec!["getChatPinnedMessage"], &json!({"for": "top", "chat": 7})));
        let counts = json!({"@type": "updateMessageInteractionInfo", "chat_id": 7, "message_id": 12,
            "interaction_info": {"view_count": 3}});
        assert!(matches!(said(&mut cache, &counts).0.as_slice(), [Event::Counts { views: 3, comments: None, .. }]));
        // A poll that changes is told for the message that shows it.
        let poll = json!({"id": "77", "question": {"text": "Lunch?"}, "total_voter_count": 2});
        let content = json!({"@type": "updateMessageContent", "chat_id": 7, "message_id": 12,
            "new_content": {"@type": "messagePoll", "poll": poll}});
        assert!(matches!(said(&mut cache, &content).0.as_slice(), [Event::Body { body: Body::Poll(_), .. }]));
        let (out, _) = said(&mut cache, &json!({"@type": "updatePoll", "poll": poll}));
        assert!(
            matches!(out.as_slice(), [Event::Body { chat: 7, message: 12, body: Body::Poll(poll) }] if poll.voters == 2)
        );
        // A community that is new is asked for its chats; what is not understood is passed over.
        let community = json!({"@type": "updateCommunity", "community": {"id": 3, "name": "Studio", "have_access": true,
            "status": {"@type": "communityMemberStatusMember"}}});
        assert_eq!(kinds(&said(&mut cache, &community).1), ["getCommunityFullInfo"]);
        for odd in
            [json!({"@type": "updateSomethingNew", "chat_id": 7}), json!({"@type": "ok"}), json!(null), json!([1])]
        {
            assert_eq!(said(&mut cache, &odd), (Vec::new(), Vec::new()), "{odd}");
        }
    }

    #[test]
    fn a_page_of_history_answers_for_the_chat_topic_and_side_asked() {
        let mut cache = cache();
        let extra = json!({"for": "history", "chat": 7, "topic": -4096, "page": "newer", "from": 12});
        let answer = json!({"@type": "messages", "@extra": extra,
            "messages": [message(14, 7), message(13, 7), message(12, 7), null, message(11, 7)]});
        let (out, _) = said(&mut cache, &answer);
        let [Event::History { chat: 7, topic: -4096, page: Page::Newer, messages }] = out.as_slice() else {
            panic!("a page, not {out:?}")
        };
        assert_eq!(messages.iter().map(|message| message.id).collect::<Vec<_>>(), [14, 13], "only what came after");
        // A page that fails is an empty one, so nobody waits for it.
        let (out, _) =
            said(&mut cache, &json!({"@type": "error", "code": 400, "message": "Chat not found", "@extra": extra}));
        assert_eq!(out, [Event::History { chat: 7, topic: -4096, page: Page::Newer, messages: Vec::new() }]);
        let older = json!({"@type": "messages", "messages": [message(12, 7)],
            "@extra": {"for": "history", "chat": 7, "topic": 0, "page": "older", "from": 12}});
        assert!(
            matches!(said(&mut cache, &older).0.as_slice(), [Event::History { page: Page::Older, messages, .. }] if messages.len() == 1)
        );
        // The message at the top, or none.
        let mut top = message(12, 7);
        top["@extra"] = json!({"for": "top", "chat": 7});
        assert!(
            matches!(said(&mut cache, &top).0.as_slice(), [Event::Top { chat: 7, message: Some(message) }] if message.id == 12)
        );
        let none = json!({"@type": "error", "code": 404, "message": "Not Found", "@extra": {"for": "top", "chat": 7}});
        assert_eq!(said(&mut cache, &none).0, [Event::Top { chat: 7, message: None }]);
        let fetched =
            json!({"@type": "messages", "messages": [null, message(9, 7)], "@extra": {"for": "fetched", "chat": 7}});
        assert!(
            matches!(said(&mut cache, &fetched).0.as_slice(), [Event::Fetched { chat: 7, messages }] if messages.len() == 1)
        );
        // A page of a chat's files: without the message it was asked from, which TDLib starts it with.
        let extra = json!({"for": "shared", "chat": 7, "kind": "files", "before": 12});
        let page = json!({"@type": "foundChatMessages", "messages": [message(12, 7), message(9, 7)], "@extra": extra});
        let told = said(&mut cache, &page).0;
        assert!(matches!(told.as_slice(),
            [Event::Shared { chat: 7, kind: adelite_chat::Shared::Files, before: 12, messages }] if messages.len() == 1 && messages[0].id == 9));
        let failed = json!({"@type": "error", "code": 400, "message": "CHAT_ADMIN_REQUIRED", "@extra": extra});
        assert!(
            matches!(said(&mut cache, &failed).0.as_slice(), [Event::Shared { messages, .. }] if messages.is_empty())
        );
    }

    #[test]
    fn topics_are_asked_for_page_after_page() {
        let mut cache = cache();
        let topic = |id: i64| json!({"info": {"chat_id": -60, "forum_topic_id": id, "name": "Roof"}, "order": "5"});
        let page = |after: Value, next: i64| {
            json!({"@type": "forumTopics", "topics": [topic(next), topic(next + 1)],
            "next_offset_date": next, "next_offset_message_id": 0, "next_offset_forum_topic_id": next,
            "@extra": {"for": "topics", "chat": -60, "seen": 0, "after": after}})
        };
        let (out, ask) = said(&mut cache, &page(json!([0, 0, 0]), 5));
        assert!(matches!(out.as_slice(), [Event::Topics { chat: -60, topics }] if topics.len() == 2));
        assert_eq!(
            (kinds(&ask), ask[0]["offset_date"].as_i64(), ask[0]["@extra"]["seen"].as_i64()),
            (vec!["getForumTopics"], Some(5), Some(2))
        );
        // The same place to go on from again, or enough of them seen: no more is asked.
        assert!(said(&mut cache, &page(json!([5, 0, 5]), 5)).1.is_empty());
        let mut full = page(json!([0, 0, 0]), 5);
        full["@extra"]["seen"] = json!(request::TOPICS_MOST);
        assert!(said(&mut cache, &full).1.is_empty());
        let empty = json!({"@type": "forumTopics", "topics": [], "next_offset_date": 9, "@extra": {"for": "topics", "chat": -60}});
        assert_eq!(said(&mut cache, &empty), (vec![Event::Topics { chat: -60, topics: Vec::new() }], Vec::new()));
        // A topic that is news in a forum the view follows is fetched whole.
        let news =
            json!({"@type": "updateForumTopicInfo", "info": {"chat_id": -60, "forum_topic_id": 9, "name": "New"}});
        assert_eq!(kinds(&said(&mut cache, &news).1), ["getForumTopic"]);
        let elsewhere = json!({"@type": "updateForumTopicInfo", "info": {"chat_id": 7, "forum_topic_id": 9}});
        assert_eq!(said(&mut cache, &elsewhere), (Vec::new(), Vec::new()));
        let mut whole = topic(9);
        whole["@extra"] = json!({"for": "topic", "chat": -60});
        assert!(matches!(said(&mut cache, &whole).0.as_slice(), [Event::Topics { topics, .. }] if topics[0].id == 9));
    }

    #[test]
    fn a_search_answers_in_parts() {
        let mut cache = cache();
        let extra = json!({"for": "found", "token": 3});
        let chats = json!({"@type": "chats", "chat_ids": [7, 999, -60], "@extra": extra});
        assert_eq!(said(&mut cache, &chats).0, [Event::Found { token: 3, chats: vec![7, -60], messages: Vec::new() }]);
        // People are found as users, and told as the chats with them.
        let (out, ask) = said(&mut cache, &json!({"@type": "users", "user_ids": [8, 9], "@extra": extra}));
        assert!(out.is_empty());
        assert_eq!(
            (kinds(&ask), &ask[1]["user_id"], &ask[1]["@extra"]),
            (vec!["createPrivateChat"; 2], &json!(9), &extra)
        );
        let chat =
            json!({"@type": "chat", "id": 8, "type": {"@type": "chatTypePrivate", "user_id": 8}, "@extra": extra});
        let (out, _) = said(&mut cache, &chat);
        assert!(
            matches!(out.as_slice(), [Event::Chat(chat), Event::Found { token: 3, chats, .. }] if chat.id == 8 && chats == &[8]),
            "a chat is told before it is pointed at: {out:?}"
        );
        let messages = json!({"@type": "foundMessages", "messages": [message(12, 7)], "@extra": extra});
        assert!(
            matches!(said(&mut cache, &messages).0.as_slice(), [Event::Found { chats, messages, .. }] if chats.is_empty() && messages.len() == 1)
        );
        let failed = json!({"@type": "error", "code": 400, "message": "SEARCH_QUERY_EMPTY", "@extra": extra});
        assert_eq!(
            said(&mut cache, &failed),
            (Vec::new(), Vec::new()),
            "a part that fails is no problem of the user's"
        );
    }

    #[test]
    fn a_link_is_followed_as_far_as_it_leads() {
        let mut cache = cache();
        let read = |kind: &str| {
            json!({"@type": kind, "url": "https://t.me/c/1/2", "chat_username": "adelite",
            "invite_link": "https://t.me/+abc", "@extra": {"for": "link", "url": "https://t.me/x"}})
        };
        let asked = |cache: &mut Cache, kind: &str| said(cache, &read(kind)).1.remove(0);
        let message_link = asked(&mut cache, "internalLinkTypeMessage");
        assert_eq!(
            (message_link["@type"].as_str(), message_link["url"].as_str()),
            (Some("getMessageLinkInfo"), Some("https://t.me/c/1/2"))
        );
        assert_eq!(asked(&mut cache, "internalLinkTypePublicChat")["username"], "adelite");
        assert_eq!(asked(&mut cache, "internalLinkTypeChatInvite")["invite_link"], "https://t.me/+abc");
        // What this app does nothing with, and what TDLib does not know as Telegram's, opens outside.
        let outside = [Event::Browse("https://t.me/x".into())];
        assert_eq!(said(&mut cache, &read("internalLinkTypeStickerSet")).0, outside);
        let unknown = json!({"@type": "error", "code": 404, "message": "Not Found", "@extra": {"for": "link", "url": "https://t.me/x"}});
        assert_eq!(said(&mut cache, &unknown).0, outside);
        // A message's link leads to it in its topic; into a chat that cannot be seen, outside.
        let info = |chat: i64| {
            json!({"@type": "messageLinkInfo", "chat_id": chat, "message": {"id": 12,
            "topic_id": {"@type": "messageTopicForum", "forum_topic_id": 5}}, "@extra": {"for": "message-link", "url": "https://t.me/x"}})
        };
        assert_eq!(said(&mut cache, &info(-60)).0, [Event::Goto { chat: -60, topic: 5, message: 12 }]);
        assert_eq!(said(&mut cache, &info(0)).0, outside);
        // An invitation is looked at, never taken.
        let invite = |chat: i64| json!({"@type": "chatInviteLinkInfo", "chat_id": chat, "title": "Roofers", "@extra": {"for": "invite-link"}});
        assert_eq!(said(&mut cache, &invite(-60)), (vec![Event::Goto { chat: -60, topic: 0, message: 0 }], Vec::new()));
        let (out, ask) = said(&mut cache, &invite(0));
        assert!(matches!(out.as_slice(), [Event::Notice(notice)] if notice.contains("Roofers")) && ask.is_empty());
        // Whoever goes by a name, or the chat with someone: told, then gone to.
        let chat = json!({"@type": "chat", "id": 9, "type": {"@type": "chatTypePrivate", "user_id": 9}, "@extra": {"for": "goto"}});
        let (out, _) = said(&mut cache, &chat);
        assert!(matches!(out.as_slice(), [Event::Chat(_), Event::Goto { chat: 9, topic: 0, message: 0 }]));
        let nobody =
            json!({"@type": "error", "code": 400, "message": "USERNAME_NOT_OCCUPIED", "@extra": {"for": "goto"}});
        assert_eq!(said(&mut cache, &nobody).0, [Event::Problem("Nobody goes by that name.".into())]);
        let comments = json!({"@type": "messageThreadInfo", "chat_id": -60, "message_thread_id": 4096, "@extra": {"for": "comments"}});
        assert_eq!(said(&mut cache, &comments).0, [Event::Goto { chat: -60, topic: -4096, message: 0 }]);
    }

    #[test]
    fn the_oldest_unread_mention_is_gone_to() {
        let mut cache = cache();
        let found = |messages: Value, oldest: bool| {
            json!({"@type": "foundChatMessages", "messages": messages,
            "@extra": {"for": "mention", "chat": 7, "oldest": oldest}})
        };
        let (out, _) = said(&mut cache, &found(json!([message(30, 7), message(20, 7), null]), true));
        assert_eq!(out, [Event::Goto { chat: 7, topic: 0, message: 20 }]);
        // Asked from the oldest and nothing came, or TDLib would not answer that: asked from the newest.
        let (out, ask) = said(&mut cache, &found(json!([]), true));
        assert_eq!(
            (out.len(), ask[0]["from_message_id"].as_i64(), ask[0]["@extra"]["oldest"].as_bool()),
            (0, Some(0), Some(false))
        );
        let refused =
            json!({"@type": "error", "message": "Invalid", "@extra": {"for": "mention", "chat": 7, "oldest": true}});
        assert_eq!(kinds(&said(&mut cache, &refused).1), ["searchChatMessages"]);
        assert_eq!(said(&mut cache, &found(json!([]), false)), (Vec::new(), Vec::new()));
        let again =
            json!({"@type": "error", "message": "Invalid", "@extra": {"for": "mention", "chat": 7, "oldest": false}});
        assert_eq!(said(&mut cache, &again), (Vec::new(), Vec::new()));
    }

    #[test]
    fn an_invitation_is_made_only_where_there_is_none() {
        let mut cache = cache();
        let full = |link: Value, purpose: &str| {
            json!({"@type": "basicGroupFullInfo", "description": "Plans",
            "invite_link": link, "@extra": {"for": purpose, "chat": -60, "link": ""}})
        };
        let info = |link: &str| Info {
            chat: -60,
            about: "Plans".into(),
            link: link.into(),
            members: 3,
            can_invite: true,
            can_edit: true,
            ..Info::default()
        };
        assert_eq!(said(&mut cache, &full(json!(null), "info")), (vec![Event::Info(info(""))], Vec::new()));
        let existing = json!({"invite_link": "https://t.me/+old"});
        assert_eq!(
            said(&mut cache, &full(existing, "invite")),
            (vec![Event::Info(info("https://t.me/+old"))], Vec::new())
        );
        // None yet: one is made, and the chat is told again with it.
        let (out, ask) = said(&mut cache, &full(json!(null), "invite"));
        assert_eq!(
            (out.len(), kinds(&ask), &ask[0]["@extra"]),
            (0, vec!["replacePrimaryChatInviteLink"], &json!({"for": "invited", "chat": -60}))
        );
        let made = json!({"@type": "chatInviteLink", "invite_link": "https://t.me/+new", "@extra": ask[0]["@extra"]});
        let (_, ask) = said(&mut cache, &made);
        assert_eq!(
            (kinds(&ask), &ask[0]["@extra"]),
            (vec!["getBasicGroupFullInfo"], &json!({"for": "info", "chat": -60, "link": "https://t.me/+new"}))
        );
        let mut again = full(json!(null), "info");
        again["@extra"] = ask[0]["@extra"].clone();
        assert_eq!(said(&mut cache, &again).0, [Event::Info(info("https://t.me/+new"))]);
        // A person has no invitation to make.
        let person = json!({"@type": "userFullInfo", "@extra": {"for": "invite", "chat": 7}});
        assert_eq!(said(&mut cache, &person), (vec![Event::Info(Info { chat: 7, ..Info::default() })], Vec::new()));
        let denied = json!({"@type": "error", "message": "Not enough rights to manage chat invite link", "@extra": {"for": "invited", "chat": -60}});
        assert_eq!(
            said(&mut cache, &denied).0,
            [Event::Problem("You are not allowed to do that in this chat.".into())]
        );
    }

    #[test]
    fn people_buttons_and_joining_answer_in_words() {
        let mut cache = cache();
        let members = json!({"@type": "chatMembers", "@extra": {"for": "members", "chat": -60, "query": "ma"}, "members": [
            {"member_id": {"@type": "messageSenderUser", "user_id": 7}, "status": {"@type": "chatMemberStatusCreator"}}]});
        let (out, _) = said(&mut cache, &members);
        assert!(
            matches!(out.as_slice(), [Event::Members { chat: -60, query, members }] if query == "ma" && members[0].name == "Mara Lind")
        );
        let hidden = json!({"@type": "error", "message": "CHAT_ADMIN_REQUIRED", "@extra": {"for": "members", "chat": -60, "query": "ma"}});
        assert_eq!(
            said(&mut cache, &hidden).0,
            [Event::Members { chat: -60, query: "ma".into(), members: Vec::new() }]
        );
        let people = json!({"@type": "users", "user_ids": [7], "@extra": {"for": "people", "query": ""}});
        assert!(
            matches!(said(&mut cache, &people).0.as_slice(), [Event::People { query, people }] if query.is_empty() && people.len() == 1)
        );
        // What a button answers: words to show, an address to open, or both.
        let pressed = json!({"@type": "callbackQueryAnswer", "text": "Saved", "url": "https://adelite.example", "@extra": {"for": "press"}});
        assert_eq!(
            said(&mut cache, &pressed).0,
            [Event::Notice("Saved".into()), Event::Browse("https://adelite.example".into())]
        );
        let silent = json!({"@type": "callbackQueryAnswer", "text": "", "@extra": {"for": "press"}});
        assert!(said(&mut cache, &silent).0.is_empty());
        let late =
            json!({"@type": "error", "code": 502, "message": "BOT_RESPONSE_TIMEOUT", "@extra": {"for": "press"}});
        assert_eq!(said(&mut cache, &late).0, [Event::Problem("The bot did not answer.".into())]);
        let joined = |kind: &str| said(&mut Cache::default(), &json!({"@type": kind, "@extra": {"for": "joined"}})).0;
        assert!(joined("chatJoinResultSuccess").is_empty());
        assert!(matches!(joined("chatJoinResultRequestSent").as_slice(), [Event::Notice(_)]));
        assert!(matches!(joined("chatJoinResultDeclined").as_slice(), [Event::Problem(_)]));
        // Someone who could not be added is said; a basic group that was left goes from the list.
        let added = |failed: Value| json!({"@type": "failedToAddMembers", "failed_to_add_members": failed, "@extra": {"for": "added"}});
        assert!(said(&mut cache, &added(json!([]))).0.is_empty());
        assert_eq!(said(&mut cache, &added(json!([{"user_id": 8}]))).0, [Event::Problem(NOT_ADDED.into())]);
        let group = json!({"@type": "createdBasicGroupChat", "chat_id": -61, "failed_to_add_members": {"failed_to_add_members": [{"user_id": 8}]},
            "@extra": {"for": "goto"}});
        assert_eq!(
            said(&mut cache, &group).0,
            [Event::Goto { chat: -61, topic: 0, message: 0 }, Event::Problem(NOT_ADDED.into())]
        );
        let (out, ask) = said(&mut cache, &json!({"@type": "ok", "@extra": {"for": "left", "chat": -60}}));
        assert_eq!(
            (out.len(), kinds(&ask), ask[0]["remove_from_chat_list"].as_bool()),
            (0, vec!["deleteChatHistory"], Some(true))
        );
    }

    #[test]
    fn what_is_asked_in_the_background_fails_quietly() {
        let mut cache = cache();
        let failed =
            |extra: Value| json!({"@type": "error", "code": 400, "message": "CHAT_ADMIN_REQUIRED", "@extra": extra});
        for purpose in ["quiet", "found", "topic", "scope", "communities", "community"] {
            assert_eq!(said(&mut cache, &failed(json!({"for": purpose}))), (Vec::new(), Vec::new()), "{purpose}");
        }
        // What the user asked for by name says why it did not happen.
        let problem = [Event::Problem("Only an admin of this chat can do that.".into())];
        assert_eq!(
            said(&mut cache, &json!({"@type": "error", "code": 400, "message": "CHAT_ADMIN_REQUIRED"})).0,
            problem
        );
        for purpose in ["joined", "added", "left", "press", "info", "comments", "invite-link"] {
            assert_eq!(said(&mut cache, &failed(json!({"for": purpose}))).0, problem, "{purpose}");
        }
        assert_eq!(
            said(&mut cache, &failed(json!({"for": "topics", "chat": -60}))).0,
            [Event::Topics { chat: -60, topics: Vec::new() }]
        );
        assert_eq!(
            said(&mut cache, &failed(json!({"for": "people", "query": "x"}))).0,
            [Event::People { query: "x".into(), people: Vec::new() }]
        );
        // The settings for a kind of chat, and a community's chats, are taken in as they come.
        let scope = json!({"@type": "scopeNotificationSettings", "mute_for": 600,
            "@extra": {"for": "scope", "scope": {"@type": "notificationSettingsScopePrivateChats"}}});
        assert!(matches!(said(&mut cache, &scope).0.as_slice(), [Event::Chat(chat)] if chat.id == 7 && chat.muted));
        let joined = json!({"@type": "communities", "community_ids": [3, 4], "@extra": {"for": "communities"}});
        assert_eq!(kinds(&said(&mut cache, &joined).1), ["getCommunityFullInfo"; 2]);
        let unknown =
            json!({"@type": "communityFullInfo", "chats": [{"chat_id": -60}], "@extra": {"for": "community", "id": 3}});
        assert_eq!(said(&mut cache, &unknown), (Vec::new(), Vec::new()), "a community nothing was told of");
    }

    #[test]
    fn errors_read_as_sentences() {
        assert_eq!(friendly("PHONE_CODE_INVALID"), "That code is not right.");
        assert_eq!(friendly("Too Many Requests: retry after 37"), "Too many attempts. Try again in 37 seconds.");
        assert_eq!(friendly("FLOOD_WAIT_600"), "Too many attempts. Try again in 10 minutes.");
        assert_eq!(friendly("Too Many Requests"), "Too many attempts. Try again later.");
        assert_eq!(friendly("Chat not found"), "Chat not found", "what is not known is said as it came");
        // What the commands run into: Telegram's codes, and TDLib's own words.
        assert_eq!(friendly("CHAT_ADMIN_REQUIRED"), "Only an admin of this chat can do that.");
        let rights = "Not enough rights to manage pinned messages in the chat";
        assert_eq!(friendly(rights), "You are not allowed to do that in this chat.");
        assert_eq!(friendly("REACTION_INVALID"), "That reaction is not allowed here.");
        assert!(friendly("USER_PRIVACY_RESTRICTED").contains("invitation"));
        assert_eq!(friendly("USER_ALREADY_PARTICIPANT"), "Already in this chat.");
        assert_eq!(friendly("Message can't be edited"), "This message cannot be changed.");
        assert_eq!(friendly("Message can't be deleted for everyone"), "This message can only be deleted for yourself.");
        assert_eq!(friendly("Message can't be deleted"), "You cannot delete this message.");
        assert_eq!(friendly("The maximum number of pinned chats exceeded"), friendly("PINNED_DIALOGS_TOO_MUCH"));
        assert_eq!(friendly("PEER_FLOOD"), "Telegram is holding you back for now. Try again later.");
    }
}
