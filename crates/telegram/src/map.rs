//! TDLib's JSON objects as the chat model, and what has to be remembered
//! to make them: who the users are, how large the groups are, and every
//! chat as last told, since TDLib sends a chat's changes one field at a time.
//! The lists the chats are in, the communities and a forum's topics are
//! kept here too; what a message holds is read in `content`.
//!
//! Nothing here trusts the JSON: a missing or mistyped field reads as
//! empty, zero or false, never a panic.

mod content;

use std::collections::{HashMap, HashSet, VecDeque};

use adelite_chat::{
    Chat, ChatId, Event, FileId, Info, Kind, List, ListInfo, Member, Message, MessageId, Place, Role, SenderId, Topic,
    TopicId,
};
use serde_json::{Value, json};

pub use content::topic;

/// Below this a chat's id is a supergroup's (or a channel's): its id negated, less this.
const SUPERGROUPS: i64 = -1_000_000_000_000;
/// How many messages are remembered as showing a poll, so a poll that changes finds them.
const POLLS: usize = 400;

/// An object's `@type`.
pub fn kind(value: &Value) -> &str {
    value["@type"].as_str().unwrap_or_default()
}

/// A number. TDLib writes 64-bit ones as strings, which JSON numbers cannot hold.
pub fn int(value: &Value) -> i64 {
    value.as_i64().or_else(|| value.as_str()?.parse().ok()).unwrap_or(0)
}

