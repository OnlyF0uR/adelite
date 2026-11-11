# CLAUDE.md

Adelite is a native desktop app for the day's work: a shell with a rail of apps along its right edge. Chat
(Telegram, through TDLib) is the first; mail and documents (Microsoft Graph) are meant to follow. It is a
Cargo workspace: `crates/ui` (the toolkit and the `Module` trait), `crates/chat` (the chat model and the
`Backend` trait, no dependencies), `crates/telegram` (TDLib behind `Backend`, no interface code),
`crates/chat-view` (the chat app, a `Module` over any `Backend`) and `crates/desktop` (the shell; package
and binary `adelite`). `README.md` describes every feature, the crate layout and where data is kept: read
it before changing behaviour, and keep it current when you do.

```sh
cargo run --release -p adelite
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

## Hard requirements

- **Modular.** The shell knows apps only as `adelite_ui::Module`, and `crates/desktop/src/modules.rs` is
  the one place that names them. The chat view knows a network only as `adelite_chat::Backend`. Nothing
  Telegram-specific goes into `chat` or `chat-view` (signing in is a `Prompt` the backend describes), and
  nothing interface-specific into `chat` or `telegram`. A new app is a new crate; never a branch in the shell.
- **Rust-native, custom UI.** `winit` + `wgpu`, and we draw the interface ourselves. No UI frameworks
  (egui, iced, Tauri, …).
- **Beautiful, elegant and fast.** Redraw only when something changes, do minimal work per frame, and keep
  the single-pipeline, single-draw-call renderer (`gpu.rs` + `shader.wgsl`). New visual effects go into
  that pipeline as new quad kinds, never as extra passes. A list lays out only what is on screen and keeps
  the heights of the rest (`chat-view/src/thread.rs`); nothing allocates per item per frame that can be kept.
- **Cross-platform:** Windows, macOS and Linux.
- **Dependencies are costly.** Add one only if it is truly required and can't reasonably be written by
  hand, with a `# why:` comment. No `thiserror` (write error types by hand: `Display` +
  `std::error::Error`), no `anyhow`, no async runtimes (threads and blocking I/O; no tokio), no convenience
  crates. JSON is read as `serde_json::Value`, without derive.
- **Unsafe** is denied workspace-wide. Three files opt in with `#![expect(unsafe_code)]`: the C interface
  of TDLib (`telegram/src/tdjson.rs`), the few system calls in `ui/src/platform.rs` (with its submodules
  `platform/tray.rs`, the Windows tray icon, and `platform/sound.rs`), and bytemuck's marker impls in
  `ui/src/gpu.rs`. One operation per block, each with a `// SAFETY:` comment.
- **Code quality:** every public item has doc comments, `clippy::pedantic` stays clean, no `unwrap`s on
  fallible runtime paths, non-trivial logic gets a small unit test. Whatever reads TDLib's JSON survives a
  missing or mistyped field (`telegram/src/map.rs`): defaults, never a panic.
- **TDLib:** check field and function names against `vendor/td/td/generate/scheme/td_api.tl` of the pinned
  commit (`packaging/tdlib/build-windows.ps1`), not from memory: the schema changes between versions.
  64-bit numbers arrive as strings (`map::int`); text offsets count UTF-16 units. `td_receive` is called
  from one thread only, which `tdjson::Inbox` enforces.
- **The API id and hash are the app's, not the user's:** the build takes them from `.env` (git ignores it)
  or the environment and seals them into the binary (`telegram/build.rs`, `telegram/src/seal.rs`); a user is
  never asked for them. The prompt for a pair exists only for a build made without. Never commit the hash,
  never let it reach the binary as text (no `env!`/`option_env!` of it, no `cargo:rustc-env`), and never
  write it to disk or a log when the app runs.
- **Secrets:** never log the API hash, a code or a password. Files holding them are written atomically,
  `0600` on Unix.
- **Safety:** links from messages only open `http(s)` and `mailto`. A received file is shown in its
  folder, never run.
- **Testing:** the owner tests the GUI and signs in. Don't drive the app with computer use and never ask
  for or use a Telegram account; `cargo build`, `clippy` and `test` are enough. To look at a visual change,
  render the screens offscreen (the README's commands): `chat-view/src/demo.rs` holds the made-up
  conversations and `snapshot.rs` the scenes (add one for a new surface).

## Design

Modern, quiet and uncluttered. Everything is the theme's greys; the only colour is a person's own
(`theme::name_color`: their face without a picture, their name in a group), a link, and the danger colour.

- The shell: the app draws its own title bar (`ui/src/titlebar.rs`) with the brand at its left. The rail
  (`desktop/src/shell.rs`) is icons only, at the far right: an app per icon with what waits there as a
  count, the settings at its foot. The selected app fills the rest.
- The chat app: its list sits on the window (`Palette::panel`); the conversation floats beside it on a
  rounded canvas (`Palette::bg`). Bubbles are `Palette::surface` with a hairline for others,
  `Palette::active` for one's own. The message box is a card on `Palette::surface` with a soft shadow: the
  paperclip at its left, send at its right, what it answers or edits and the attached files over the words.
- Say it with a control, not a sentence: an icon where one is understood, a chip for one of a few, a
  second click to confirm only what loses a whole chat or the session (sign out, leave or delete a chat).
  Deleting messages never asks: the owner dislikes confirmations.
- Whatever scrolls uses `widgets::Scroll`: the wheel moves a target that never leaves the content, and the
  view eases after it.
- Colours and sizes live in `ui/src/theme.rs`, from the active `Palette` (`p.theme`), never hard-coded, so
  every theme (Black default, White, Amber) keeps working.
- Icons come from the Lucide font (`ui::icon`): add a glyph by its codepoint from `lucide-static`'s
  `font/lucide.css` and to `icon::ALL`; the icon test checks every one resolves.
- Nothing animates by itself: an idle app draws nothing.
