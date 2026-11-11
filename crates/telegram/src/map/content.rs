//! What a message is and holds: a `message` and its `MessageContent` as
//! the chat model has them, with the reactions, buttons and link preview
//! that come along, and `formattedText` as styled words.

use adelite_chat::{
    Body, Button, ChatId, Delivery, Event, FileId, Kind, LinkPreview, Message, MessageId, Motion, Poll, PollOption,
    Press, Preview, Reaction, Span, Style, Text, TopicId,
};
use serde_json::Value;

use super::{Cache, POLLS, count, file, flag, int, items, kind, string};

impl Cache {
    /// A chat's latest `message` as its row in the list shows it.
    pub(super) fn preview(&self, message: &Value, chat: Kind, out: &mut Vec<Event>) -> Option<Preview> {
        if !message.is_object() {
            return None;
        }
        let outgoing = flag(&message["is_outgoing"]);
        let (_, name, _) = self.sender(&message["sender_id"]);
        let body = self.body(&message["content"], &name, out);
        let sender = match (chat, &body, outgoing) {
            // What the chat says itself names who did it.
            (Kind::Group, Body::Service(_), _) | (Kind::Private | Kind::Channel, ..) => String::new(),
            (Kind::Group, _, true) => "You".to_owned(),
            (Kind::Group, _, false) => self.first_name(&message["sender_id"]),
        };
        Some(Preview { id: int(&message["id"]), sender, text: body.preview(), date: int(&message["date"]), outgoing })
    }

    /// A `message` as the chat model has it.
    pub fn message(&mut self, message: &Value, out: &mut Vec<Event>) -> Message {
        let (id, chat) = (int(&message["id"]), int(&message["chat_id"]));
        let (sender, sender_name, sender_photo) = self.sender(&message["sender_id"]);
        let reply = &message["reply_to"];
        let origin = &message["forward_info"]["origin"];
        let forwarded_from = match kind(origin) {
            "messageOriginUser" => self.user_name(int(&origin["sender_user_id"])).to_owned(),
            "messageOriginHiddenUser" => string(&origin["sender_name"]).to_owned(),
            "messageOriginChat" => self.chat_title(int(&origin["sender_chat_id"])),
            "messageOriginChannel" => self.chat_title(int(&origin["chat_id"])),
            _ => String::new(),
        };
        let content = &message["content"];
        let interaction = &message["interaction_info"];
        self.shows_poll(chat, id, content);
        Message {
            id,
            chat,
            topic: topic(&message["topic_id"]),
            sender,
            body: self.body(content, &sender_name, out),
            sender_name,
            sender_photo,
            outgoing: flag(&message["is_outgoing"]),
            date: int(&message["date"]),
            edited: int(&message["edit_date"]) > 0,
            delivery: match kind(&message["sending_state"]) {
                "messageSendingStatePending" => Delivery::Sending,
                "messageSendingStateFailed" => Delivery::Failed,
                _ => Delivery::Sent,
            },
            // An answer to a message of another chat has nothing here to point at.
            reply_to: (kind(reply) == "messageReplyToMessage" && int(&reply["chat_id"]) == chat)
                .then(|| int(&reply["message_id"])),
            forwarded_from,
            link: link(&content["link_preview"], out),
            reactions: reactions(interaction),
            buttons: buttons(&message["reply_markup"]),
            pinned: flag(&message["is_pinned"]),
            album: int(&message["media_album_id"]),
            views: count(&interaction["view_count"]),
            comments: comments(interaction, flag(&message["is_channel_post"])),
            mentions_me: flag(&message["contains_unread_mention"]),
        }
    }

    /// What people made of a message (`updateMessageInteractionInfo`).
    pub fn counts(&self, update: &Value) -> Event {
        let (chat, interaction) = (int(&update["chat_id"]), &update["interaction_info"]);
        Event::Counts {
            chat,
            message: int(&update["message_id"]),
            reactions: reactions(interaction),
            views: count(&interaction["view_count"]),
            comments: comments(interaction, self.is_channel(chat)),
        }
    }

    /// Remembers that a message shows a poll, when its content is one:
    /// TDLib tells a poll's changes by the poll alone.
    pub fn shows_poll(&mut self, chat: ChatId, message: MessageId, content: &Value) {
        let shown = (int(&content["poll"]["id"]), chat, message);
        if kind(content) != "messagePoll" || shown.0 == 0 || self.polls.contains(&shown) {
            return;
        }
        if self.polls.len() >= POLLS {
            self.polls.pop_front();
        }
        self.polls.push_back(shown);
    }

    /// A `poll` changed: the messages known to show it hold something new.
    pub fn polled(&self, changed: &Value, out: &mut Vec<Event>) {
        let id = int(&changed["id"]);
        for (_, chat, message) in self.polls.iter().filter(|(shown, ..)| *shown == id) {
            out.push(Event::Body { chat: *chat, message: *message, body: Body::Poll(poll(changed)) });
        }
    }

