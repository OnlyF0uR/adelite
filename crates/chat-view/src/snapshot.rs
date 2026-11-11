//! The chat app drawn offscreen and saved as PNGs, to look at the design
//! in every scheme without an account. Run by hand:
//! `ADELITE_SNAPSHOTS=<folder> cargo test -p adelite-chat-view snapshots -- --ignored`

use std::path::{Path, PathBuf};

use adelite_chat::Session;
use adelite_ui::gpu::Offscreen;
use adelite_ui::snapshot::{frame, write_png};
use adelite_ui::text::Fonts;
use adelite_ui::theme::Scheme;
use adelite_ui::{Context, Module};

use crate::{ChatModule, demo};

/// Logical size of every snapshot: the window without its title bar and rail.
const SIZE: (u32, u32) = (1388, 864);

/// A picture for the demo's photo and face: a soft diagonal blend.
pub(crate) fn picture(dir: &Path) -> String {
    let (w, h) = (640_u32, 400_u32);
    let pixels: Vec<u8> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x as f32 / w as f32, y as f32 / h as f32)))
        .flat_map(|(u, v)| [(60.0 + 150.0 * u) as u8, (90.0 + 110.0 * v) as u8, (190.0 - 90.0 * u * v) as u8, 255])
        .collect();
    let path = dir.join("demo-picture.png");
    write_png(&path, (w, h), &pixels);
    path.display().to_string()
}

#[test]
#[ignore = "draws on the GPU; run by hand to look at the design"]
fn snapshots() {
    let Some(dir) = std::env::var_os("ADELITE_SNAPSHOTS").map(PathBuf::from) else { return };
    let Some(mut gpu) = Offscreen::new() else { return };
    std::fs::create_dir_all(&dir).expect("snapshot folder");
    let fonts = Fonts::load();
    let picture = picture(&dir);
    for scheme in Scheme::ALL {
        // Each with where the mouse is, for what shows only under it.
        let away = (-1.0, -1.0);
        let scenes: Vec<(&str, ChatModule, (f32, f32))> = vec![
            ("chat", demo::signed_in(true, Some(&picture)), away),
            ("media", demo::media(Some(&picture)), (700.0, 330.0)),
            ("writing", demo::writing(Some(&picture)), away),
            ("list", demo::signed_in(false, None), away),
            ("chat-menu", demo::chat_menu(Some(&picture)), (230.0, 262.0)),
            ("message-menu", demo::message_menu(Some(&picture)), (800.0, 560.0)),
            ("picked", demo::picked(Some(&picture)), away),
            ("marked", demo::marked(Some(&picture)), away),
            ("search", demo::search(Some(&picture)), away),
            ("topics", demo::topics(Some(&picture)), away),
            ("info", demo::info(Some(&picture)), (690.0, 470.0)),
            ("info-media", demo::info_media(Some(&picture)), away),
            ("info-files", demo::info_files(Some(&picture)), (690.0, 400.0)),
            ("info-edit", demo::info_edit(Some(&picture)), away),
            ("forward", demo::forward(Some(&picture)), away),
            ("viewer", demo::viewer(Some(&picture)), away),
            ("channel", demo::channel(Some(&picture)), away),
            ("starting", demo::at(Session::Starting), away),
            ("connect", demo::at(Session::Asking(demo::app_prompt())), away),
            ("code", demo::at(Session::Asking(demo::code_prompt())), away),
            (
                "missing",
                demo::at(Session::Unavailable {
                    title: "Telegram's library is missing".into(),
                    detail: "Adelite talks to Telegram through TDLib. Put tdjson.dll beside the app, or build it with packaging/tdlib.".into(),
                }),
                away,
            ),
        ];
        for (name, mut module, mouse) in scenes {
            let pixels = frame(&mut gpu, &fonts, scheme, SIZE, &mut |p, ui, view| {
                ui.mouse = mouse;
                let (mut clipboard, mut requests) = (None, Vec::new());
                module.draw(p, ui, view, &mut Context { clipboard: &mut clipboard, requests: &mut requests });
            });
            write_png(&dir.join(format!("chat-{name}-{}.png", scheme.key())), SIZE, &pixels);
        }
    }
}

