//! The whole window drawn offscreen and saved as PNGs, to look at the
//! design in every scheme without opening the app. Run by hand:
//! `ADELITE_SNAPSHOTS=<folder> cargo test -p adelite snapshots -- --ignored`

use std::path::PathBuf;

use adelite_chat_view::demo;
use adelite_ui::gpu::Offscreen;
use adelite_ui::snapshot::{frame, write_png};
use adelite_ui::text::Fonts;
use adelite_ui::theme::Scheme;
use adelite_ui::{Context, Module, titlebar};
use adelite_update::{Asset, Release, State, Updater, Version};

use crate::shell::Shell;

/// Logical size of every snapshot, drawn at a scale of one.
const SIZE: (u32, u32) = (1440, 900);

#[test]
#[ignore = "draws on the GPU; run by hand to look at the design"]
fn snapshots() {
    let Some(dir) = std::env::var_os("ADELITE_SNAPSHOTS").map(PathBuf::from) else { return };
    let Some(mut gpu) = Offscreen::new() else { return };
    std::fs::create_dir_all(&dir).expect("snapshot folder");
    let fonts = Fonts::load();
    let picture = dir.join("demo-picture.png");
    let picture = picture.exists().then(|| picture.display().to_string());
    for scheme in Scheme::ALL {
        let chat = || -> Vec<Box<dyn Module>> { vec![Box::new(demo::signed_in(true, picture.as_deref()))] };
        let shell = |state| Shell::new(chat(), Updater::showing(state));
        let mut settings = shell(State::Idle);
        settings.open_settings(false);
        let mut closing = shell(State::Idle);
        closing.open_settings(true);
        let mut update = shell(State::Available(newer()));
        update.open_settings(false);
        let mut notice = shell(State::Idle);
        notice.tell("A message could not be sent: You cannot write in this chat.".into(), true);
        let scenes = [
            ("window", shell(State::Idle)),
            ("settings", settings),
            ("settings-close", closing),
            ("settings-update", update),
            ("notice", notice),
        ];
        for (name, mut shell) in scenes {
            let pixels = frame(&mut gpu, &fonts, scheme, SIZE, &mut |p, ui, view| {
                let (mut clipboard, mut requests) = (None, Vec::new());
                shell.draw(p, ui, view, scheme, &mut Context { clipboard: &mut clipboard, requests: &mut requests });
                titlebar::draw(p, ui, view, false);
            });
            write_png(&dir.join(format!("{name}-{}.png", scheme.key())), SIZE, &pixels);
        }
    }
}

/// A made-up newer release, offered in the settings.
fn newer() -> Release {
    let file = |name: &str| Asset { name: name.to_owned(), url: format!("https://example.com/{name}"), size: 1 };
    Release {
        version: Version(9, 1, 0),
        page: "https://example.com/releases".to_owned(),
        download: Some((file("Adelite-9.1.0"), file("Adelite-9.1.0.sig"))),
    }
}