    /// A `MessageContent` as the chat model has it; `sender` did what a
    /// service message tells of.
    pub fn body(&self, content: &Value, sender: &str, out: &mut Vec<Event>) -> Body {
        let caption = || formatted(&content["caption"]);
        let other = |label: &str| Body::Other { label: label.to_owned(), caption: caption() };
        let service = |what: String| Body::Service(format!("{sender} {what}"));
        let names = |ids: &Value| {
            let names: Vec<&str> = items(ids).map(|id| self.user_name(int(id))).collect();
            names.join(", ")
        };
        match kind(content) {
            "messageText" => Body::Text(formatted(&content["text"])),
            "messageRichMessage" => {
                let mut words = String::new();
                items(&content["message"]["blocks"]).for_each(|block| rich(block, 0, &mut words));
                if words.is_empty() { other("Message") } else { Body::Text(Text::plain(words)) }
            }
            "messageAnimatedEmoji" | "messageDice" => Body::Text(Text::plain(string(&content["emoji"]))),
            "messagePhoto" => {
                let sizes = &content["photo"]["sizes"];
                let Some(size) = fitting(sizes) else { return other("Photo") };
                // Looked at alone, it is shown at its largest: its size is that one's.
                let whole = largest(sizes).unwrap_or(size);
                Body::Photo {
                    file: file(&size["photo"], out),
                    full: file(&whole["photo"], out),
                    width: pixels(&whole["width"]),
                    height: pixels(&whole["height"]),
                    caption: caption(),
                }
            }
            "messageVideo" => {
                // The cover its sender chose, else the frame Telegram made of it.
                let cover = fitting(&content["cover"]["sizes"]).and_then(|size| file(&size["photo"], out));
                moving(Motion::Video, &content["video"], "video", cover, caption(), out)
            }
            "messageAnimation" => moving(Motion::Gif, &content["animation"], "animation", None, caption(), out),
            "messageVideoNote" => moving(Motion::Round, &content["video_note"], "video", None, Text::default(), out),
            "messageSticker" => {
                let sticker = &content["sticker"];
                // A sticker that moves is shown by its still.
                let picture = match kind(&sticker["format"]) {
                    "stickerFormatWebp" => file(&sticker["sticker"], out),
                    _ => still(&sticker["thumbnail"], out),
                };
                Body::Sticker {
                    file: picture,
                    emoji: string(&sticker["emoji"]).to_owned(),
                    width: pixels(&sticker["width"]),
                    height: pixels(&sticker["height"]),
                }
            }
            "messageVoiceNote" => {
                let voice = &content["voice_note"];
                Body::Audio {
                    file: file(&voice["voice"], out),
                    voice: true,
                    title: String::new(),
                    seconds: count(&voice["duration"]),
                    caption: caption(),
                }
            }
            "messageAudio" => {
                let audio = &content["audio"];
                let title = match (string(&audio["performer"]), string(&audio["title"])) {
                    ("", "") => string(&audio["file_name"]).to_owned(),
                    (one, "") | ("", one) => one.to_owned(),
                    (performer, title) => format!("{performer} – {title}"),
                };
                Body::Audio {
                    file: file(&audio["audio"], out),
                    voice: false,
                    title,
                    seconds: count(&audio["duration"]),
                    caption: caption(),
                }
            }
            "messagePoll" => Body::Poll(poll(&content["poll"])),
            "messageDocument" => {
                let document = &content["document"];
                let size = int(&document["document"]["size"]).max(int(&document["document"]["expected_size"]));
                Body::File {
                    file: file(&document["document"], out),
                    name: string(&document["file_name"]).to_owned(),
                    size: size.max(0) as u64,
                    caption: caption(),
                }
            }
            "messageCall" => other(if flag(&content["is_video"]) { "Video call" } else { "Call" }),
            "messageExpiredPhoto" => other("Photo (expired)"),
            "messageExpiredVideo" => other("Video (expired)"),
            "messageExpiredVideoNote" => other("Video message (expired)"),
            "messageExpiredVoiceNote" => other("Voice message (expired)"),
            "messageUnsupported" => other("A message this app cannot show"),
            "messageChatAddMembers" => {
                let added = names(&content["member_user_ids"]);
                service(if added == sender { "joined".to_owned() } else { format!("added {added}") })
            }
            "messageChatJoinByLink" | "messageChatJoinByRequest" => service("joined".to_owned()),
            "messageChatJoinFromCommunity" => {
                service(format!("joined from {}", self.community_name(int(&content["community_id"]))))
            }
            "messageChatDeleteMember" => {
                let gone = self.user_name(int(&content["user_id"]));
                service(if gone == sender { "left".to_owned() } else { format!("removed {gone}") })
            }
            "messageChatAddedToCommunity" => {
                service(format!("added the chat to {}", self.community_name(int(&content["community_id"]))))
            }
            "messageChatRemovedFromCommunity" => service("took the chat out of its community".to_owned()),
            "messageChatChangeTitle" => service(format!("renamed the chat to “{}”", string(&content["title"]))),
            "messageChatChangePhoto" => service("changed the chat's picture".to_owned()),
            "messageChatDeletePhoto" => service("removed the chat's picture".to_owned()),
            "messageBasicGroupChatCreate" | "messageSupergroupChatCreate" => {
                service(format!("created “{}”", string(&content["title"])))
            }
            "messageChatUpgradeTo" | "messageChatUpgradeFrom" => {
                Body::Service("The group was made a supergroup".to_owned())
            }
            "messageChatSetMessageAutoDeleteTime" => service(match int(&content["message_auto_delete_time"]) {
                ..=0 => "turned off deleting messages after a while".to_owned(),
                seconds => format!("set messages to be deleted after {}", span(seconds)),
            }),
            "messageForumTopicCreated" => service(format!("created the topic “{}”", string(&content["name"]))),
            "messageForumTopicEdited" => service(match string(&content["name"]) {
                "" => "changed the topic's icon".to_owned(),
                name => format!("renamed the topic to “{name}”"),
            }),
            "messageForumTopicIsClosedToggled" => {
                service(if flag(&content["is_closed"]) { "closed the topic" } else { "reopened the topic" }.to_owned())
            }
            "messageForumTopicIsHiddenToggled" => {
                service(if flag(&content["is_hidden"]) { "hid the topic" } else { "showed the topic again" }.to_owned())
            }
            // Said of the chat, whoever the message is from.
            "messageVideoChatScheduled" => Body::Service("Video chat scheduled".to_owned()),
            "messageVideoChatStarted" => Body::Service("Video chat started".to_owned()),
            "messageVideoChatEnded" => Body::Service(match int(&content["duration"]) {
                ..=0 => "Video chat ended".to_owned(),
                seconds => format!("Video chat ended after {}", span(seconds)),
            }),
            "messagePinMessage" => service("pinned a message".to_owned()),
            "messageScreenshotTaken" => service("took a screenshot".to_owned()),
            "messageContactRegistered" => service("joined Telegram".to_owned()),
            "messageCustomServiceAction" => Body::Service(string(&content["text"]).to_owned()),
            // Whatever else Telegram has: named after its type.
            name => other(&humanize(name.strip_prefix("message").unwrap_or(name))),
        }
    }
}