/// Every screen draws, with nothing to show it on: no panic at any size,
/// down to no room at all.
#[test]
fn every_screen_draws_at_any_size() {
    use adelite_ui::image::ImageAtlas;
    use adelite_ui::paint::{Painter, Rect};
    use adelite_ui::text::GlyphAtlas;
    use adelite_ui::ui::Ui;

    let fonts = Fonts::load();
    let scenes = vec![
        demo::signed_in(true, None),
        demo::media(None),
        demo::writing(None),
        demo::signed_in(false, None),
        demo::chat_menu(None),
        demo::message_menu(None),
        demo::picked(None),
        demo::marked(None),
        demo::search(None),
        demo::topics(None),
        demo::info(None),
        demo::info_media(None),
        demo::info_files(None),
        demo::info_edit(None),
        demo::forward(None),
        demo::viewer(None),
        demo::channel(None),
        demo::at(Session::Starting),
        demo::at(Session::Asking(demo::app_prompt())),
        demo::at(Session::Asking(demo::code_prompt())),
    ];
    for mut module in scenes {
        for size in [(1388.0, 864.0), (700.0, 500.0), (200.0, 120.0), (0.0, 0.0)] {
            let (mut atlas, mut images, mut instances) = (GlyphAtlas::default(), ImageAtlas::default(), Vec::new());
            let mut ui = Ui::default();
            let view = Rect::new(0.0, 0.0, size.0, size.1);
            // The mouse over the conversation: what shows under it is drawn too.
            ui.mouse = (size.0 * 0.6, size.1 * 0.5);
            for _ in 0..3 {
                ui.begin(0.016, 1.0);
                let mut p =
                    Painter::new(&fonts, (&mut atlas, &mut images), &mut instances, 1.0, view, Scheme::Black.palette());
                let (mut clipboard, mut requests) = (None, Vec::new());
                module.draw(&mut p, &mut ui, view, &mut Context { clipboard: &mut clipboard, requests: &mut requests });
                ui.end();
            }
            assert!(size.0 < 1.0 || !instances.is_empty(), "something is drawn at {size:?}");
        }
    }
}

/// A chat that is opened again is drawn at its end from its very first
/// frame: no frame shows another part of it, which the eye takes for a
/// flicker of some other chat.
#[test]
fn a_chat_opened_again_is_at_its_end_at_once() {
    use adelite_chat::{Body, Chat, Event, Kind, List, Message, Page, Place, Text};
    use adelite_ui::image::ImageAtlas;
    use adelite_ui::paint::{Painter, Rect};
    use adelite_ui::text::GlyphAtlas;
    use adelite_ui::ui::Ui;

    let fonts = Fonts::load();
    let mut module = demo::signed_in(true, None);
    let (mut atlas, mut images, mut instances) = (GlyphAtlas::default(), ImageAtlas::default(), Vec::new());
    let mut ui = Ui::default();
    (ui.focused, ui.mouse) = (true, (-1.0, -1.0));
    let view = Rect::new(0.0, 0.0, SIZE.0 as f32, SIZE.1 as f32);
    let mut draw = |module: &mut ChatModule, ui: &mut Ui| {
        ui.begin(0.016, 1.0);
        let mut p = Painter::new(&fonts, (&mut atlas, &mut images), &mut instances, 1.0, view, Scheme::Black.palette());
        let (mut clipboard, mut requests) = (None, Vec::new());
        module.draw(&mut p, ui, view, &mut Context { clipboard: &mut clipboard, requests: &mut requests });
        ui.end();
        instances.iter().map(|quad| quad.rect).collect::<Vec<_>>()
    };
    // Looked at, read, and left for another chat: its rows are measured and kept.
    for _ in 0..3 {
        draw(&mut module, &mut ui);
    }
    let places = vec![Place { list: List::Main, order: 99, pinned: true }];
    let read = Chat {
        id: -60,
        title: "Design".into(),
        kind: Kind::Group,
        places,
        can_send: true,
        joined: true,
        ..Chat::default()
    };
    let (mut clipboard, mut requests) = (None, Vec::new());
    module.apply(Event::Chat(read), &mut Context { clipboard: &mut clipboard, requests: &mut requests });
    module.pick(7);
    draw(&mut module, &mut ui);
    module.pick(-60);
    let first = draw(&mut module, &mut ui);
    let second = draw(&mut module, &mut ui);
    let third = draw(&mut module, &mut ui);
    assert!(first.len() > 200, "the conversation is drawn: {} quads", first.len());
    assert_eq!(first, second, "the first frame is the conversation as it stays");
    assert_eq!(second, third);

    // A chat never opened: nothing until its messages come, then all of them at their end, at once.
    module.pick(8);
    let waiting = draw(&mut module, &mut ui);
    let said = |id: i64| Message {
        id,
        chat: 8,
        date: 1_794_300_000 + id * 400,
        sender: if id % 3 == 0 { 1 } else { 8 },
        outgoing: id % 3 == 0,
        body: Body::Text(Text::plain(format!("Message number {id}, long enough to be a line of its own."))),
        ..Message::default()
    };
    let page = Event::History { chat: 8, topic: 0, page: Page::Older, messages: (1..=60).map(said).collect() };
    let (mut clipboard, mut requests) = (None, Vec::new());
    module.apply(page, &mut Context { clipboard: &mut clipboard, requests: &mut requests });
    let first = draw(&mut module, &mut ui);
    let second = draw(&mut module, &mut ui);
    assert!(first.len() > waiting.len() + 100, "its messages are drawn");
    assert_eq!(first, second, "at their end from the first frame");
    // Older messages that come in above leave what is on screen where it is.
    let older = Event::History { chat: 8, topic: 0, page: Page::Older, messages: (-40..=0).map(said).collect() };
    module.store.thread_mut((8, 0)).loading = true;
    module.apply(older, &mut Context { clipboard: &mut clipboard, requests: &mut requests });
    assert_eq!(draw(&mut module, &mut ui), second, "what is above the view does not move it");
}