/// A string; empty when it is not one.
pub fn string(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

/// A boolean; false when it is not one.
pub fn flag(value: &Value) -> bool {
    value.as_bool().unwrap_or(false)
}

/// How many of something: a number that is never negative.
pub fn count(value: &Value) -> u32 {
    int(value).clamp(0, i64::from(u32::MAX)) as u32
}

/// An array's elements; none when it is not one.
pub fn items(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

/// A user, as far as the chats need them.
#[derive(Debug, Default)]
struct User {
    /// First and last name.
    name: String,
    /// First name: who is typing, who wrote a group's latest message.
    first: String,
    photo: Option<FileId>,
    /// "online", "last seen recently", "bot", …
    status: String,
    /// When they were last here, when Telegram says exactly.
    last_seen: i64,
    /// The name they are found under, without its sign.
    handle: String,
    /// Their phone number, where the user may see it.
    phone: String,
    /// A program, which is never online or away.
    bot: bool,
}

/// A basic group or a supergroup (a channel is one), as far as its chat needs it.
#[derive(Debug, Default, Clone)]
struct Group {
    members: i64,
    channel: bool,
    /// Its messages are kept in topics.
    forum: bool,
    /// The user runs it: may write where only those who do may.
    runs: bool,
    /// The user is in it.
    member: bool,
    /// Whether the user's standing lets them put others in it; None leaves it to the chat's permissions.
    invites: Option<bool>,
    /// The same for changing its name and what it says about itself.
    edits: Option<bool>,
    /// The name it is found under, without its sign.
    handle: String,
}

/// Whom a chat is with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peer {
    /// A person, by their id.
    User(i64),
    /// A basic group.
    Group(i64),
    /// A supergroup or a channel.
    Supergroup(i64),
}

/// A chat as last told, and what its derived fields come from.
#[derive(Debug)]
struct Entry {
    chat: Chat,
    peer: Peer,
    /// The title Telegram gave it (the chat with oneself is shown under another).
    title: String,
    /// What the chat's permissions say about writing, adding people and renaming it.
    may_write: bool,
    may_invite: bool,
    may_edit: bool,
    /// Its own `chatNotificationSettings`, which may leave muting to a default.
    notify: Value,
}

/// Groups and channels that belong together.
#[derive(Debug, Default)]
struct Community {
    id: i64,
    name: String,
    /// The user is in it, so it is one of the lists.
    listed: bool,
    /// Its chats; none until its full information came.
    chats: Vec<ChatId>,
    /// Its `chatNotificationSettings`, which its chats fall back on.
    notify: Value,
    /// Its chats were asked for.
    asked: bool,
}

/// Everything remembered between updates.
#[derive(Debug, Default)]
pub struct Cache {
    /// The signed-in user's id.
    pub me: i64,
    users: HashMap<i64, User>,
    groups: HashMap<i64, Group>,
    supergroups: HashMap<i64, Group>,
    chats: HashMap<ChatId, Entry>,
    /// The user's folders in their order, and where the main list goes among them.
    folders: Vec<(i64, String)>,
    main_at: usize,
    communities: Vec<Community>,
    /// How many unread chats that are not muted each list has.
    unread: HashMap<List, u32>,
    /// How many chats Telegram says the archive has, and how many known ones are in it.
    archive_total: i64,
    archived: usize,
    /// The lists as last told, and whether they may have changed since.
    told: Vec<ListInfo>,
    stale: bool,
    /// Whether private chats, groups and channels are muted unless they say otherwise.
    scopes: [bool; 3],
    /// The topics of the forums the view asked about, each with what it says itself about muting.
    topics: HashMap<(ChatId, TopicId), (Topic, Option<bool>)>,
    forums: HashSet<ChatId>,
    /// Which message showed which poll, oldest first.
    polls: VecDeque<(i64, ChatId, MessageId)>,
}

impl Cache {
    /// Takes in a `user`. The chat with them, if there is one, changed.
    pub fn user(&mut self, user: &Value, out: &mut Vec<Event>) -> Option<Chat> {
        let id = int(&user["id"]);
        let (first, last) = (string(&user["first_name"]), string(&user["last_name"]));
        let name = if last.is_empty() { first.to_owned() } else { format!("{first} {last}") };
        let bot = kind(&user["type"]) == "userTypeBot";
        let (status, last_seen) = if bot { ("bot".to_owned(), 0) } else { presence(&user["status"]) };
        let known = User {
            name: if name.is_empty() { "Deleted account".to_owned() } else { name },
            first: first.to_owned(),
            photo: file(&user["profile_photo"]["small"], out),
            status,
            last_seen,
            handle: handle(&user["usernames"]),
            phone: string(&user["phone_number"]).to_owned(),
            bot,
        };
        self.users.insert(id, known);
        self.refresh(id)
    }

    /// A user came or went. The chat with them, if there is one, changed.
    pub fn user_status(&mut self, update: &Value) -> Option<Chat> {
        let id = int(&update["user_id"]);
        let user = self.users.get_mut(&id).filter(|user| !user.bot)?;
        (user.status, user.last_seen) = presence(&update["status"]);
        self.refresh(id)
    }

    /// Takes in a `basicGroup` or a `supergroup`. Its chat, if there is one, changed.
    pub fn group(&mut self, group: &Value, supergroup: bool) -> Option<Chat> {
        let id = int(&group["id"]);
        let status = &group["status"];
        let rights = &status["rights"];
        // Someone restricted may still be in it; an owner may have left.
        let inside = status["is_member"].as_bool().unwrap_or(true);
        let (runs, member, invites, edits) = match kind(status) {
            "chatMemberStatusCreator" => (true, inside, Some(true), Some(true)),
            "chatMemberStatusAdministrator" => {
                (true, true, Some(flag(&rights["can_invite_users"])), Some(flag(&rights["can_change_info"])))
            }
            "chatMemberStatusRestricted" => {
                // What the restriction leaves open, the chat's permissions still decide.
                let allowed = |what: &str| if flag(&status["permissions"][what]) { None } else { Some(false) };
                (false, inside, allowed("can_invite_users"), allowed("can_change_info"))
            }
            "chatMemberStatusLeft" | "chatMemberStatusBanned" => (false, false, Some(false), Some(false)),
            _ => (false, true, None, None),
        };
        let known = Group {
            members: int(&group["member_count"]),
            channel: flag(&group["is_channel"]),
            forum: flag(&group["is_forum"]),
            runs,
            member,
            invites,
            edits,
            handle: handle(&group["usernames"]),
        };
        // A chat's id says what it is with: a basic group's is its id negated,
        // a supergroup's the same below minus a trillion.
        if supergroup {
            self.supergroups.insert(id, known);
            self.refresh(SUPERGROUPS - id)
        } else {
            self.groups.insert(id, known);
            self.refresh(-id)
        }
    }

    /// Takes in a `chat`, new or told again in full.
    pub fn chat(&mut self, chat: &Value, out: &mut Vec<Event>) -> Option<Chat> {
        let id = int(&chat["id"]);
        let about = &chat["type"];
        let (peer, kind) = match kind(about) {
            "chatTypePrivate" | "chatTypeSecret" => (Peer::User(int(&about["user_id"])), Kind::Private),
            "chatTypeBasicGroup" => (Peer::Group(int(&about["basic_group_id"])), Kind::Group),
            _ => {
                let kind = if flag(&about["is_channel"]) { Kind::Channel } else { Kind::Group };
                (Peer::Supergroup(int(&about["supergroup_id"])), kind)
            }
        };
        let permissions = &chat["permissions"];
        let mut entry = Entry {
            chat: Chat {
                id,
                kind,
                photo: file(&chat["photo"]["small"], out),
                unread: count(&chat["unread_count"]),
                mentioned: int(&chat["unread_mention_count"]) > 0,
                marked_unread: flag(&chat["is_marked_as_unread"]),
                read_inbox: int(&chat["last_read_inbox_message_id"]),
                read_outbox: int(&chat["last_read_outbox_message_id"]),
                draft: draft(&chat["draft_message"]),
                ..Chat::default()
            },
            peer,
            title: string(&chat["title"]).to_owned(),
            may_write: flag(&permissions["can_send_basic_messages"]),
            may_invite: flag(&permissions["can_invite_users"]),
            may_edit: flag(&permissions["can_change_info"]),
            notify: chat["notification_settings"].clone(),
        };
        items(&chat["positions"]).for_each(|position| place(&mut entry.chat, position));
        entry.chat.last = self.preview(&chat["last_message"], kind, out);
        let archived = entry.chat.place(List::Archive).is_some();
        let was = self.chats.insert(id, entry).is_some_and(|old| old.chat.place(List::Archive).is_some());
        self.archive(was, archived);
        self.refresh(id)
    }

    /// Applies one of the updates that change a field of a chat.
    pub fn chat_update(&mut self, update: &Value, out: &mut Vec<Event>) -> Option<Chat> {
        let id = int(&update["chat_id"]);
        let chat_kind = self.chats.get(&id)?.chat.kind;
        // The latest message names its sender, which is looked up before the chat is borrowed.
        let last =
            (kind(update) == "updateChatLastMessage").then(|| self.preview(&update["last_message"], chat_kind, out));
        let entry = self.chats.get_mut(&id)?;
        let chat = &mut entry.chat;
        let was = chat.place(List::Archive).is_some();
        match kind(update) {
            "updateChatTitle" => string(&update["title"]).clone_into(&mut entry.title),
            "updateChatPhoto" => chat.photo = file(&update["photo"]["small"], out),
            "updateChatPermissions" => {
                let permissions = &update["permissions"];
                entry.may_write = flag(&permissions["can_send_basic_messages"]);
                entry.may_invite = flag(&permissions["can_invite_users"]);
                entry.may_edit = flag(&permissions["can_change_info"]);
            }
            "updateChatLastMessage" => {
                chat.last = last.flatten();
                items(&update["positions"]).for_each(|position| place(chat, position));
            }
            "updateChatDraftMessage" => {
                chat.draft = draft(&update["draft_message"]);
                items(&update["positions"]).for_each(|position| place(chat, position));
            }
            "updateChatPosition" => place(chat, &update["position"]),
            "updateChatReadInbox" => {
                chat.read_inbox = int(&update["last_read_inbox_message_id"]);
                chat.unread = count(&update["unread_count"]);
            }
            "updateChatReadOutbox" => chat.read_outbox = int(&update["last_read_outbox_message_id"]),
            "updateChatUnreadMentionCount" | "updateMessageMentionRead" => {
                chat.mentioned = int(&update["unread_mention_count"]) > 0;
            }
            "updateChatIsMarkedAsUnread" => chat.marked_unread = flag(&update["is_marked_as_unread"]),
            "updateChatNotificationSettings" => entry.notify = update["notification_settings"].clone(),
            _ => return None,
        }
        let archived = entry.chat.place(List::Archive).is_some();
        self.archive(was, archived);
        self.refresh(id)
    }

    /// Counts a chat into or out of the archive, which is a list only while it has chats.
    fn archive(&mut self, was: bool, is: bool) {
        if was == is {
            return;
        }
        let archived = if is { self.archived + 1 } else { self.archived.saturating_sub(1) };
        self.stale |= (self.archived == 0) != (archived == 0);
        self.archived = archived;
    }

    /// Works out what a chat shows that other objects decide (its name, the
    /// line under it, what the user may do there, whether it is muted, its
    /// place in a community) and hands back a copy.
    fn refresh(&mut self, id: ChatId) -> Option<Chat> {
        let entry = self.chats.get_mut(&id)?;
        let chat = &mut entry.chat;
        chat.title.clone_from(&entry.title);
        (chat.status, chat.last_seen, chat.can_send) = (String::new(), 0, true);
        (chat.forum, chat.joined, chat.admin) = (false, true, false);
        match entry.peer {
            Peer::User(user) if user == self.me => "Saved Messages".clone_into(&mut chat.title),
            Peer::User(user) => {
                if let Some(user) = self.users.get(&user) {
                    chat.status.clone_from(&user.status);
                    chat.last_seen = user.last_seen;
                }
            }
            Peer::Group(group) | Peer::Supergroup(group) => {
                let groups = if matches!(entry.peer, Peer::Group(_)) { &self.groups } else { &self.supergroups };
                let unknown = Group::default();
                let group = groups.get(&group).unwrap_or(&unknown);
                chat.can_send = group.member && if group.channel { group.runs } else { entry.may_write || group.runs };
                (chat.forum, chat.joined, chat.admin) = (group.forum, group.member, group.runs);
                if group.members > 0 {
                    let noun = match (group.channel, group.members) {
                        (true, 1) => "subscriber",
                        (true, _) => "subscribers",
                        (false, 1) => "member",
                        (false, _) => "members",
                    };
                    chat.status = format!("{} {noun}", group.members);
                }
            }
        }
        // A community's chats are listed under it, wherever else they are: by
        // their place in the main list or the archive, else by their latest message.
        chat.places.retain(|place| !matches!(place.list, List::Community(_)));
        let own = [List::Main, List::Archive].iter().find_map(|list| chat.place(*list).map(|place| place.order));
        let order = own.or(chat.last.as_ref().map(|last| last.date).filter(|date| *date > 0)).unwrap_or(1);
        let mut community = None;
        for known in self.communities.iter().filter(|known| known.chats.contains(&id)) {
            community = community.or(Some(&known.notify));
            if known.listed {
                chat.places.push(Place { list: List::Community(known.id), order, pinned: false });
            }
        }
        chat.muted = muted(&entry.notify, community, self.scopes[scope(chat.kind)]);
        Some(chat.clone())
    }

    /// Tells these chats again: something they derive from changed.
    fn retell(&mut self, chats: &[ChatId], out: &mut Vec<Event>) {
        for chat in chats {
            let chat = self.refresh(*chat);
            out.extend(chat.map(Event::Chat));
        }
    }

    /// Takes in a `chat` an answer carries when it is not known yet, so the
    /// view has heard of every chat it is pointed at. Its id.
    pub fn tell(&mut self, chat: &Value, out: &mut Vec<Event>) -> ChatId {
        let id = int(&chat["id"]);
        if id != 0 && !self.chats.contains_key(&id) {
            let chat = self.chat(chat, out);
            out.extend(chat.map(Event::Chat));
        }
        id
    }

    /// The ids of a `chats` answer that the view has heard of.
    pub fn known(&self, chats: &Value) -> Vec<ChatId> {
        items(&chats["chat_ids"]).map(int).filter(|id| self.chats.contains_key(id)).collect()
    }

    /// Whom the chat `id` is with; from its id where the chat is not known.
    pub fn peer(&self, id: ChatId) -> Peer {
        match self.chats.get(&id) {
            Some(entry) => entry.peer,
            None if id > 0 => Peer::User(id),
            None if id > SUPERGROUPS => Peer::Group(-id),
            None => Peer::Supergroup(SUPERGROUPS - id),
        }
    }

    /// Whether the chat `id` is a channel.
    pub fn is_channel(&self, id: ChatId) -> bool {
        self.chats.get(&id).is_some_and(|entry| entry.chat.kind == Kind::Channel)
    }

    /// The latest message of the chat `id`, if it has one.
    pub fn last_message(&self, id: ChatId) -> Option<MessageId> {
        self.chats.get(&id)?.chat.last.as_ref().map(|last| last.id)
    }

    /// The `chatNotificationSettings` of the chat `id` as last told; ones
    /// that leave everything to the defaults where none were.
    pub fn notify(&self, id: ChatId) -> Value {
        match self.chats.get(&id) {
            Some(entry) if entry.notify.is_object() => entry.notify.clone(),
            _ => json!({
                "@type": "chatNotificationSettings", "use_default_mute_for": true, "use_default_sound": true,
                "use_default_show_preview": true, "use_default_mute_stories": true, "use_default_story_sound": true,
                "use_default_show_story_poster": true, "use_default_disable_pinned_message_notifications": true,
                "use_default_disable_mention_notifications": true,
            }),
        }
    }

    /// Takes in the `scopeNotificationSettings` of a kind of chat (a
    /// `NotificationSettingsScope`). The chats that leave muting to them
    /// are told again when it changed.
    pub fn scope(&mut self, of: &Value, settings: &Value, out: &mut Vec<Event>) {
        let index = match kind(of) {
            "notificationSettingsScopePrivateChats" => scope(Kind::Private),
            "notificationSettingsScopeGroupChats" => scope(Kind::Group),
            "notificationSettingsScopeChannelChats" => scope(Kind::Channel),
            _ => return,
        };
        let muted = int(&settings["mute_for"]) > 0;
        if std::mem::replace(&mut self.scopes[index], muted) == muted {
            return;
        }
        let follow = |entry: &&Entry| scope(entry.chat.kind) == index && own_mute(&entry.notify).is_none();
        let chats: Vec<ChatId> = self.chats.values().filter(follow).map(|entry| entry.chat.id).collect();
        for id in chats {
            // A community's settings may stand between a chat and its kind's.
            let was = self.chats.get(&id).is_some_and(|entry| entry.chat.muted);
            let chat = self.refresh(id).filter(|chat| chat.muted != was);
            out.extend(chat.map(Event::Chat));
        }
    }

    /// Takes in the user's folders (`updateChatFolders`).
    pub fn folders(&mut self, update: &Value) {
        let folder = |folder: &Value| (int(&folder["id"]), string(&folder["name"]["text"]["text"]).to_owned());
        self.folders = items(&update["chat_folders"]).map(folder).collect();
        self.main_at = int(&update["main_chat_list_position"]).max(0) as usize;
        self.stale = true;
    }

    /// Takes in how many unread chats a list has (`updateUnreadChatCount`).
    pub fn unread(&mut self, update: &Value) {
        let Some(list) = list(&update["chat_list"]) else { return };
        self.unread.insert(list, count(&update["unread_unmuted_count"]));
        if list == List::Archive {
            self.archive_total = int(&update["total_count"]);
        }
        self.stale = true;
    }

    /// The user signed in: the lists are told, even when Telegram says nothing about them.
    pub fn signed_in(&mut self) {
        self.stale = true;
    }

    /// The lists in the order they are shown, when they changed since they
    /// were last told: the folders with the main list among them, the
    /// communities, and the archive while it holds something.
    pub fn lists(&mut self) -> Option<Event> {
        if !std::mem::take(&mut self.stale) {
            return None;
        }
        let unread = &self.unread;
        let entry = |list: List, name: &str| ListInfo {
            list,
            name: name.to_owned(),
            unread: unread.get(&list).copied().unwrap_or(0),
        };
        let mut lists: Vec<ListInfo> = self.folders.iter().map(|(id, name)| entry(List::Folder(*id), name)).collect();
        lists.insert(self.main_at.min(lists.len()), entry(List::Main, "All"));
        let shown = self.communities.iter().filter(|community| community.listed && !community.chats.is_empty());
        lists.extend(shown.map(|community| entry(List::Community(community.id), &community.name)));
        if self.archive_total > 0 || self.archived > 0 {
            lists.push(entry(List::Archive, "Archive"));
        }
        if lists == self.told {
            return None;
        }
        self.told.clone_from(&lists);
        Some(Event::Lists(lists))
    }

    /// Takes in a `community`. True when its chats are still to be asked for.
    pub fn community(&mut self, community: &Value, out: &mut Vec<Event>) -> bool {
        let id = int(&community["id"]);
        if id == 0 {
            return false;
        }
        let status = kind(&community["status"]);
        let inside =
            ["communityMemberStatusCreator", "communityMemberStatusAdministrator", "communityMemberStatusMember"];
        let listed = flag(&community["have_access"]) && inside.contains(&status);
        if !self.communities.iter().any(|known| known.id == id) {
            self.communities.push(Community { id, ..Community::default() });
        }
        let Some(known) = self.communities.iter_mut().find(|known| known.id == id) else { return false };
        string(&community["name"]).clone_into(&mut known.name);
        let moved = std::mem::replace(&mut known.listed, listed) != listed;
        let ask = listed && !known.asked;
        known.asked |= listed;
        self.stale = true;
        if moved {
            // Its chats come into the list, or leave it.
            let chats = known.chats.clone();
            self.retell(&chats, out);
        }
        ask
    }

    /// Takes in a `communityFullInfo`: which chats the community `id` has.
    /// They, and the ones it no longer has, are told again.
    pub fn community_info(&mut self, id: i64, info: &Value, out: &mut Vec<Event>) {
        let Some(known) = self.communities.iter_mut().find(|known| known.id == id) else { return };
        let chats: Vec<ChatId> =
            items(&info["chats"]).map(|chat| int(&chat["chat_id"])).filter(|id| *id != 0).collect();
        let mut touched = std::mem::replace(&mut known.chats, chats.clone());
        known.notify = info["notification_settings"].clone();
        touched.extend(chats);
        touched.sort_unstable();
        touched.dedup();
        self.stale = true;
        self.retell(&touched, out);
    }

    /// What a community is called.
    fn community_name(&self, id: i64) -> &str {
        let known = self.communities.iter().find(|known| known.id == id && !known.name.is_empty());
        known.map_or("a community", |known| known.name.as_str())
    }

    /// Takes in a `forumTopic`, in full. None when it names no topic.
    pub fn topic(&mut self, topic: &Value, out: &mut Vec<Event>) -> Option<Topic> {
        let info = &topic["info"];
        let (chat, id) = (int(&info["chat_id"]), int(&info["forum_topic_id"]));
        if chat == 0 || id == 0 {
            return None;
        }
        let own = own_mute(&topic["notification_settings"]);
        let mut told = Topic {
            chat,
            id,
            order: int(&topic["order"]),
            pinned: flag(&topic["is_pinned"]),
            muted: own.unwrap_or_else(|| self.chat_muted(chat)),
            unread: count(&topic["unread_count"]),
            mentioned: int(&topic["unread_mention_count"]) > 0,
            last: self.preview(&topic["last_message"], Kind::Group, out),
            read_inbox: int(&topic["last_read_inbox_message_id"]),
            read_outbox: int(&topic["last_read_outbox_message_id"]),
            ..Topic::default()
        };
        describe(&mut told, info);
        self.forums.insert(chat);
        self.topics.insert((chat, id), (told.clone(), own));
        Some(told)
    }

    /// Whether the view asked for the topics of the chat `id`, so that one
    /// that is new there is worth fetching.
    pub fn follows_topics(&self, id: ChatId) -> bool {
        self.forums.contains(&id)
    }

    /// A topic's name, colour or state changed (a `forumTopicInfo`). None when the topic is not remembered.
    pub fn topic_info(&mut self, info: &Value) -> Option<Topic> {
        let (topic, _) = self.topics.get_mut(&(int(&info["chat_id"]), int(&info["forum_topic_id"])))?;
        describe(topic, info);
        Some(topic.clone())
    }

    /// Applies an `updateForumTopic`, which tells part of a topic. With the
    /// topic comes whether what was read moved: the update does not say how
    /// much is still unread. None when the topic is not remembered.
    pub fn topic_update(&mut self, update: &Value) -> Option<(Topic, bool)> {
        let chat = int(&update["chat_id"]);
        let muted = self.chat_muted(chat);
        let (topic, own) = self.topics.get_mut(&(chat, int(&update["forum_topic_id"])))?;
        let read = int(&update["last_read_inbox_message_id"]);
        let moved = std::mem::replace(&mut topic.read_inbox, read) != read;
        topic.pinned = flag(&update["is_pinned"]);
        topic.read_outbox = int(&update["last_read_outbox_message_id"]);
        topic.mentioned = int(&update["unread_mention_count"]) > 0;
        *own = own_mute(&update["notification_settings"]);
        topic.muted = own.unwrap_or(muted);
        Some((topic.clone(), moved))
    }

    /// A new message came into a topic. TDLib tells nothing of the topic
    /// then, so its latest message, its place and what is unread are worked
    /// out here. None when the topic is not remembered.
    pub fn topic_message(&mut self, json: &Value, message: &Message) -> Option<Topic> {
        if message.topic <= 0 || !self.topics.contains_key(&(message.chat, message.topic)) {
            return None;
        }
        let last = self.preview(json, Kind::Group, &mut Vec::new());
        let (topic, _) = self.topics.get_mut(&(message.chat, message.topic))?;
        topic.last = last;
        // As TDLib orders topics: by the date of the latest message, then its id on the server.
        topic.order = topic.order.max((message.date << 32) + (message.id >> 20));
        if !message.outgoing && message.id > topic.read_inbox {
            topic.unread += 1;
            topic.mentioned |= message.mentions_me;
        }
        Some(topic.clone())
    }

    /// Whether the chat `id` is muted, as last worked out.
    fn chat_muted(&self, id: ChatId) -> bool {
        self.chats.get(&id).is_some_and(|entry| entry.chat.muted)
    }

    /// What there is to know about the chat `id`, from its `userFullInfo`,
    /// `basicGroupFullInfo` or `supergroupFullInfo`.
    pub fn info(&self, id: ChatId, full: &Value) -> Info {
        let mut info = Info { chat: id, ..Info::default() };
        let named = |handle: &str| {
            if handle.is_empty() {
                Default::default()
            } else {
                (format!("@{handle}"), format!("https://t.me/{handle}"))
            }
        };
        match self.peer(id) {
            Peer::User(user) => {
                string(&full["bio"]["text"]).clone_into(&mut info.about);
                if let Some(user) = self.users.get(&user) {
                    (info.handle, info.link) = named(&user.handle);
                    if !user.phone.is_empty() {
                        info.phone = format!("+{}", user.phone.trim_start_matches('+'));
                    }
                }
            }
            peer @ (Peer::Group(group) | Peer::Supergroup(group)) => {
                let groups = if matches!(peer, Peer::Group(_)) { &self.groups } else { &self.supergroups };
                let unknown = Group::default();
                let group = groups.get(&group).unwrap_or(&unknown);
                string(&full["description"]).clone_into(&mut info.about);
                (info.handle, info.link) = named(&group.handle);
                if info.link.is_empty() {
                    string(&full["invite_link"]["invite_link"]).clone_into(&mut info.link);
                }
                let counted = count(&full["member_count"]);
                info.members = if counted > 0 { counted } else { group.members.clamp(0, i64::from(u32::MAX)) as u32 };
                // A channel is run by its admins alone; elsewhere the chat's permissions speak for the rest.
                let entry = self.chats.get(&id).filter(|_| !group.channel);
                info.can_invite = group.member && group.invites.unwrap_or(entry.is_some_and(|entry| entry.may_invite));
                info.can_edit = group.member && group.edits.unwrap_or(entry.is_some_and(|entry| entry.may_edit));
            }
        }
        info
    }

    /// What there is to know about the chat of a user (`id` positive), a
    /// basic group or a supergroup, when TDLib says their full information
    /// changed: none for a chat that was never told.
    pub fn info_of(&self, peer: Peer, full: &Value) -> Option<Info> {
        let id = match peer {
            Peer::User(user) => user,
            Peer::Group(group) => -group,
            Peer::Supergroup(group) => SUPERGROUPS - group,
        };
        self.chats.contains_key(&id).then(|| self.info(id, full))
    }

    /// The people of a `chatMembers`, each with what they are there. `now`
    /// is the time in Unix seconds, for saying how long ago someone was here.
    pub fn members(&self, members: &Value, now: i64) -> Vec<Member> {
        let member = |member: &Value| {
            let who = &member["member_id"];
            let role = match kind(&member["status"]) {
                "chatMemberStatusCreator" => Role::Owner,
                "chatMemberStatusAdministrator" => Role::Admin,
                _ => Role::Member,
            };
            // A chat that is in a chat (a channel speaking in its group) is no person to list.
            let person = (kind(who) == "messageSenderUser").then(|| self.person(int(&who["user_id"]), now)).flatten();
            person.map(|person| Member { role, ..person })
        };
        items(&members["members"]).filter_map(member).collect()
    }

    /// The people of a `users`.
    pub fn people(&self, users: &Value, now: i64) -> Vec<Member> {
        items(&users["user_ids"]).filter_map(|id| self.person(int(id), now)).collect()
    }

    /// The user `id` as someone to list.
    fn person(&self, id: SenderId, now: i64) -> Option<Member> {
        let user = self.users.get(&id)?;
        let status = if user.status.is_empty() && user.last_seen > 0 {
            seen(now.saturating_sub(user.last_seen))
        } else {
            user.status.clone()
        };
        Some(Member { id, name: user.name.clone(), photo: user.photo, status, role: Role::Member })
    }

    /// Who a `MessageSender` is: their id, name and picture.
    fn sender(&self, sender: &Value) -> (SenderId, String, Option<FileId>) {
        if kind(sender) == "messageSenderChat" {
            let id = int(&sender["chat_id"]);
            let entry = self.chats.get(&id);
            return (id, entry.map(|e| e.title.clone()).unwrap_or_default(), entry.and_then(|e| e.chat.photo));
        }
        let id = int(&sender["user_id"]);
        let user = self.users.get(&id);
        (id, user.map(|u| u.name.clone()).unwrap_or_default(), user.and_then(|u| u.photo))
    }

    /// A sender's first name (a chat's whole one).
    pub fn first_name(&self, sender: &Value) -> String {
        match self.users.get(&int(&sender["user_id"])) {
            Some(user) if kind(sender) == "messageSenderUser" => user.first.clone(),
            _ => self.sender(sender).1,
        }
    }

    /// The name of the user `id`.
    fn user_name(&self, id: i64) -> &str {
        self.users.get(&id).map_or("Someone", |user| user.name.as_str())
    }

    /// The title of the chat `id`, as Telegram gave it.
    fn chat_title(&self, id: ChatId) -> String {
        self.chats.get(&id).map_or_else(|| "a chat".to_owned(), |entry| entry.title.clone())
    }

    /// Forgets everything: the user signed out.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Which of the settings for a kind of chat a chat of this kind follows.
fn scope(kind: Kind) -> usize {
    match kind {
        Kind::Private => 0,
        Kind::Group => 1,
        Kind::Channel => 2,
    }
}

/// Whether a chat's messages ask for no attention: by its own settings, or
/// where they leave it open by its community's, then by those for its kind of chat.
fn muted(notify: &Value, community: Option<&Value>, scope: bool) -> bool {
    own_mute(notify).or_else(|| community.and_then(own_mute)).unwrap_or(scope)
}

/// What `chatNotificationSettings` say themselves about muting; None where they leave it to a default.
fn own_mute(settings: &Value) -> Option<bool> {
    (settings.is_object() && !flag(&settings["use_default_mute_for"])).then(|| int(&settings["mute_for"]) > 0)
}

/// The words of a `draftMessage`; empty for none, and for one that is not words.
fn draft(draft: &Value) -> String {
    string(&draft["content"]["text"]["text"]).to_owned()
}

/// The first of the active names in a `usernames`, without its sign.
fn handle(usernames: &Value) -> String {
    items(&usernames["active_usernames"]).map(string).find(|name| !name.is_empty()).unwrap_or_default().to_owned()
}

/// Writes what a `forumTopicInfo` says into a topic.
fn describe(topic: &mut Topic, info: &Value) {
    string(&info["name"]).clone_into(&mut topic.name);
    topic.color = (int(&info["icon"]["color"]) & 0xFF_FFFF) as u32;
    topic.closed = flag(&info["is_closed"]);
}

/// A `ChatList` as the model's list. A community is no list of TDLib's.
pub fn list(list: &Value) -> Option<List> {
    match kind(list) {
        "chatListMain" => Some(List::Main),
        "chatListArchive" => Some(List::Archive),
        "chatListFolder" => Some(List::Folder(int(&list["chat_folder_id"]))),
        _ => None,
    }
}

/// Puts `chat` where a `chatPosition` says it is. An order of zero takes
/// it out of that list.
fn place(chat: &mut Chat, position: &Value) {
    let Some(list) = list(&position["list"]) else { return };
    chat.places.retain(|place| place.list != list);
    let order = int(&position["order"]);
    if order != 0 {
        chat.places.push(Place { list, order, pinned: flag(&position["is_pinned"]) });
    }
}

/// A `UserStatus` as the line under a name, and when they were last here
/// where Telegram says exactly.
fn presence(status: &Value) -> (String, i64) {
    let line = match kind(status) {
        "userStatusOnline" => "online",
        "userStatusOffline" => return (String::new(), int(&status["was_online"])),
        "userStatusRecently" => "last seen recently",
        "userStatusLastWeek" => "last seen within a week",
        "userStatusLastMonth" => "last seen within a month",
        _ => "",
    };
    (line.to_owned(), 0)
}

/// How long ago someone was here, as the line under their name.
fn seen(seconds: i64) -> String {
    let ago = |many: i64, unit: &str| format!("last seen {many} {unit}{} ago", if many == 1 { "" } else { "s" });
    match seconds {
        ..60 => "last seen just now".to_owned(),
        60..3600 => ago(seconds / 60, "minute"),
        3600..86_400 => ago(seconds / 3600, "hour"),
        _ => ago(seconds / 86_400, "day"),
    }
}

/// A `file`'s id. One already on disk is told to the view at once.
fn file(file: &Value, out: &mut Vec<Event>) -> Option<FileId> {
    let id = int(&file["id"]);
    if id == 0 {
        return None;
    }
    out.extend(downloaded(file));
    Some(id)
}

/// The event for a `file` that is wholly on disk.
pub fn downloaded(file: &Value) -> Option<Event> {
    let local = &file["local"];
    let path = string(&local["path"]);
    (flag(&local["is_downloading_completed"]) && !path.is_empty())
        .then(|| Event::File { file: int(&file["id"]), path: path.to_owned() })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    pub fn cache() -> (Cache, Vec<Event>) {
        let mut cache = Cache { me: 1, ..Cache::default() };
        let mut out = Vec::new();
        cache.user(
            &json!({"id": 7, "first_name": "Mara", "last_name": "Lind", "status": {"@type": "userStatusOnline"},
            "usernames": {"active_usernames": ["mara"]}, "phone_number": "31612345678",
            "profile_photo": {"small": {"id": 31, "local": {"path": "/p/31.jpg", "is_downloading_completed": true}}}}),
            &mut out,
        );
        cache.user(&json!({"id": 8, "first_name": "Jonas", "last_name": "", "status": {"@type": "userStatusOffline", "was_online": 1700}}), &mut out);
        (cache, out)
    }

    pub fn text_message(id: i64, chat: i64, user: i64, text: &str) -> Value {
        json!({"id": id, "chat_id": chat, "sender_id": {"@type": "messageSenderUser", "user_id": user}, "date": 1000 + id,
            "content": {"@type": "messageText", "text": {"text": text, "entities": []}}})
    }

    fn position(list: &str, order: i64) -> Value {
        json!({"list": {"@type": list}, "order": order.to_string(), "is_pinned": false})
    }

    fn place_of(chat: &Chat, list: List) -> (i64, bool) {
        chat.place(list).map_or((0, false), |place| (place.order, place.pinned))
    }

    /// A supergroup and its chat, the user in it as `status` says.
    fn supergroup(cache: &mut Cache, id: i64, channel: bool, status: &str) -> Chat {
        cache.group(
            &json!({"id": id, "is_channel": channel, "is_forum": !channel, "status": {"@type": status},
            "usernames": {"active_usernames": []}}),
            true,
        );
        let about = json!({"id": SUPERGROUPS - id, "title": "Design", "permissions": {"can_send_basic_messages": true,
            "can_invite_users": true}, "type": {"@type": "chatTypeSupergroup", "supergroup_id": id, "is_channel": channel}});
        cache.chat(&about, &mut Vec::new()).expect("the chat")
    }

    #[test]
    fn chats_take_their_name_status_and_place_from_what_is_known() {
        let (mut cache, mut out) = cache();
        assert_eq!(out, [Event::File { file: 31, path: "/p/31.jpg".into() }], "a picture already on disk is told");
        let chat = cache
            .chat(
                &json!({"id": 7, "type": {"@type": "chatTypePrivate", "user_id": 7}, "title": "Mara Lind",
                "unread_count": 2, "permissions": {"can_send_basic_messages": true}, "is_marked_as_unread": true,
                "last_read_inbox_message_id": 38, "draft_message": {"content": {"@type": "draftMessageContentText",
                    "text": {"text": "see you at"}}},
                "positions": [{"list": {"@type": "chatListFolder", "chat_folder_id": 4}, "order": "5", "is_pinned": false},
                              {"list": {"@type": "chatListMain"}, "order": "9221294784512000005", "is_pinned": true}],
                "last_message": text_message(40, 7, 7, "see you\nthere")}),
                &mut out,
            )
            .expect("the chat");
        assert_eq!((chat.title.as_str(), chat.status.as_str(), chat.unread), ("Mara Lind", "online", 2));
        assert_eq!(place_of(&chat, List::Main), (9_221_294_784_512_000_005, true), "a 64-bit order");
        assert_eq!((place_of(&chat, List::Folder(4)), chat.place(List::Archive)), ((5, false), None));
        assert_eq!((chat.marked_unread, chat.read_inbox, chat.draft.as_str()), (true, 38, "see you at"));
        assert!(chat.can_send && chat.joined && !chat.admin && !chat.forum, "a person is no group to run");
        let last = chat.last.expect("a preview");
        assert_eq!((last.sender.as_str(), last.text.as_str(), last.date), ("", "see you there", 1040));
        // The person leaves: the chat with them says when.
        let chat = cache
            .user_status(&json!({"user_id": 7, "status": {"@type": "userStatusOffline", "was_online": 1234}}))
            .expect("the chat");
        assert_eq!((chat.status.as_str(), chat.last_seen), ("", 1234));
        // The chat with oneself has a name of its own.
        let saved =
            cache.chat(&json!({"id": 1, "type": {"@type": "chatTypePrivate", "user_id": 1}, "title": "Me"}), &mut out);
        assert_eq!(saved.expect("the chat").title, "Saved Messages");
        assert!(
            cache.chat_update(&json!({"@type": "updateChatTitle", "chat_id": 999, "title": "x"}), &mut out).is_none()
        );
        // A program is never online or away.
        cache.user(
            &json!({"id": 9, "first_name": "Helper", "type": {"@type": "userTypeBot"},
            "status": {"@type": "userStatusOnline"}}),
            &mut out,
        );
        let bot = cache.chat(&json!({"id": 9, "type": {"@type": "chatTypePrivate", "user_id": 9}}), &mut out);
        assert_eq!(bot.expect("the chat").status, "bot");
        assert!(cache.user_status(&json!({"user_id": 9, "status": {"@type": "userStatusOnline"}})).is_none());
    }

    #[test]
    fn a_chat_follows_what_is_told_of_it_one_field_at_a_time() {
        let (mut cache, mut out) = cache();
        cache.chat(&json!({"id": 7, "type": {"@type": "chatTypePrivate", "user_id": 7}}), &mut out);
        let mut tell = |update: Value| cache.chat_update(&update, &mut Vec::new()).expect("the chat");
        let chat = tell(
            json!({"@type": "updateChatReadInbox", "chat_id": 7, "last_read_inbox_message_id": 90, "unread_count": 3}),
        );
        assert_eq!((chat.read_inbox, chat.unread), (90, 3));
        assert!(
            tell(json!({"@type": "updateChatIsMarkedAsUnread", "chat_id": 7, "is_marked_as_unread": true})).waiting()
        );
        let drafted = json!({"@type": "updateChatDraftMessage", "chat_id": 7, "positions": [position("chatListMain", 12)],
            "draft_message": {"content": {"@type": "draftMessageContentText", "text": {"text": "hm"}}}});
        let chat = tell(drafted);
        assert_eq!((chat.draft.as_str(), place_of(&chat, List::Main)), ("hm", (12, false)));
        let chat =
            tell(json!({"@type": "updateChatDraftMessage", "chat_id": 7, "draft_message": null, "positions": []}));
        assert_eq!((chat.draft.as_str(), place_of(&chat, List::Main).0), ("", 12), "no position is no change of place");
        // Fields that are missing or of another type read as nothing.
        let chat = tell(json!({"@type": "updateChatReadInbox", "chat_id": 7, "unread_count": "many"}));
        assert_eq!((chat.read_inbox, chat.unread), (0, 0));
        let chat = tell(json!({"@type": "updateChatPosition", "chat_id": 7, "position": {"list": 5, "order": []}}));
        assert_eq!(chat.places.len(), 1);
    }

    #[test]
    fn groups_say_how_many_and_who_may_write() {
        let (mut cache, mut out) = cache();
        let id = -1_000_000_000_050_i64;
        let about = json!({"id": id, "type": {"@type": "chatTypeSupergroup", "supergroup_id": 50, "is_channel": true}, "title": "News"});
        let unknown = cache.chat(&about, &mut out).expect("the chat");
        assert!(!unknown.can_send && !unknown.joined, "an unknown channel is read only");
        let chat = cache
            .group(&json!({"id": 50, "member_count": 1200, "is_channel": true, "status": {"@type": "chatMemberStatusMember"}}), true)
            .expect("the chat");
        assert_eq!((chat.kind, chat.status.as_str(), chat.can_send), (Kind::Channel, "1200 subscribers", false));
        assert!(chat.joined && !chat.admin);
        let chat = cache
            .group(&json!({"id": 50, "member_count": 1, "is_channel": true, "status": {"@type": "chatMemberStatusCreator"}}), true)
            .expect("the chat");
        assert_eq!((chat.status.as_str(), chat.can_send, chat.admin), ("1 subscriber", true, true));
        let left =
            cache.group(&json!({"id": 50, "is_channel": true, "status": {"@type": "chatMemberStatusLeft"}}), true);
        assert!(left.is_some_and(|chat| !chat.joined && !chat.admin && !chat.can_send));
        // A group names who wrote its latest message, and drops out of the list at order zero.
        cache.group(&json!({"id": 60, "member_count": 3, "status": {"@type": "chatMemberStatusMember"}}), false);
        cache.chat(
            &json!({"id": -60, "type": {"@type": "chatTypeBasicGroup", "basic_group_id": 60}, "title": "Design",
            "permissions": {"can_send_basic_messages": true}}),
            &mut out,
        );
        let update = json!({"@type": "updateChatLastMessage", "chat_id": -60, "last_message": text_message(5, -60, 8, "done"),
            "positions": [{"list": {"@type": "chatListMain"}, "order": "77", "is_pinned": false}]});
        let chat = cache.chat_update(&update, &mut out).expect("the chat");
        assert_eq!((place_of(&chat, List::Main).0, chat.can_send, chat.status.as_str()), (77, true, "3 members"));
        assert_eq!(chat.last.map(|last| last.sender).as_deref(), Some("Jonas"));
        let gone = json!({"@type": "updateChatPosition", "chat_id": -60, "position": {"list": {"@type": "chatListMain"}, "order": "0"}});
        assert!(cache.chat_update(&gone, &mut out).expect("the chat").places.is_empty());
        // A forum is a supergroup that says so.
        let forum = supergroup(&mut cache, 70, false, "chatMemberStatusAdministrator");
        assert!(forum.forum && forum.admin && forum.joined);
        assert_eq!(cache.peer(forum.id), Peer::Supergroup(70));
        assert_eq!(
            (cache.peer(5), cache.peer(-5), cache.peer(SUPERGROUPS - 5)),
            (Peer::User(5), Peer::Group(5), Peer::Supergroup(5))
        );
    }

    #[test]
    fn a_chat_is_muted_by_its_own_settings_or_those_it_defers_to() {
        let (mut cache, mut out) = cache();
        let about = |notify: Value| json!({"id": 7, "type": {"@type": "chatTypePrivate", "user_id": 7}, "notification_settings": notify});
        let own = cache.chat(&about(json!({"use_default_mute_for": false, "mute_for": 600})), &mut out);
        assert!(own.expect("the chat").muted);
        // Muted for an hour by default says nothing when the default is not used, and the other way round.
        let default = cache.chat(&about(json!({"use_default_mute_for": true, "mute_for": 600})), &mut out);
        assert!(!default.expect("the chat").muted, "the default for people is not muted");
        out.clear();
        let people = json!({"@type": "notificationSettingsScopePrivateChats"});
        cache.scope(&people, &json!({"mute_for": 31_000_000}), &mut out);
        assert!(matches!(out.as_slice(), [Event::Chat(chat)] if chat.id == 7 && chat.muted), "{out:?}");
        out.clear();
        cache.scope(&people, &json!({"mute_for": 31_000_000}), &mut out);
        cache.scope(&json!({"@type": "notificationSettingsScopeGroupChats"}), &json!({"mute_for": 9}), &mut out);
        cache.scope(&json!({"@type": "somethingElse"}), &json!({"mute_for": 9}), &mut out);
        assert!(out.is_empty(), "nothing is told that did not change");
        let unmuted = cache.chat(&about(json!({"use_default_mute_for": false, "mute_for": 0})), &mut out);
        assert!(!unmuted.expect("the chat").muted, "its own word goes before the default");
        // Settings that are missing or not settings leave it to the default.
        assert!(cache.chat(&about(json!("loud")), &mut out).expect("the chat").muted);
        let settings = cache.notify(7);
        assert_eq!(
            (settings["use_default_mute_for"].as_bool(), settings["use_default_sound"].as_bool()),
            (Some(true), Some(true))
        );
        // A community's settings stand between its chats and those for their kind.
        let group = supergroup(&mut cache, 70, false, "chatMemberStatusMember");
        cache.community(&json!({"id": 3, "name": "Studio", "have_access": true, "status": {"@type": "communityMemberStatusMember"}}), &mut out);
        out.clear();
        assert!(group.muted, "groups are muted unless they say otherwise");
        let full = json!({"chats": [{"chat_id": group.id}], "notification_settings": {"use_default_mute_for": false, "mute_for": 0}});
        cache.community_info(3, &full, &mut out);
        assert!(matches!(out.as_slice(), [Event::Chat(chat)] if chat.id == group.id && !chat.muted), "{out:?}");
    }

    #[test]
    fn the_lists_are_the_folders_the_communities_and_the_archive() {
        let (mut cache, mut out) = cache();
        assert_eq!(cache.lists(), None, "nothing is told before anything is known");
        cache.signed_in();
        let names = |event: Option<Event>| match event {
            Some(Event::Lists(lists)) => {
                lists.into_iter().map(|list| format!("{}:{}", list.name, list.unread)).collect()
            }
            _ => Vec::new(),
        };
        assert_eq!(names(cache.lists()), ["All:0"]);
        cache.folders(&json!({"main_chat_list_position": 1, "chat_folders": [
            {"id": 4, "name": {"text": {"text": "Work"}}}, {"id": 9, "name": {"text": {"text": "Family"}}}]}));
        cache
            .unread(&json!({"chat_list": {"@type": "chatListFolder", "chat_folder_id": 4}, "unread_unmuted_count": 2}));
        cache.unread(&json!({"chat_list": {"@type": "chatListMain"}, "unread_unmuted_count": 5, "total_count": 80}));
        cache.unread(&json!({"chat_list": {"@type": "chatListArchive"}, "unread_unmuted_count": 0, "total_count": 0}));
        assert_eq!(names(cache.lists()), ["Work:2", "All:5", "Family:0"]);
        assert_eq!(cache.lists(), None, "told once");
        cache.unread(&json!({"chat_list": {"@type": "chatListMain"}, "unread_unmuted_count": 5}));
        assert_eq!(cache.lists(), None, "the same again is not told");
        // The archive is a list while a chat is in it, or Telegram says it has some.
        let archived = |order: i64| json!({"@type": "updateChatPosition", "chat_id": 7, "position": position("chatListArchive", order)});
        cache.chat(&json!({"id": 7, "type": {"@type": "chatTypePrivate", "user_id": 7}}), &mut out);
        cache.chat_update(&archived(6), &mut out);
        assert_eq!(names(cache.lists()), ["Work:2", "All:5", "Family:0", "Archive:0"]);
        cache.chat_update(&archived(0), &mut out);
        assert_eq!(names(cache.lists()).len(), 3);
        cache.unread(&json!({"chat_list": {"@type": "chatListArchive"}, "unread_unmuted_count": 1, "total_count": 4}));
        assert_eq!(names(cache.lists()).last().map(String::as_str), Some("Archive:1"));
        // A main list said to be beyond the folders goes last among them; a list of no known kind is ignored.
        cache.folders(&json!({"main_chat_list_position": 40, "chat_folders": [{"id": 4, "name": "Work"}]}));
        cache.unread(&json!({"chat_list": {"@type": "chatListNew"}, "unread_unmuted_count": 1}));
        assert_eq!(names(cache.lists()), [":2", "All:5", "Archive:1"], "a name that is not one reads as empty");
    }

    #[test]
    fn a_community_lists_its_chats_wherever_else_they_are() {
        let (mut cache, mut out) = cache();
        let community =
            |status: &str| json!({"id": 3, "name": "Studio", "have_access": true, "status": {"@type": status}});
        let listed = supergroup(&mut cache, 70, false, "chatMemberStatusMember");
        let quiet = supergroup(&mut cache, 71, true, "chatMemberStatusLeft");
        cache.chat_update(
            &json!({"@type": "updateChatPosition", "chat_id": listed.id, "position": position("chatListMain", 900)}),
            &mut out,
        );
        cache.chat_update(&json!({"@type": "updateChatLastMessage", "chat_id": quiet.id, "last_message": text_message(5, quiet.id, 8, "x"), "positions": []}), &mut out);
        assert!(cache.community(&community("communityMemberStatusMember"), &mut out), "its chats are asked for");
        assert!(!cache.community(&community("communityMemberStatusMember"), &mut out), "once");
        cache.signed_in();
        assert!(matches!(cache.lists(), Some(Event::Lists(lists)) if lists.len() == 1), "no list before it has chats");
        out.clear();
        let full =
            json!({"chats": [{"chat_id": listed.id}, {"chat_id": quiet.id}, {"chat_id": -99}, {"chat_id": "x"}]});
        cache.community_info(3, &full, &mut out);
        let orders: Vec<(i64, i64)> = out
            .iter()
            .filter_map(|event| match event {
                Event::Chat(chat) => Some((chat.id, place_of(chat, List::Community(3)).0)),
                _ => None,
            })
            .collect();
        // By its place in the main list; a chat that is in no list by its latest message.
        assert_eq!(orders, [(quiet.id, 1005), (listed.id, 900)], "an unknown chat is not told");
        let Some(Event::Lists(lists)) = cache.lists() else { panic!("the lists") };
        assert_eq!(lists.last().map(|list| (list.list, list.name.as_str())), Some((List::Community(3), "Studio")));
        // A chat in no list and without a message is still listed.
        cache.chat_update(
            &json!({"@type": "updateChatLastMessage", "chat_id": quiet.id, "last_message": null}),
            &mut out,
        );
        out.clear();
        cache.community_info(3, &json!({"chats": [{"chat_id": quiet.id}]}), &mut out);
        let places: Vec<(i64, usize)> = out
            .iter()
            .filter_map(|event| match event {
                Event::Chat(chat) => Some((place_of(chat, List::Community(3)).0, chat.places.len())),
                _ => None,
            })
            .collect();
        assert_eq!(places, [(1, 1), (0, 1)], "the chat it no longer has is told too");
        // Left, or one that never loads: no list, and nothing breaks.
        out.clear();
        assert!(!cache.community(&community("communityMemberStatusLeft"), &mut out));
        assert!(matches!(out.as_slice(), [Event::Chat(chat)] if chat.places.is_empty()));
        assert!(matches!(cache.lists(), Some(Event::Lists(lists)) if lists.len() == 1));
        cache.community_info(8, &json!({"chats": 4}), &mut out);
        assert!(!cache.community(&json!({"id": 0}), &mut out) && !cache.community(&json!("x"), &mut out));
        assert_eq!((cache.community_name(3), cache.community_name(8)), ("Studio", "a community"));
    }

    #[test]
    fn topics_are_remembered_so_that_parts_of_them_can_change() {
        let (mut cache, mut out) = cache();
        let forum = supergroup(&mut cache, 70, false, "chatMemberStatusMember");
        let info = |name: &str, closed: bool| {
            json!({"chat_id": forum.id, "forum_topic_id": 5, "name": name,
            "icon": {"color": 0x6F_B9F0}, "is_closed": closed})
        };
        assert!(cache.topic_info(&info("Roof", false)).is_none() && !cache.follows_topics(forum.id));
        let topic = cache
            .topic(
                &json!({"info": info("Roof", false), "order": "77", "is_pinned": true, "unread_count": 2,
                "unread_mention_count": 1, "last_read_inbox_message_id": 40 << 20, "last_read_outbox_message_id": 9,
                "last_message": text_message(40 << 20, forum.id, 8, "tiles"),
                "notification_settings": {"use_default_mute_for": false, "mute_for": 5}}),
                &mut out,
            )
            .expect("the topic");
        assert_eq!((topic.chat, topic.id, topic.name.as_str(), topic.color), (forum.id, 5, "Roof", 0x6F_B9F0));
        assert_eq!((topic.order, topic.pinned, topic.unread, topic.mentioned, topic.muted), (77, true, 2, true, true));
        assert_eq!(topic.last.map(|last| (last.sender, last.text)), Some(("Jonas".into(), "tiles".into())));
        assert!(cache.follows_topics(forum.id));
        assert!(
            cache.topic_info(&info("Roof and gutters", true)).is_some_and(|topic| topic.closed && topic.unread == 2)
        );
        // Part of it: what it leaves to the chat's settings follows the chat.
        let update = json!({"chat_id": forum.id, "forum_topic_id": 5, "is_pinned": false, "last_read_inbox_message_id": 40 << 20,
            "notification_settings": {"use_default_mute_for": true}});
        let (topic, moved) = cache.topic_update(&update).expect("the topic");
        assert!(!topic.pinned && !topic.muted && !topic.mentioned && !moved);
        assert_eq!(topic.name, "Roof and gutters");
        // A new message is its latest, moves it up and is unread when it is not the user's own.
        let json = json!({"id": 41 << 20, "chat_id": forum.id, "date": 5000, "sender_id": {"@type": "messageSenderUser", "user_id": 7},
            "topic_id": {"@type": "messageTopicForum", "forum_topic_id": 5}, "contains_unread_mention": true,
            "content": {"@type": "messageText", "text": {"text": "slates"}}});
        let message = cache.message(&json, &mut out);
        let topic = cache.topic_message(&json, &message).expect("the topic");
        assert_eq!((topic.order, topic.unread, topic.mentioned), ((5000 << 32) + 41, 3, true));
        assert_eq!(topic.last.map(|last| last.text).as_deref(), Some("slates"));
        let (_, moved) = cache
            .topic_update(&json!({"chat_id": forum.id, "forum_topic_id": 5, "last_read_inbox_message_id": 41 << 20}))
            .expect("the topic");
        assert!(moved, "what is unread now is to be asked for");
        // Topics nobody asked about, and what is no topic, are passed over.
        assert!(cache.topic_update(&json!({"chat_id": forum.id, "forum_topic_id": 6})).is_none());
        assert!(cache.topic_message(&json, &Message { topic: 6, ..message.clone() }).is_none());
        assert!(cache.topic_message(&json, &Message { topic: -5, ..message }).is_none());
        assert!(
            cache.topic(&json!({"info": {"chat_id": forum.id}}), &mut out).is_none()
                && cache.topic(&json!(7), &mut out).is_none()
        );
    }

    #[test]
    fn what_there_is_to_know_about_a_chat_comes_from_its_full_information() {
        let (mut cache, mut out) = cache();
        cache.chat(&json!({"id": 7, "type": {"@type": "chatTypePrivate", "user_id": 7}}), &mut out);
        let person = cache.info(7, &json!({"bio": {"text": "Roofs."}}));
        assert_eq!(
            (person.about.as_str(), person.handle.as_str(), person.phone.as_str()),
            ("Roofs.", "@mara", "+31612345678")
        );
        assert_eq!(
            (person.link.as_str(), person.members, person.can_invite, person.can_edit),
            ("https://t.me/mara", 0, false, false)
        );
        assert_eq!(
            cache.info(8, &json!(null)),
            Info { chat: 8, ..Info::default() },
            "someone without a name or a number"
        );
        // A member of a private group: its invitation, and what the chat's permissions allow.
        let group = supergroup(&mut cache, 70, false, "chatMemberStatusMember");
        let full =
            json!({"description": "Plans", "member_count": 12, "invite_link": {"invite_link": "https://t.me/+abc"}});
        let info = cache.info(group.id, &full);
        assert_eq!((info.about.as_str(), info.link.as_str(), info.members), ("Plans", "https://t.me/+abc", 12));
        assert!(info.handle.is_empty() && info.can_invite && !info.can_edit);
        // An admin's rights go before the permissions; a public chat is found by its name.
        let rights = json!({"@type": "chatMemberStatusAdministrator", "rights": {"can_change_info": true}});
        cache.group(&json!({"id": 70, "status": rights, "member_count": 30, "usernames": {"active_usernames": ["", "roofers"]}}), true);
        let info = cache.info(group.id, &json!({"invite_link": {"invite_link": "https://t.me/+abc"}}));
        assert_eq!((info.handle.as_str(), info.link.as_str(), info.members), ("@roofers", "https://t.me/roofers", 30));
        assert!(!info.can_invite && info.can_edit);
        // Someone who reads a channel changes nothing about it, whatever its permissions say.
        let channel = supergroup(&mut cache, 71, true, "chatMemberStatusMember");
        let info = cache.info(channel.id, &json!({}));
        assert!(!info.can_invite && !info.can_edit && info.link.is_empty());
        let restricted = json!({"@type": "chatMemberStatusRestricted", "is_member": true, "permissions": {"can_invite_users": false}});
        cache.group(&json!({"id": 70, "status": restricted}), true);
        assert!(!cache.info(group.id, &json!({})).can_invite);
    }

    #[test]
    fn people_are_listed_with_what_they_are_and_when_they_were_here() {
        let (cache, _) = cache();
        let members = json!({"members": [
            {"member_id": {"@type": "messageSenderUser", "user_id": 7}, "status": {"@type": "chatMemberStatusCreator"}},
            {"member_id": {"@type": "messageSenderUser", "user_id": 8}, "status": {"@type": "chatMemberStatusAdministrator"}},
            {"member_id": {"@type": "messageSenderChat", "chat_id": -5}, "status": {"@type": "chatMemberStatusMember"}},
            {"member_id": {"@type": "messageSenderUser", "user_id": 404}}, {"member_id": 3}]});
        let members = cache.members(&members, 1700 + 7200);
        let told: Vec<_> = members
            .iter()
            .map(|member| (member.id, member.name.as_str(), member.status.as_str(), member.role))
            .collect();
        assert_eq!(told, [(7, "Mara Lind", "online", Role::Owner), (8, "Jonas", "last seen 2 hours ago", Role::Admin)]);
        assert_eq!(members[0].photo, Some(31));
        let people = cache.people(&json!({"user_ids": [8, "7", null]}), 1700);
        assert_eq!(
            people.iter().map(|person| (person.id, person.role)).collect::<Vec<_>>(),
            [(8, Role::Member), (7, Role::Member)]
        );
        assert!(cache.people(&json!({"user_ids": 8}), 0).is_empty() && cache.members(&json!(null), 0).is_empty());
        assert_eq!(
            (seen(-5), seen(59), seen(60), seen(3600)),
            (
                "last seen just now".into(),
                "last seen just now".into(),
                "last seen 1 minute ago".into(),
                "last seen 1 hour ago".into()
            )
        );
        assert_eq!(seen(86_400 * 3), "last seen 3 days ago");
    }

    #[test]
    fn numbers_come_as_numbers_or_as_strings() {
        assert_eq!(
            (int(&json!("9221294784512000005")), int(&json!(12)), int(&json!("x")), int(&json!(1.5))),
            (9_221_294_784_512_000_005, 12, 0, 0)
        );
        assert_eq!((count(&json!(-4)), count(&json!("7")), count(&json!(99_999_999_999_i64))), (0, 7, u32::MAX));
        assert_eq!(items(&json!({"a": 1})).count() + items(&json!([1, 2])).count(), 2);
        assert_eq!(list(&json!({"@type": "chatListFolder", "chat_folder_id": 3})), Some(List::Folder(3)));
    }
}