/// A `MessageTopic` as the model's topic: a forum's topic by its id, a
/// thread (the comments under a post) by its id negated, anything else zero.
pub fn topic(topic: &Value) -> TopicId {
    match kind(topic) {
        "messageTopicForum" => int(&topic["forum_topic_id"]).max(0),
        "messageTopicThread" => -int(&topic["message_thread_id"]).max(0),
        _ => 0,
    }
}

/// The emoji a message was answered with (a `messageInteractionInfo`),
/// most used first. Custom and paid reactions are not shown.
fn reactions(interaction: &Value) -> Vec<Reaction> {
    let reaction = |reaction: &Value| {
        let emoji = string(&reaction["type"]["emoji"]);
        (kind(&reaction["type"]) == "reactionTypeEmoji" && !emoji.is_empty()).then(|| Reaction {
            emoji: emoji.to_owned(),
            count: count(&reaction["total_count"]),
            chosen: flag(&reaction["is_chosen"]),
        })
    };
    let mut reactions: Vec<Reaction> = items(&interaction["reactions"]["reactions"]).filter_map(reaction).collect();
    reactions.sort_by_key(|reaction| std::cmp::Reverse(reaction.count));
    reactions
}

/// How many comments a message has, where it can be commented on: a
/// channel's post that says so.
fn comments(interaction: &Value, post: bool) -> Option<u32> {
    let replies = &interaction["reply_info"];
    (post && replies.is_object()).then(|| count(&replies["reply_count"]))
}

/// The buttons under a message (a `ReplyMarkup`), in rows.
fn buttons(markup: &Value) -> Vec<Vec<Button>> {
    if kind(markup) != "replyMarkupInlineKeyboard" {
        return Vec::new();
    }
    let button = |button: &Value| {
        let about = &button["type"];
        let press = match kind(about) {
            // Signing in through the button is not done here: its page opens as any other.
            "inlineKeyboardButtonTypeUrl" | "inlineKeyboardButtonTypeLoginUrl" => {
                Press::Link(string(&about["url"]).to_owned())
            }
            "inlineKeyboardButtonTypeCallback" => Press::Data(string(&about["data"]).to_owned()),
            _ => Press::Nothing,
        };
        Button { label: string(&button["text"]).to_owned(), press }
    };
    let rows = items(&markup["rows"]).map(|row| items(row).map(button).collect::<Vec<_>>());
    rows.filter(|row| !row.is_empty()).collect()
}

/// The page a message's link leads to (a `linkPreview`), with its picture where it has one.
fn link(preview: &Value, out: &mut Vec<Event>) -> Option<LinkPreview> {
    let url = string(&preview["url"]);
    if url.is_empty() {
        return None;
    }
    let about = &preview["type"];
    let mut link = LinkPreview {
        url: url.to_owned(),
        site: string(&preview["site_name"]).to_owned(),
        title: string(&preview["title"]).to_owned(),
        description: string(&preview["description"]["text"]).to_owned(),
        ..LinkPreview::default()
    };
    // A photo, a video's cover, an embedded player's picture; else the still of what moves.
    let sized = |picture: Option<FileId>, of: &Value| (picture, pixels(&of["width"]), pixels(&of["height"]));
    let photo = ["photo", "cover", "thumbnail"].iter().find_map(|field| fitting(&about[*field]["sizes"]));
    let frame = |field: &&str| {
        let thumbnail = &about[*field]["thumbnail"];
        still(thumbnail, out).map(|picture| sized(Some(picture), thumbnail))
    };
    if let Some(size) = photo {
        (link.photo, link.width, link.height) = sized(file(&size["photo"], out), size);
    } else if let Some(frame) = ["video", "animation"].iter().find_map(frame) {
        (link.photo, link.width, link.height) = frame;
    }
    Some(link)
}

/// A `poll` as the model has it. Its answers stay in the order Telegram
/// counts them in, so the place of one is the number it is voted for by.
fn poll(poll: &Value) -> Poll {
    let option = |option: &Value| PollOption {
        text: string(&option["text"]["text"]).to_owned(),
        votes: count(&option["voter_count"]),
        percent: int(&option["vote_percentage"]).clamp(0, 100) as u8,
        // An answer on its way to Telegram shows as given already.
        chosen: flag(&option["is_chosen"]) || flag(&option["is_being_chosen"]),
    };
    Poll {
        question: string(&poll["question"]["text"]).to_owned(),
        options: items(&poll["options"]).map(option).collect(),
        voters: count(&poll["total_voter_count"]),
        closed: flag(&poll["is_closed"]),
        multiple: flag(&poll["allows_multiple_answers"]),
        quiz: kind(&poll["type"]) == "pollTypeQuiz",
    }
}

/// Something that moves (a `video`, an `animation`, a `videoNote`), whose
/// file is the field `media`. `cover` goes before its own still.
fn moving(
    motion: Motion,
    video: &Value,
    media: &str,
    cover: Option<FileId>,
    caption: Text,
    out: &mut Vec<Event>,
) -> Body {
    // A round video says one length for both sides.
    let side = |side: &str| if video[side].is_null() { &video["length"] } else { &video[side] };
    Body::Video {
        motion,
        still: cover.or_else(|| still(&video["thumbnail"], out)),
        file: file(&video[media], out),
        width: pixels(side("width")),
        height: pixels(side("height")),
        seconds: count(&video["duration"]),
        caption,
    }
}

/// A `thumbnail`'s file when it is a picture the app can decode: the
/// first frame of a video may itself be a video.
fn still(thumbnail: &Value, out: &mut Vec<Event>) -> Option<FileId> {
    let decodable = ["thumbnailFormatJpeg", "thumbnailFormatPng", "thumbnailFormatWebp"];
    decodable.contains(&kind(&thumbnail["format"])).then(|| file(&thumbnail["file"], out)).flatten()
}

/// Of a photo's `sizes`, the smallest that fills a bubble on a dense display; the last when none does.
fn fitting(sizes: &Value) -> Option<&Value> {
    let sizes = sizes.as_array()?;
    let fills = |size: &&Value| int(&size["width"]).max(int(&size["height"])) >= 800;
    sizes.iter().find(fills).or(sizes.last())
}