/// A chat with something unread opens at the first new message, from its first frame.
#[test]
fn a_chat_with_news_opens_at_the_news_at_once() {
    use adelite_ui::image::ImageAtlas;
    use adelite_ui::paint::{Painter, Rect};
    use adelite_ui::text::GlyphAtlas;
    use adelite_ui::ui::Ui;

    let fonts = Fonts::load();
    let mut module = demo::signed_in(true, None);
    let (mut atlas, mut images, mut instances) = (GlyphAtlas::default(), ImageAtlas::default(), Vec::new());
    let mut ui = Ui::default();
    (ui.focused, ui.mouse) = (true, (-1.0, -1.0));
    let view = Rect::new(0.0, 0.0, SIZE.0 as f32, SIZE.1 as f32);
    let mut frames = Vec::new();
    for _ in 0..3 {
        ui.begin(0.016, 1.0);
        let mut p = Painter::new(&fonts, (&mut atlas, &mut images), &mut instances, 1.0, view, Scheme::Black.palette());
        let (mut clipboard, mut requests) = (None, Vec::new());
        module.draw(&mut p, &mut ui, view, &mut Context { clipboard: &mut clipboard, requests: &mut requests });
        ui.end();
        frames.push(instances.iter().map(|quad| quad.rect).collect::<Vec<_>>());
    }
    assert_eq!(frames[0], frames[1], "the first frame is the conversation as it stays");
    assert_eq!(frames[1], frames[2]);
}

/// What a frame of the conversation costs to build on the CPU, in steady
/// state (the layout and the draw list; the GPU's part is one draw call):
/// `cargo test --release -p adelite-chat-view frame_cost -- --ignored --nocapture`
#[test]
#[ignore = "a measurement; run by hand in release"]
fn frame_cost() {
    use adelite_ui::image::ImageAtlas;
    use adelite_ui::paint::{Painter, Rect};
    use adelite_ui::text::GlyphAtlas;
    use adelite_ui::ui::Ui;

    let fonts = Fonts::load();
    let mut module = demo::signed_in(true, None);
    let (mut atlas, mut images, mut instances) = (GlyphAtlas::default(), ImageAtlas::default(), Vec::new());
    let mut ui = Ui::default();
    let view = Rect::new(0.0, 0.0, SIZE.0 as f32, SIZE.1 as f32);
    let mut draw = |ui: &mut Ui, instances: &mut Vec<_>| {
        ui.begin(0.016, 1.0);
        let mut p = Painter::new(&fonts, (&mut atlas, &mut images), instances, 1.0, view, Scheme::Black.palette());
        let (mut clipboard, mut requests) = (None, Vec::new());
        module.draw(&mut p, ui, view, &mut Context { clipboard: &mut clipboard, requests: &mut requests });
        ui.end();
    };
    // Warm: glyphs rasterised, heights measured.
    for _ in 0..20 {
        draw(&mut ui, &mut instances);
    }
    let frames = 2_000;
    let started = std::time::Instant::now();
    for _ in 0..frames {
        draw(&mut ui, &mut instances);
    }
    let each = started.elapsed().as_secs_f64() * 1e6 / f64::from(frames);
    println!("frame: {each:.0} µs for {} quads", instances.len());
    assert!(each < 4_000.0, "a frame takes {each:.0} µs: over a quarter of 60 Hz's budget");
}

