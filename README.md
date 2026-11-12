<p align="center"><img src="branding/adelite-500.png" width="112" alt="Adelite"></p>

# Adelite

A native desktop app for the day's work. One window, a rail of apps along its right edge. The first app is
chat, a Telegram client built on Telegram's own library (TDLib). Mail and documents are meant to follow as
apps of their own on the same rail.

No web view and no UI framework: winit provides the window, wgpu draws every pixel through a single
instanced pipeline, and the interface is redrawn only when something changes. An idle window uses no CPU.
It is written for Windows, macOS and Linux.

**Status:** early and moving fast. Windows is where it is used and tried; macOS and Linux are built and
tested by CI but have barely been run.

## Installing and updating

Each release on the [releases page](https://github.com/OnlyF0uR/adelite/releases) has a download per
system, built by CI ([Releasing](#releasing)):

- **Windows:** `Adelite-<version>-windows-x64-setup.exe` installs for the current user into
  `%LOCALAPPDATA%\Programs\Adelite`, without asking for administrator rights, with a Start menu entry and an
  uninstaller. It is not code-signed, so SmartScreen warns the first time: "More info", then "Run anyway".
- **macOS** (Apple silicon): open `Adelite-<version>-macos-arm64.dmg` and drag Adelite into Applications.
  It is not signed by an Apple developer, so macOS refuses it at first: try to open it once, then allow it
  in System Settings, Privacy & Security, "Open Anyway".
- **Linux** (x86-64): unpack `Adelite-<version>-linux-x64.tar.gz` where you keep programs and run
  `adelite/adelite`. `adelite.desktop` and `adelite.png` beside it are for a menu entry.

Uninstalling leaves your data in `~/.adelite/` ([Data](#data)).

**Updates.** Settings, "Check for updates" asks GitHub for the newest published release. Nothing is
checked or downloaded unless it is pressed. When a newer version is out, "Install and restart" downloads
it, checks its signature, quits, puts it in place and starts it again. On Windows it runs the installer
silently. On macOS and Linux it replaces the app's files where they are, so that folder must be writable
(on macOS the app must have been moved into Applications). "What's new" opens the release's page.

Every download is signed (Ed25519) by the release workflow with a key only it holds. The app carries the
public half (`crates/update/src/signature.rs`) and refuses a download that does not match, so a release
someone else put up, or a file changed on the way, is never run. Downloads go through the system's `curl`
(part of Windows 10 and later, macOS and practically every Linux), so the app has no network code of its
own for this.

## Getting started

You need Rust (rustup fetches the version `rust-toolchain.toml` pins by itself) and TDLib.

1. **TDLib.** On Windows, `pwsh packaging/tdlib/build-windows.ps1` builds it into `vendor/tdlib/`; on
   macOS and Linux, `bash packaging/tdlib/build-unix.sh` does. See [TDLib](#tdlib).
2. **The API id and hash.** Copy `.env.example` to `.env` and fill in the app's pair from
   <https://my.telegram.org/apps>. Git ignores `.env`. Without a pair the build still works and asks for
   one on its first start. See [The API id and hash](#the-api-id-and-hash).
3. **Run it** and sign in with your phone number:

   ```sh
   cargo run --release -p adelite
   ```

Before a change goes in, these pass (CI runs them on all three systems):

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

The rules the code keeps to (modularity, dependencies, unsafe, design) are in [`CLAUDE.md`](CLAUDE.md).

## Chat

What works today:

- Signing in: phone number, code (the prompt says where Telegram sent it, and can ask for it another way),
  two-step password, login email. Signing out (in the menu beside the search, asked twice).
- The chats, in Telegram's order, in every list they are kept in: a strip over them holds the main list,
  your folders, your communities and the archive. Pinned chats, unread and mention counts, chats marked
  unread, muted chats, the latest message with who wrote it and whether yours was read, who is typing, and
  words you began and did not send.
- A chat's menu (the other mouse button, or the dots in its header): mark read or unread, mute, pin,
  archive, its info, leave or delete (asked twice).
- Search (Ctrl/Cmd+K): your chats by name as you type, then people you know, public chats and messages
  everywhere. Ctrl/Cmd+F, or the magnifier in a chat's header, looks through that chat only. A click on a
  found message opens its chat at that message.
- Conversations: text with bold, italic, code, strike-through, underline, quotations, hidden words (a
  click shows them), links, mentions and hashtags (a click follows them: a t.me link opens inside the
  app); photos, alone or as an album, opened in a viewer (there at once as the bubble showed it, sharp when
  the whole picture has come); videos, GIFs and round videos as a still that
  plays in the system's player; voice messages and music, likewise; stickers as their picture; files;
  polls you can answer; the page a link leads to; reactions; buttons under a bot's message; a channel
  post's views and its comments; replies (click one to jump to the message, loaded or not); forwarded
  messages; service messages; days between messages; who wrote what in groups, with faces; sent, read,
  sending and failed marks on your own.
- A chat opens where you left off: at the first unread message, under a line that says so. The message
  pinned in it shows under its name. A button takes you to its end, another to where you were mentioned.
- A message's menu (the other mouse button, or the dots beside it): answer it with an emoji, reply, copy,
  forward (to a chat picked on a card), pin, edit, select, delete (at once, no question; for everyone or only
  for you where Telegram has both). Selected messages are forwarded, copied or deleted together.
- The words of messages are selected like text anywhere: drag over them, across as many messages as you
  like (the view scrolls when you drag past its edge); double-click for a word, triple-click for a whole
  message, Shift+click to extend. Ctrl+C, or Copy selection in the message's menu, copies it, a blank
  line between messages. Escape or a click elsewhere in the conversation lets it go.
- Forums: a forum's topics take the place of the chat list while it is open; each topic is a conversation
  of its own, and what you write goes into it.
- A chat's card (click its name, or Info in its menu): who or what it is; its bell, a search through
  it, its pin, the archive and the way out as a row of tiles; what it says about itself and how it is
  reached (a click copies); and under tabs its people (searchable) and the pictures, files and links
  that were sent in it, each a click from where it was said. Whoever runs the chat changes its name and
  description on a form with Save and Cancel, makes an invite link, adds people, and beside a person's
  name makes them an admin or removes them (asked twice). The app's own menu starts a new group.
- Writing: Enter sends, Shift+Enter breaks the line, `**bold**` and `` `code` `` are read as Markdown.
  Up in an empty box edits your last message. Attach files with the paperclip, by dropping them on the
  window, or by pasting copied files or a copied picture; several go as an album. What you typed stays
  with its chat when you leave it, and is kept as a draft on your other devices.
- Live: new messages, edits, deletions, reactions, read receipts, who is online or was last seen. Others
  see when you are typing.
- Notifications: a message that arrives while the window is in the background is said by the system
  (and flashes the taskbar), unless its chat is muted and it does not mention you. On Windows a click on
  the notification brings the window forward. The settings switch them off.
- Sounds: a soft one when such a message arrives (not for the conversation you are looking at), another
  when a message of yours has left. Both are made in the app, not played from files; the settings switch
  them off.
- Closing the window keeps Adelite running, so messages keep arriving: on Windows the window goes and an
  icon in the tray brings it back (a click opens it, its menu opens or quits; starting Adelite again
  shows the running one), on macOS the app hides behind its Dock icon, on Linux, which has no tray
  without more, the window minimises. The settings choose between that, minimising and quitting.
  Ctrl+Q always quits.
- Older messages load as you scroll up, newer ones as you scroll down after a jump far back; a
  conversation keeps its place when they arrive.

Not there yet: playing video and sound inside the app (they open in the system's player), animated
stickers (their still is shown), custom emoji, calls and video chats, secret chats, stories, bots'
keyboards and inline bots, joining through an invite link (the app says what the link leads to), making
polls, scheduled messages, checklists, managing the account (profile, privacy, sessions), fine-grained
admin rights, right-to-left scripts, and the inline text of an input
method while composing (the candidate window still shows it).

### TDLib

Adelite loads TDLib's JSON interface (`tdjson`) when it starts. It looks, in order: where the
`ADELITE_TDJSON` environment variable points, beside the executable, in `tdlib/` beside it, in the app
bundle's `Contents/Frameworks/` on macOS, in `vendor/tdlib/` of a source checkout, in
`~/.adelite/telegram/`, and wherever the system finds libraries.
Without it the app still starts and says what is missing.

Build it on Windows (needs git and Visual Studio 2022 with the C++ workload; about half an hour):

```powershell
pwsh packaging/tdlib/build-windows.ps1
```

This builds the TDLib commit pinned in `packaging/tdlib/COMMIT` from Telegram's source into
`vendor/tdlib/` (`tdjson.dll` and the OpenSSL and zlib DLLs it needs). On macOS and Linux,
`bash packaging/tdlib/build-unix.sh` builds the same commit into `vendor/tdlib/libtdjson.dylib` or
`libtdjson.so`, with OpenSSL linked in, so the library needs nothing that is not on every system
(it needs git, CMake, a C++ compiler, gperf, and OpenSSL's and zlib's headers; the script names them).
The release workflow builds it the same way and packs it with the app.

### The API id and hash

Telegram knows an app by an API id and hash. They belong to Adelite, not to whoever uses it: create one
pair for the app at <https://my.telegram.org/apps> and give it to the build, which carries it from then on.
People who use that build are only asked for their phone number.

Copy `.env.example` to `.env` at the workspace's root, fill in the pair, and build:

```sh
ADELITE_TELEGRAM_API_ID=1234567
ADELITE_TELEGRAM_API_HASH=0123456789abcdef0123456789abcdef
```

Git ignores `.env`: the hash never goes into the repository. In CI the same two names are secrets of the
repository's `release` environment ([Releasing](#releasing)), where they win over the file.

The build seals the pair into the binary (`crates/telegram/build.rs`, `src/seal.rs`): XORed with a keystream
from seeds drawn anew for every build, in a block of fixed size, opened only when TDLib asks. Neither the
hash nor the id is in the file as text or as anything shaped like one, and release builds carry no symbols.
That keeps them from a search of the file. It does not keep them from someone who runs the app under a
debugger, since TDLib must be handed them in the clear; nothing on the user's machine can. The releases are
public, so take the pair as known, as every open-source Telegram client does: whoever has it can pass for
the app and use up its quota, or get the id blocked. Then the app says "Telegram did not accept this app",
and the way out is a new pair (from another phone number: Telegram gives each number one) and a release.

A build made without them still works, for trying the code: it asks for a pair on its first start and
keeps it in `~/.adelite/telegram/app.json`.

## Releasing

1. Set `version` under `[workspace.package]` in `Cargo.toml` to the new version, and commit.
2. Tag that commit `v<version>` and push the tag: `git tag v0.2.0 && git push origin v0.2.0`.
3. `.github/workflows/release.yml` checks that the tag and the version agree. It builds TDLib for each
   system (kept in CI's cache once built for the pinned commit), the app with the API pair, and the
   downloads (`packaging/`), then signs them and leaves them in a draft release.
4. Try the draft's downloads, then publish it. Only a published release is offered by "Check for updates".

The workflow runs in the repository's `release` environment, which only `v*` tags may use. It holds three
secrets: `ADELITE_TELEGRAM_API_ID`, `ADELITE_TELEGRAM_API_HASH`, and `ADELITE_UPDATE_KEY`, the private half
of the update-signing key (32 bytes in hex). Keep a copy of that key somewhere safe. Without it no update
can be signed that installed copies accept, and everyone would have to reinstall by hand. The signing tool
refuses any other key: `ADELITE_UPDATE_KEY=<key> cargo run -p adelite-update --bin adelite-sign -- --public`
prints the public half of the key it was given.

## Layout

| Crate | What it is |
|---|---|
| `crates/ui` (`adelite-ui`) | The toolkit: the wgpu renderer and its shader, text layout and fonts, the image atlas, the painter, widgets, the theme, platform helpers, and `Module`, what an app on the rail implements. |
| `crates/chat` (`adelite-chat`) | What a chat is, whatever network carries it: `Chat`, `Message`, `Command`, `Event`, and the `Backend` trait. No dependencies. |
| `crates/telegram` (`adelite-telegram`) | Telegram as a `Backend`: loads `tdjson`, speaks its JSON, maps it to the chat model. No interface code. |
| `crates/chat-view` (`adelite-chat-view`) | The chat app as a `Module`: the list, the conversation, the message box, the sign-in card. Knows `Backend`, not Telegram. |
| `crates/update` (`adelite-update`) | Updates: the newest release on GitHub, its download and signature check, and putting it in place (the installer on Windows, swapping the files on macOS and Linux). No interface code. Also `adelite-sign`, which signs the release files. |
| `crates/desktop` (`adelite`) | The shell: the window and event loop, the title bar, the rail, settings. `src/modules.rs` is the one place that names the apps. |

Dependencies point one way: `desktop` → `chat-view` → `ui` and `chat`; `telegram` → `chat`; `desktop` →
`update`.

### Adding or replacing an app

An app is a crate that implements `adelite_ui::Module` (an icon, a place to draw, the keyboard while it is
shown) and a line in `crates/desktop/src/modules.rs`. It does its work on threads of its own, keeps their
results in a channel, and calls the `Waker` it was given; the shell then calls its `poll` and draws.

Replacing Telegram with another chat network (or one of our own) is a crate that implements
`adelite_chat::Backend` and is handed to `ChatModule::new` in `modules.rs` instead of `Telegram`. The chat
view does not change: signing in is described by the backend as a `Prompt` (a title, fields, a button),
which the view draws whatever it asks for.

### Threads

No async runtime. The winit thread owns all state and draws. TDLib runs its own threads; one thread of
ours (`adelite-telegram`) waits in `td_receive`, maps what arrives, and wakes the interface once per batch.
Commands go to TDLib straight from the interface's thread (`td_send` never blocks). Thumbnails are decoded
on a worker, file dialogs run on a thread each, and so do an update's check and download (`adelite-update`).

## Data

Everything is under `~/.adelite/`:

- `config.json`: the colour scheme, the zoom, the window's size, whether notifications are shown and
  sounds are heard, and what closing the window does.
- `desktop.log`, `desktop-crash.log`: problems, and a report if the app crashes.
- `desktop.lock`: keeps a second copy from starting. A copy started by an update waits for it.
- `update/`: an update's download while it is fetched and checked; removed at the next start. On macOS and
  Linux an update is unpacked beside the installed app (`.adelite-update/`), and what it replaced waits
  there as `.<name>.adelite-old` until the next start.
- `telegram/app.json`: an API id and hash, only in a build made without its own. `telegram/tdlib/`: TDLib's database, which holds the session
  (it is not encrypted: whoever can read this folder can read the account). `telegram/files/`: downloads.

## Looking at the design without an account

The screens are drawn offscreen with made-up conversations and saved as PNGs, one per colour scheme:

```sh
ADELITE_SNAPSHOTS=<folder> cargo test -p adelite-chat-view snapshots -- --ignored   # the chat app's screens
ADELITE_SNAPSHOTS=<folder> cargo test -p adelite snapshots -- --ignored             # the whole window
```

The app icon and the logo in `branding/` are drawn from the mark in `crates/ui/src/logo.rs`:

```sh
ADELITE_ICON=<workspace>/crates/desktop/assets/icon cargo test -p adelite-ui --features offscreen icon -- --ignored
```

With TDLib built, this checks that it loads and answers (no account needed):

```sh
cargo test -p adelite-telegram tdlib -- --ignored
```

## Credits

Fonts: Inter, JetBrains Mono, Lucide and Twemoji, under the licenses beside them in `crates/ui/assets/`.
TDLib is Telegram's, under the Boost Software License.

## License

MIT or Apache-2.0, at your option (`LICENSE-MIT`, `LICENSE-APACHE`).