/// Of a photo's `sizes`, the one with the most pixels.
fn largest(sizes: &Value) -> Option<&Value> {
    items(sizes).max_by_key(|size| int(&size["width"]).saturating_mul(int(&size["height"])))
}

/// A width or height in pixels: at least one, so nothing divides by zero.
fn pixels(value: &Value) -> u32 {
    count(value).max(1)
}

/// A time span in the largest unit that fits: "1 day", "3 hours".
fn span(seconds: i64) -> String {
    let units = [(2_592_000, "month"), (604_800, "week"), (86_400, "day"), (3600, "hour"), (60, "minute")];
    let (many, unit) = units
        .iter()
        .find(|(length, _)| seconds >= *length)
        .map_or((seconds, "second"), |(length, unit)| (seconds / length, *unit));
    format!("{many} {unit}{}", if many == 1 { "" } else { "s" })
}

/// Gathers the words of a block of a rich message (a `PageBlock`), a line
/// for each part that has some. Its pictures, tables and the like are left out.
fn rich(block: &Value, depth: usize, words: &mut String) {
    if depth > 8 {
        return;
    }
    for field in ["title", "subtitle", "author", "header", "subheader", "kicker", "text", "footer", "credit"] {
        let mut line = String::new();
        rich_text(&block[field], 0, &mut line);
        if !line.trim().is_empty() {
            if !words.is_empty() {
                words.push('\n');
            }
            words.push_str(line.trim_end());
        }
    }
    rich(&block["cover"], depth + 1, words);
    rich(&block["caption"], depth + 1, words);
    for inner in items(&block["blocks"]).chain(items(&block["items"])) {
        rich(inner, depth + 1, words);
    }
}

/// The plain words of a `RichText`, which nests: styled text holds text.
fn rich_text(text: &Value, depth: usize, words: &mut String) {
    if depth > 16 {
        return;
    }
    match &text["text"] {
        Value::String(plain) => words.push_str(plain),
        inner @ Value::Object(_) => rich_text(inner, depth + 1, words),
        _ => words.push_str(string(&text["alternative_text"])),
    }
    items(&text["texts"]).for_each(|part| rich_text(part, depth + 1, words));
}

/// `VideoChatStarted` as "Video chat started".
fn humanize(name: &str) -> String {
    let mut words = String::with_capacity(name.len() + 4);
    for (index, c) in name.chars().enumerate() {
        if c.is_uppercase() && index > 0 {
            words.push(' ');
            words.extend(c.to_lowercase());
        } else {
            words.push(c);
        }
    }
    if words.is_empty() { "Message".to_owned() } else { words }
}

/// A `formattedText` as styled words. TDLib counts in UTF-16 units; the
/// model in bytes. Of entities that overlap, the first (the outer) is kept.
pub fn formatted(formatted: &Value) -> Text {
    let text = string(&formatted["text"]).to_owned();
    let mut spans: Vec<Span> = Vec::new();
    for entity in items(&formatted["entities"]) {
        let (offset, length) = (int(&entity["offset"]), int(&entity["length"]));
        let range = utf16_to_byte(&text, offset)..utf16_to_byte(&text, offset.saturating_add(length));
        let words = text.get(range.clone()).unwrap_or_default();
        let style = match kind(&entity["type"]) {
            "textEntityTypeBold" => Style::Bold,
            "textEntityTypeItalic" => Style::Italic,
            "textEntityTypeUnderline" => Style::Underline,
            "textEntityTypeStrikethrough" => Style::Strike,
            "textEntityTypeSpoiler" => Style::Spoiler,
            "textEntityTypeBlockQuote" | "textEntityTypeExpandableBlockQuote" => Style::Quote,
            "textEntityTypeCode" | "textEntityTypePre" | "textEntityTypePreCode" => Style::Code,
            "textEntityTypeUrl" if words.contains("://") => Style::Link(words.to_owned()),
            "textEntityTypeUrl" => Style::Link(format!("https://{words}")),
            "textEntityTypeTextUrl" => Style::Link(string(&entity["type"]["url"]).to_owned()),
            "textEntityTypeEmailAddress" => Style::Link(format!("mailto:{words}")),
            // "@name" leads to whoever goes by it; a name without one, to its user.
            "textEntityTypeMention" => Style::Mention(words.to_owned()),
            "textEntityTypeMentionName" => Style::Mention(match int(&entity["type"]["user_id"]) {
                0 => String::new(),
                user => format!("user:{user}"),
            }),
            "textEntityTypeBotCommand" => Style::Mention(String::new()),
            "textEntityTypeHashtag" | "textEntityTypeCashtag" => Style::Tag,
            _ => continue,
        };
        if range.is_empty() || spans.last().is_some_and(|last| range.start < last.range.end) {
            continue;
        }
        spans.push(Span { range, style });
    }
    Text { text, spans }
}