/// Words are picked by dragging over them, across messages; the release
/// that ends the drag is not a click. A double press picks a word, and
/// Escape lets the pick go.
#[test]
fn words_are_picked_by_dragging_over_them() {
    use adelite_chat::{Body, Event, Message, Page, Text};
    use adelite_ui::image::ImageAtlas;
    use adelite_ui::paint::{Painter, Rect};
    use adelite_ui::text::GlyphAtlas;
    use adelite_ui::theme;
    use adelite_ui::ui::Ui;

    let fonts = Fonts::load();
    let mut module = demo::signed_in(true, None);
    let (mut atlas, mut images, mut instances) = (GlyphAtlas::default(), ImageAtlas::default(), Vec::new());
    let mut ui = Ui::default();
    (ui.focused, ui.mouse) = (true, (-1.0, -1.0));
    let view = Rect::new(0.0, 0.0, SIZE.0 as f32, SIZE.1 as f32);
    let mut draw = |module: &mut ChatModule, ui: &mut Ui| {
        ui.begin(0.016, 1.0);
        let mut p = Painter::new(&fonts, (&mut atlas, &mut images), &mut instances, 1.0, view, Scheme::Black.palette());
        let (mut clipboard, mut requests) = (None, Vec::new());
        module.draw(&mut p, ui, view, &mut Context { clipboard: &mut clipboard, requests: &mut requests });
        ui.end();
    };
    module.pick(8);
    // Nobody's own: every bubble at the column's left, every one with words.
    let said = |id: i64| Message {
        id,
        chat: 8,
        date: 1_794_300_000 + id * 400,
        sender: 8,
        body: Body::Text(Text::plain(format!("Message number {id}, long enough to be a line of its own."))),
        ..Message::default()
    };
    let page = Event::History { chat: 8, topic: 0, page: Page::Older, messages: (1..=30).map(said).collect() };
    let (mut clipboard, mut requests) = (None, Vec::new());
    module.apply(page, &mut Context { clipboard: &mut clipboard, requests: &mut requests });
    for _ in 0..3 {
        draw(&mut module, &mut ui);
    }
    // Where the words of the bubbles start: past the list, in the middle of the canvas.
    let canvas_x = theme::SIDEBAR_WIDTH.min(view.w * 0.4);
    let canvas_w = view.w - canvas_x;
    let column_w = (canvas_w - 48.0).clamp(120.0, 920.0);
    let x = (canvas_x + (canvas_w - column_w) * 0.5).round() + 40.0;
    let mut time = 10.0;
    let mut press = |ui: &mut Ui, at: (f32, f32), gap: f32| {
        time += gap;
        ui.mouse = at;
        ui.press(time);
    };
    let release = |ui: &mut Ui| (ui.down, ui.released) = (false, true);
    // Down the column, from the middle of the view: the first press that lands on words.
    let mut found = None;
    for y in (300..700).step_by(4).map(|y| y as f32) {
        press(&mut ui, (x, y), 1.0);
        draw(&mut module, &mut ui);
        if module.marked.is_some() {
            found = Some(y);
            break;
        }
        release(&mut ui);
        draw(&mut module, &mut ui);
    }
    let y = found.expect("a press on a message's words starts a pick");
    assert!(module.marked.is_some_and(|marked| marked.is_empty()), "a press alone picks nothing yet");
    // Dragged up into the message above: the end of that one and the start of this one.
    ui.mouse = (x + 30.0, y - 70.0);
    draw(&mut module, &mut ui);
    draw(&mut module, &mut ui);
    let picked = module.marked_text().expect("words are picked");
    let (above, here) = picked.split_once("\n\n").expect("over two messages");
    assert!(above.starts_with("Message number ") && above.ends_with("own."), "the one above, to its end: {above:?}");
    assert!(!here.is_empty() && "Message number".starts_with(here), "this one, up to the press: {here:?}");
    release(&mut ui);
    draw(&mut module, &mut ui);
    assert_eq!(module.marked_text().as_deref(), Some(picked.as_str()), "letting go keeps the pick");
    assert!(module.marked.is_some_and(|marked| !marked.dragging));
    // A double press picks the word under it.
    press(&mut ui, (x, y), 1.0);
    draw(&mut module, &mut ui);
    release(&mut ui);
    draw(&mut module, &mut ui);
    assert!(module.marked.is_none(), "a click on words lets the pick go");
    press(&mut ui, (x, y), 0.1);
    draw(&mut module, &mut ui);
    release(&mut ui);
    draw(&mut module, &mut ui);
    let word = module.marked_text().expect("a word is picked");
    assert!(!word.is_empty() && word.chars().all(char::is_alphanumeric), "one word: {word:?}");
    module.escape();
    assert!(module.marked.is_none());
}