/// The byte of `text` at which its UTF-16 unit number `units` starts; the
/// end of the text when it has fewer.
fn utf16_to_byte(text: &str, units: i64) -> usize {
    let mut seen = 0;
    for (byte, c) in text.char_indices() {
        if seen >= units {
            return byte;
        }
        seen += c.len_utf16() as i64;
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::tests::{cache, text_message};
    use super::*;

    #[test]
    fn messages_keep_their_sender_answer_and_state() {
        let (mut cache, mut out) = cache();
        let mut message = text_message(12, 7, 7, "hello");
        message["reply_to"] = json!({"@type": "messageReplyToMessage", "chat_id": 7, "message_id": 9});
        message["edit_date"] = json!(5);
        message["sending_state"] = json!({"@type": "messageSendingStatePending"});
        message["forward_info"] = json!({"origin": {"@type": "messageOriginHiddenUser", "sender_name": "Ana"}});
        let message = cache.message(&message, &mut out);
        assert_eq!((message.sender, message.sender_name.as_str(), message.sender_photo), (7, "Mara Lind", Some(31)));
        assert_eq!((message.reply_to, message.edited, message.delivery), (Some(9), true, Delivery::Sending));
        assert_eq!((message.forwarded_from.as_str(), message.body.text()), ("Ana", "hello"));
        assert_eq!((message.topic, message.album, message.views, message.comments), (0, 0, 0, None));
        assert!(message.link.is_none() && message.reactions.is_empty() && message.buttons.is_empty());
        assert!(!message.pinned && !message.mentions_me);
        // Hostile or half a message: defaults, never a panic.
        let empty = cache.message(
            &json!({"content": 5, "sender_id": "x", "reply_to": [], "topic_id": 4, "interaction_info": "many",
            "reply_markup": {"@type": "replyMarkupInlineKeyboard", "rows": [3, [4]]}, "media_album_id": {}}),
            &mut out,
        );
        assert_eq!(
            (empty.id, empty.reply_to, empty.sender_name.as_str(), empty.topic, empty.album),
            (0, None, "", 0, 0)
        );
        assert_eq!(empty.buttons, [[Button { label: String::new(), press: Press::Nothing }]]);
        assert!(empty.reactions.is_empty() && empty.comments.is_none());
    }

    #[test]
    fn messages_say_where_they_are_and_what_was_made_of_them() {
        let (mut cache, mut out) = cache();
        let mut message = text_message(12, -1_000_000_000_050, 7, "the plan");
        message["topic_id"] = json!({"@type": "messageTopicForum", "forum_topic_id": 5});
        message["is_pinned"] = json!(true);
        message["is_channel_post"] = json!(true);
        message["contains_unread_mention"] = json!(true);
        message["media_album_id"] = json!("13579246801234567");
        message["interaction_info"] = json!({"view_count": 420, "reply_info": {"reply_count": 3}, "reactions": {"reactions": [
            {"type": {"@type": "reactionTypeEmoji", "emoji": "👍"}, "total_count": 2, "is_chosen": true},
            {"type": {"@type": "reactionTypeCustomEmoji", "custom_emoji_id": "5"}, "total_count": 9},
            {"type": {"@type": "reactionTypePaid"}, "total_count": 4},
            {"type": {"@type": "reactionTypeEmoji", "emoji": "🎉"}, "total_count": 6}]}});
        message["reply_markup"] = json!({"@type": "replyMarkupInlineKeyboard", "rows": [
            [{"text": "Open", "type": {"@type": "inlineKeyboardButtonTypeUrl", "url": "https://adelite.example"}},
             {"text": "Yes", "type": {"@type": "inlineKeyboardButtonTypeCallback", "data": "eWVz"}}],
            [{"text": "Pay", "type": {"@type": "inlineKeyboardButtonTypeBuy"}}], []]});
        let told = cache.message(&message, &mut out);
        assert_eq!((told.topic, told.pinned, told.mentions_me, told.album), (5, true, true, 13_579_246_801_234_567));
        assert_eq!((told.views, told.comments), (420, Some(3)));
        let reaction = |emoji: &str, count, chosen| Reaction { emoji: emoji.into(), count, chosen };
        assert_eq!(told.reactions, [reaction("🎉", 6, false), reaction("👍", 2, true)], "most used first, emoji only");
        let button = |label: &str, press| Button { label: label.into(), press };
        assert_eq!(
            told.buttons,
            [
                vec![
                    button("Open", Press::Link("https://adelite.example".into())),
                    button("Yes", Press::Data("eWVz".into()))
                ],
                vec![button("Pay", Press::Nothing)]
            ]
        );
        // The comments under a post are a thread; a message that is no post has none to count.
        message["topic_id"] = json!({"@type": "messageTopicThread", "message_thread_id": 4096});
        message["is_channel_post"] = json!(false);
        let told = cache.message(&message, &mut out);
        assert_eq!((told.topic, told.comments), (-4096, None));
        assert_eq!(topic(&json!({"@type": "messageTopicSavedMessages", "saved_messages_topic_id": 7})), 0);
        assert_eq!(topic(&json!({"@type": "messageTopicForum", "forum_topic_id": "x"})), 0);
        // A reply keyboard is not buttons under the message.
        message["reply_markup"] = json!({"@type": "replyMarkupShowKeyboard", "rows": [[{"text": "Hi"}]]});
        assert!(cache.message(&message, &mut out).buttons.is_empty());
    }

    #[test]
    fn what_people_made_of_a_message_is_told_when_it_changes() {
        let (mut cache, mut out) = cache();
        let channel = json!({"id": -1_000_000_000_050_i64, "type": {"@type": "chatTypeSupergroup", "supergroup_id": 50, "is_channel": true}});
        cache.chat(&channel, &mut out);
        let info = json!({"view_count": 9, "reply_info": {"reply_count": 2},
            "reactions": {"reactions": [{"type": {"@type": "reactionTypeEmoji", "emoji": "🔥"}, "total_count": 1}]}});
        let update = |chat: i64| json!({"chat_id": chat, "message_id": 12, "interaction_info": info});
        let Event::Counts { chat, message, reactions, views, comments } = cache.counts(&update(-1_000_000_000_050))
        else {
            panic!("counts")
        };
        assert_eq!((chat, message, views, comments), (-1_000_000_000_050, 12, 9, Some(2)));
        assert_eq!(reactions, [Reaction { emoji: "🔥".into(), count: 1, chosen: false }]);
        assert!(
            matches!(cache.counts(&update(7)), Event::Counts { comments: None, views: 9, .. }),
            "a thread in a group is no post"
        );
        let nothing = cache.counts(&json!({"chat_id": 7, "interaction_info": null}));
        assert_eq!(nothing, Event::Counts { chat: 7, message: 0, reactions: Vec::new(), views: 0, comments: None });
    }

    #[test]
    fn a_link_shows_the_page_it_leads_to() {
        let (mut cache, mut out) = cache();
        out.clear();
        let mut message = text_message(12, 7, 7, "adelite.example");
        message["content"]["link_preview"] = json!({"url": "https://adelite.example/", "site_name": "Adelite",
            "title": "A quiet desk", "description": {"text": "Chat, mail and documents."},
            "type": {"@type": "linkPreviewTypeArticle", "photo": {"sizes": [
                {"width": 320, "height": 180, "photo": {"id": 4}}, {"width": 1280, "height": 720, "photo": {"id": 5}}]}}});
        let page = cache.message(&message, &mut out).link.expect("a preview");
        assert_eq!(
            (page.url.as_str(), page.site.as_str(), page.title.as_str()),
            ("https://adelite.example/", "Adelite", "A quiet desk")
        );
        assert_eq!(
            (page.description.as_str(), page.photo, page.width, page.height),
            ("Chat, mail and documents.", Some(5), 1280, 720)
        );
        // A video's cover; without one, its still when that is a picture.
        let video = |cover: Value, format: &str| {
            json!({"url": "https://v.example", "type": {"@type": "linkPreviewTypeVideo", "cover": cover,
            "video": {"thumbnail": {"format": {"@type": format}, "width": 320, "height": 200, "file": {"id": 8}}}}})
        };
        let cover = json!({"sizes": [{"width": 640, "height": 360, "photo": {"id": 6}}]});
        assert_eq!(link_of(&video(cover, "thumbnailFormatJpeg")), (Some(6), 640, 360));
        assert_eq!(link_of(&video(json!(null), "thumbnailFormatJpeg")), (Some(8), 320, 200));
        assert_eq!(link_of(&video(json!(null), "thumbnailFormatWebm")), (None, 0, 0));
        assert_eq!(
            link_of(
                &json!({"url": "https://x.example", "type": {"@type": "linkPreviewTypeMessage"}, "description": 4})
            ),
            (None, 0, 0)
        );
        assert!(link(&json!({"title": "No address"}), &mut out).is_none() && link(&json!(null), &mut out).is_none());
        assert!(out.is_empty());
    }

    fn link_of(preview: &Value) -> (Option<FileId>, u32, u32) {
        let page = link(preview, &mut Vec::new()).expect("a preview");
        (page.photo, page.width, page.height)
    }

    #[test]
    fn content_is_words_a_picture_a_file_or_named() {
        let (cache, mut out) = cache();
        out.clear();
        let photo = json!({"@type": "messagePhoto", "caption": {"text": "the roof"}, "photo": {"sizes": [
            {"width": 90, "height": 60, "photo": {"id": 1}},
            {"width": 1280, "height": 853, "photo": {"id": 2, "local": {"path": "/f/2.jpg", "is_downloading_completed": true}}},
            {"width": 2560, "height": 1706, "photo": {"id": 3}}]}});
        let Body::Photo { file, full, width, height, caption } = cache.body(&photo, "Mara", &mut out) else {
            panic!("a photo")
        };
        assert_eq!((file, full, width, height, caption.text.as_str()), (Some(2), Some(3), 2560, 1706, "the roof"));
        assert_eq!(out, [Event::File { file: 2, path: "/f/2.jpg".into() }]);
        let document = json!({"@type": "messageDocument", "document": {"file_name": "q3.pdf", "document": {"id": 9, "size": 2048}}});
        assert!(
            matches!(cache.body(&document, "", &mut out), Body::File { file: Some(9), size: 2048, name, .. } if name == "q3.pdf")
        );
        let preview = |content: Value| cache.body(&content, "Mara", &mut Vec::new()).preview();
        assert_eq!(preview(json!({"@type": "messageSticker", "sticker": {"emoji": "😀"}})), "😀 Sticker");
        assert_eq!(preview(json!({"@type": "messageGiveawayCreated"})), "Giveaway created");
        assert_eq!(preview(json!({"@type": "messageChatAddMembers", "member_user_ids": [8]})), "Mara added Jonas");
        assert_eq!(preview(json!({"@type": "messageChatDeleteMember", "user_id": 7})), "Mara removed Mara Lind");
        assert_eq!(preview(json!({"@type": "messagePhoto", "photo": {"sizes": []}})), "Photo");
        assert_eq!(preview(json!({"@type": "messageExpiredVideo"})), "Video (expired)");
        assert_eq!(preview(json!(null)), "Message");
    }

    #[test]
    fn what_moves_has_a_still_only_when_that_is_a_picture() {
        let (cache, _) = cache();
        let body = |content: Value| cache.body(&content, "", &mut Vec::new());
        let thumbnail =
            |format: &str| json!({"format": {"@type": format}, "width": 320, "height": 180, "file": {"id": 20}});
        let video = json!({"@type": "messageVideo", "caption": {"text": "the roof"}, "video": {"duration": 12, "width": 1920,
            "height": 1080, "thumbnail": thumbnail("thumbnailFormatJpeg"), "video": {"id": 21}}});
        let Body::Video { motion, still, file, width, height, seconds, caption } = body(video.clone()) else {
            panic!("a video")
        };
        assert_eq!((motion, still, file, width, height, seconds), (Motion::Video, Some(20), Some(21), 1920, 1080, 12));
        assert_eq!(caption.text, "the roof");
        let mut covered = video;
        covered["cover"] = json!({"sizes": [{"width": 1280, "height": 720, "photo": {"id": 22}}]});
        assert!(matches!(body(covered), Body::Video { still: Some(22), .. }), "the cover goes before the frame");
        let gif = json!({"@type": "messageAnimation", "animation": {"duration": 3, "width": 480, "height": 270,
            "thumbnail": thumbnail("thumbnailFormatWebm"), "animation": {"id": 23}}});
        assert!(matches!(body(gif), Body::Video { motion: Motion::Gif, still: None, file: Some(23), width: 480, .. }));
        let round = json!({"@type": "messageVideoNote", "video_note": {"duration": 9, "length": 384,
            "thumbnail": thumbnail("thumbnailFormatPng"), "video": {"id": 24}}});
        let Body::Video { motion, still, file, width, height, .. } = body(round) else { panic!("a round video") };
        assert_eq!((motion, still, file, width, height), (Motion::Round, Some(20), Some(24), 384, 384));
        // Nothing of it reads: a video still, of no size that divides by zero.
        let Body::Video { still, file, width, height, seconds, .. } =
            body(json!({"@type": "messageVideo", "video": "x"}))
        else {
            panic!("a video")
        };
        assert_eq!((still, file, width, height, seconds), (None, None, 1, 1, 0));
    }

    #[test]
    fn stickers_and_sound_are_what_the_app_can_show() {
        let (cache, _) = cache();
        let body = |content: Value| cache.body(&content, "", &mut Vec::new());
        let sticker = |format: &str, thumbnail: &str| {
            json!({"@type": "messageSticker", "sticker": {"emoji": "🎉", "width": 512,
            "height": 512, "format": {"@type": format}, "sticker": {"id": 30},
            "thumbnail": {"format": {"@type": thumbnail}, "file": {"id": 31}}}})
        };
        let shown = |content: Value| match body(content) {
            Body::Sticker { file, emoji, width, height } => (file, emoji, width, height),
            other => panic!("a sticker, not {other:?}"),
        };
        assert_eq!(shown(sticker("stickerFormatWebp", "thumbnailFormatWebp")), (Some(30), "🎉".into(), 512, 512));
        assert_eq!(
            shown(sticker("stickerFormatTgs", "thumbnailFormatWebp")).0,
            Some(31),
            "one that moves, by its still"
        );
        assert_eq!(shown(sticker("stickerFormatWebm", "thumbnailFormatTgs")).0, None);
        assert_eq!(shown(json!({"@type": "messageSticker"})), (None, String::new(), 1, 1));
        let voice = json!({"@type": "messageVoiceNote", "caption": {"text": "listen"}, "voice_note": {"duration": 7, "voice": {"id": 40}}});
        assert!(matches!(body(voice), Body::Audio { file: Some(40), voice: true, seconds: 7, title, caption }
            if title.is_empty() && caption.text == "listen"));
        let audio = |performer: &str, title: &str| {
            json!({"@type": "messageAudio", "audio": {"duration": 200, "performer": performer,
            "title": title, "file_name": "track01.mp3", "audio": {"id": 41}}})
        };
        let title = |content: Value| match body(content) {
            Body::Audio { voice: false, file: Some(41), title, .. } => title,
            other => panic!("music, not {other:?}"),
        };
        assert_eq!(title(audio("Nils Frahm", "Says")), "Nils Frahm – Says");
        assert_eq!((title(audio("", "Says")), title(audio("Nils Frahm", ""))), ("Says".into(), "Nils Frahm".into()));
        assert_eq!(title(audio("", "")), "track01.mp3");
        assert!(matches!(
            body(json!({"@type": "messageAudio", "audio": 3})),
            Body::Audio { file: None, seconds: 0, .. }
        ));
    }

    #[test]
    fn a_poll_is_told_again_when_it_changes() {
        let (mut cache, mut out) = cache();
        let poll = |votes: i64, closed: bool| {
            json!({"id": "77", "question": {"text": "Lunch?"}, "total_voter_count": votes,
            "is_closed": closed, "allows_multiple_answers": true, "type": {"@type": "pollTypeQuiz"}, "options": [
                {"text": {"text": "Soup"}, "voter_count": votes, "vote_percentage": 100, "is_chosen": true},
                {"text": {"text": "Bread"}, "vote_percentage": 250, "is_being_chosen": true}, "x"]})
        };
        let mut message = text_message(12, 7, 7, "");
        message["content"] = json!({"@type": "messagePoll", "poll": poll(1, false)});
        let Body::Poll(told) = cache.message(&message, &mut out).body else { panic!("a poll") };
        assert_eq!(
            (told.question.as_str(), told.voters, told.closed, told.multiple, told.quiz),
            ("Lunch?", 1, false, true, true)
        );
        let option = |text: &str, votes, percent, chosen| PollOption { text: text.into(), votes, percent, chosen };
        assert_eq!(
            told.options,
            [option("Soup", 1, 100, true), option("Bread", 0, 100, true), option("", 0, 0, false)]
        );
        assert!(told.answered());
        // The same poll shown by another message, and told of the first twice.
        message["id"] = json!(13);
        cache.message(&message, &mut out);
        cache.message(&message, &mut out);
        cache.shows_poll(7, 14, &json!({"@type": "messageText", "poll": {"id": 77}}));
        out.clear();
        cache.polled(&poll(4, true), &mut out);
        let told: Vec<_> = out
            .iter()
            .map(|event| match event {
                Event::Body { chat, message, body: Body::Poll(poll) } => (*chat, *message, poll.voters, poll.closed),
                other => panic!("a poll, not {other:?}"),
            })
            .collect();
        assert_eq!(told, [(7, 12, 4, true), (7, 13, 4, true)]);
        cache.polled(&json!({"id": 78}), &mut out);
        assert_eq!(out.len(), 2, "a poll no message shows");
        // Only so many are remembered: the oldest are let go.
        for id in 0..POLLS as i64 {
            cache.shows_poll(8, id, &json!({"@type": "messagePoll", "poll": {"id": 80}}));
        }
        out.clear();
        cache.polled(&poll(4, true), &mut out);
        assert!(out.is_empty());
        let regular = super::poll(&json!({"type": {"@type": "pollTypeRegular"}, "options": 4}));
        assert_eq!(regular, Poll::default());
    }

    #[test]
    fn the_chat_says_what_happened_in_it() {
        let (mut cache, mut out) = cache();
        cache.community(&json!({"id": 3, "name": "Studio"}), &mut out);
        let said = |content: Value| match cache.body(&content, "Mara", &mut Vec::new()) {
            Body::Service(text) => text,
            other => panic!("a service line, not {other:?}"),
        };
        assert_eq!(said(json!({"@type": "messageForumTopicCreated", "name": "Roof"})), "Mara created the topic “Roof”");
        assert_eq!(
            said(json!({"@type": "messageForumTopicEdited", "name": "Roofs"})),
            "Mara renamed the topic to “Roofs”"
        );
        assert_eq!(
            said(json!({"@type": "messageForumTopicEdited", "edit_icon_custom_emoji_id": true})),
            "Mara changed the topic's icon"
        );
        assert_eq!(
            said(json!({"@type": "messageForumTopicIsClosedToggled", "is_closed": true})),
            "Mara closed the topic"
        );
        assert_eq!(said(json!({"@type": "messageForumTopicIsClosedToggled"})), "Mara reopened the topic");
        assert_eq!(said(json!({"@type": "messageVideoChatStarted"})), "Video chat started");
        assert_eq!(said(json!({"@type": "messageVideoChatScheduled", "start_date": 5})), "Video chat scheduled");
        assert_eq!(
            said(json!({"@type": "messageVideoChatEnded", "duration": 754})),
            "Video chat ended after 12 minutes"
        );
        assert_eq!(said(json!({"@type": "messageVideoChatEnded"})), "Video chat ended");
        assert_eq!(
            said(json!({"@type": "messageChatJoinFromCommunity", "community_id": 3})),
            "Mara joined from Studio"
        );
        assert_eq!(
            said(json!({"@type": "messageChatAddedToCommunity", "community_id": 4})),
            "Mara added the chat to a community"
        );
        assert_eq!(
            said(json!({"@type": "messageChatRemovedFromCommunity"})),
            "Mara took the chat out of its community"
        );
        let timer = |seconds: i64| {
            said(json!({"@type": "messageChatSetMessageAutoDeleteTime", "message_auto_delete_time": seconds}))
        };
        assert_eq!(timer(86_400), "Mara set messages to be deleted after 1 day");
        assert_eq!(timer(0), "Mara turned off deleting messages after a while");
        assert_eq!(
            said(json!({"@type": "messageChatUpgradeTo", "supergroup_id": 5})),
            "The group was made a supergroup"
        );
        assert_eq!(said(json!({"@type": "messagePinMessage", "message_id": 5})), "Mara pinned a message");
        assert_eq!(
            (span(45), span(7200), span(1_209_600), span(2_592_000)),
            ("45 seconds".into(), "2 hours".into(), "2 weeks".into(), "1 month".into())
        );
    }

    #[test]
    fn a_rich_message_is_read_as_its_words() {
        let (cache, _) = cache();
        let plain = |text: &str| json!({"@type": "richTextPlain", "text": text});
        let content = json!({"@type": "messageRichMessage", "message": {"blocks": [
            {"@type": "pageBlockHeader", "header": plain("Plan")},
            {"@type": "pageBlockParagraph", "text": {"@type": "richTexts", "texts": [
                {"@type": "richTextBold", "text": plain("Roof")}, plain(" first, "),
                {"@type": "richTextCustomEmoji", "alternative_text": "🙂"}]}},
            {"@type": "pageBlockDivider"},
            {"@type": "pageBlockList", "items": [{"blocks": [{"@type": "pageBlockParagraph", "text": plain("tiles")}]}]},
            {"@type": "pageBlockPhoto", "caption": {"text": plain("the roof"), "credit": plain("")}}, 7]}});
        assert_eq!(cache.body(&content, "", &mut Vec::new()).text(), "Plan\nRoof first, 🙂\ntiles\nthe roof");
        let empty = cache.body(
            &json!({"@type": "messageRichMessage", "message": {"blocks": [{"@type": "pageBlockDivider"}]}}),
            "",
            &mut Vec::new(),
        );
        assert_eq!(empty.preview(), "Message");
        // Nested deeper than any message is: given up on, not followed to the end of the stack.
        let mut deep = plain("x");
        for _ in 0..100 {
            deep = json!({"@type": "richTextBold", "text": deep});
        }
        let mut words = String::new();
        rich_text(&deep, 0, &mut words);
        assert!(words.is_empty());
    }

    #[test]
    fn entities_count_in_utf16_and_become_byte_spans() {
        // "😀" is two UTF-16 units and four bytes; "é" one and two.
        let text = formatted(&json!({"text": "😀 bold é link.example", "entities": [
            {"offset": 3, "length": 4, "type": {"@type": "textEntityTypeBold"}},
            {"offset": 3, "length": 2, "type": {"@type": "textEntityTypeItalic"}},
            {"offset": 10, "length": 12, "type": {"@type": "textEntityTypeUrl"}},
            {"offset": 500, "length": 4, "type": {"@type": "textEntityTypeBold"}},
            {"offset": 0, "length": -3, "type": {"@type": "textEntityTypeCode"}}]}));
        assert_eq!(
            text.spans,
            [
                Span { range: 5..9, style: Style::Bold },
                Span { range: 13..25, style: Style::Link("https://link.example".into()) }
            ],
            "nested, out of range and negative entities are dropped"
        );
        assert_eq!(&text.text[5..9], "bold");
        assert_eq!(formatted(&json!({"text": 7})), Text::default());
    }

    #[test]
    fn mentions_tags_and_what_is_hidden_or_quoted_are_styled() {
        let entity =
            |offset: i64, length: i64, about: Value| json!({"offset": offset, "length": length, "type": about});
        let text = formatted(&json!({"text": "@mara Jonas #roof $ADL /start hidden quoted more", "entities": [
            entity(0, 5, json!({"@type": "textEntityTypeMention"})),
            entity(6, 5, json!({"@type": "textEntityTypeMentionName", "user_id": 8})),
            entity(12, 5, json!({"@type": "textEntityTypeHashtag"})),
            entity(18, 4, json!({"@type": "textEntityTypeCashtag"})),
            entity(23, 6, json!({"@type": "textEntityTypeBotCommand"})),
            entity(30, 6, json!({"@type": "textEntityTypeSpoiler"})),
            entity(37, 6, json!({"@type": "textEntityTypeBlockQuote"})),
            entity(44, 4, json!({"@type": "textEntityTypeExpandableBlockQuote"})),
            entity(44, 4, json!({"@type": "textEntityTypeCustomEmoji", "custom_emoji_id": "5"}))]}));
        let styles: Vec<(&str, &Style)> =
            text.spans.iter().map(|span| (&text.text[span.range.clone()], &span.style)).collect();
        assert_eq!(
            styles,
            [
                ("@mara", &Style::Mention("@mara".into())),
                ("Jonas", &Style::Mention("user:8".into())),
                ("#roof", &Style::Tag),
                ("$ADL", &Style::Tag),
                ("/start", &Style::Mention(String::new())),
                ("hidden", &Style::Spoiler),
                ("quoted", &Style::Quote),
                ("more", &Style::Quote),
            ]
        );
        let unnamed = formatted(
            &json!({"text": "Jonas", "entities": [entity(0, 5, json!({"@type": "textEntityTypeMentionName"}))]}),
        );
        assert_eq!(unnamed.spans, [Span { range: 0..5, style: Style::Mention(String::new()) }], "it leads nowhere");
    }
}
