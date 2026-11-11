//! A picture looked at alone: over the whole app, as large as it fits,
//! with the others it came with a key away. It is there at once, as the
//! bubble showed it, and turns sharp when the whole picture has come.

use adelite_chat::{Body, FileId, Message, MessageId};
use adelite_ui::image::{Lookup, MAX_THUMB};
use adelite_ui::paint::{Painter, Rect, fade};
use adelite_ui::ui::{Ui, icon, id};
use adelite_ui::{platform, theme};
use winit::window::CursorIcon;

use crate::store::Item;
use crate::{ChatModule, Overlay};

/// One of the pictures being looked at.
struct Picture {
    /// The picture as its bubble shows it: what is on screen until the whole one is.
    small: Option<FileId>,
    /// The picture at its largest.
    whole: Option<FileId>,
    /// Its size in pixels.
    size: (u32, u32),
}

/// The pictures being looked at.
pub(crate) struct Viewer {
    pictures: Vec<Picture>,
    /// The one that is shown.
    at: usize,
}

impl Viewer {
    /// The pictures among `messages`, showing the one of the message `shown`.
    fn of_messages<'a>(messages: impl Iterator<Item = &'a Message>, shown: MessageId) -> Option<Self> {
        let mut pictures = Vec::new();
        let mut at = 0;
        for message in messages {
            if let Body::Photo { file, full, width, height, .. } = &message.body {
                if message.id == shown {
                    at = pictures.len();
                }
                pictures.push(Picture { small: *file, whole: full.or(*file), size: (*width, *height) });
            }
        }
        (!pictures.is_empty()).then_some(Self { pictures, at })
    }

    /// The pictures of the row `item`, showing the one at `at` among its messages.
    pub(crate) fn of(item: &Item, at: usize) -> Option<Self> {
        let shown = item.messages().nth(at).map_or(item.message.id, |message| message.id);
        Self::of_messages(item.messages(), shown)
    }

    /// The pictures among what was sent in a chat (newest first), showing
    /// the one of the message `shown`. They are walked through oldest first,
    /// as in the conversation.
    pub(crate) fn among(sent: &[Message], shown: MessageId) -> Option<Self> {
        Self::of_messages(sent.iter().rev(), shown)
    }

    /// Shows the next picture, or the one before; the ends stay where they are.
    pub(crate) fn step(&mut self, forward: bool) {
        let last = self.pictures.len().saturating_sub(1);
        self.at = if forward { (self.at + 1).min(last) } else { self.at.saturating_sub(1) };
    }
}

/// Where a picture of `size` pixels lies in `area`: as large as fits with
/// room around it, never larger than its own pixels or than a thumbnail
/// can be at `scale` physical pixels per logical one.
fn fit(size: (u32, u32), area: Rect, scale: f32) -> Rect {
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    // Room is left for the buttons at its sides and under it.
    let room = ((area.w - 144.0).max(40.0), (area.h - 128.0).max(40.0));
    // A pixel short of the largest thumbnail: rounding to whole pixels must never go past it.
    let most = (MAX_THUMB as f32 - 2.0) / scale.max(0.1);
    let shrink = (room.0 / w).min(room.1 / h).min(most / w.max(h)).min(1.0);
    let (w, h) = ((w * shrink).floor().max(1.0), (h * shrink).floor().max(1.0));
    Rect::new((area.x + (area.w - w) * 0.5).round(), (area.y + (area.h - h) * 0.5).round(), w, h)
}

impl ChatModule {
    /// Draws the picture over the app.
    pub(crate) fn draw_viewer(&mut self, p: &mut Painter<'_>, ui: &mut Ui, area: Rect) {
        let t = p.theme;
        let Overlay::Viewer(viewer) = &mut self.overlay else { return };
        let appear = ui.appear(id("viewer"));
        // Twice the scrim: what is under it is out of mind while the picture is looked at.
        p.rect(area, fade(t.scrim, appear), 0.0);
        p.rect(area, fade(t.scrim, appear), 0.0);
        let Some(picture) = viewer.pictures.get(viewer.at) else { return };
        let frame = fit(picture.size, area, p.scale);
        let path_of = |file: Option<FileId>| file.and_then(|file| self.store.files.get(&file));
        let (small, whole) = (path_of(picture.small), path_of(picture.whole));
        // The whole picture is fetched when it is looked at, not before.
        if whole.is_none() {
            self.want.extend(picture.whole);
        }
        // Sharp when the whole picture is here and decoded; until then, as the bubble showed it.
        let state = whole.map_or(Lookup::Loading, |path| p.image(path, frame, 6.0));
        let sharp = matches!(state, Lookup::Ready(_));
        let shown = sharp || small.is_some_and(|path| p.image_at_hand(path, frame, 6.0));
        if !shown {
            p.rect(frame, fade(t.on_scrim, 0.06), 6.0);
        }
        let words = match state {
            Lookup::Ready(_) => "",
            Lookup::Loading => "Loading…",
            Lookup::Failed => "This picture cannot be shown here",
        };
        if !words.is_empty() {
            let words = p.layout(words, theme::CAPTION, None);
            let at = if shown { frame.bottom() - 34.0 } else { frame.y + (frame.h - 22.0) * 0.5 };
            let pill = Rect::new(frame.x + (frame.w - words.width()) * 0.5 - 10.0, at, words.width() + 20.0, 22.0);
            p.rect(pill, fade(t.scrim, appear), 11.0);
            p.text(&words, pill.x + 10.0, pill.y + (pill.h - words.height()) * 0.5, t.on_scrim);
        }

        let several = viewer.pictures.len() > 1;
        let mut on_button = false;
        let mut press = |p: &mut Painter<'_>, ui: &mut Ui, center: (f32, f32), glyph: char, enabled: bool| {
            let rect = Rect::new(center.0 - 19.0, center.1 - 19.0, 38.0, 38.0);
            let hovered = enabled && ui.hovered(rect);
            let hover = ui.anim(id(("viewer", glyph)), f32::from(u8::from(hovered)));
            p.rect(rect, fade(t.scrim, appear * (0.6 + 0.4 * hover)), 19.0);
            icon::draw(p, glyph, center, 17.0, fade(t.on_scrim, appear * if enabled { 1.0 } else { 0.35 }));
            on_button |= ui.hovered(rect);
            if hovered {
                ui.cursor = CursorIcon::Pointer;
            }
            hovered && ui.clicked(rect)
        };
        let on_disk = whole.or(small);
        let close = press(p, ui, (area.right() - 36.0, area.y + 36.0), icon::X, true);
        let outside = press(p, ui, (area.right() - 36.0, area.bottom() - 36.0), icon::EXTERNAL, on_disk.is_some());
        let folder = press(p, ui, (area.right() - 82.0, area.bottom() - 36.0), icon::FOLDER, on_disk.is_some());
        let mid = area.y + area.h * 0.5;
        let back = several && press(p, ui, (area.x + 36.0, mid), icon::CHEVRON_LEFT, viewer.at > 0);
        let last = viewer.at + 1 >= viewer.pictures.len();
        let forward = several && press(p, ui, (area.right() - 36.0, mid), icon::CHEVRON_RIGHT, !last);
        if several {
            let count = format!("{} of {}", viewer.at + 1, viewer.pictures.len());
            let count = p.layout(&count, theme::CAPTION, None);
            let pill =
                Rect::new(area.x + (area.w - count.width()) * 0.5 - 10.0, area.y + 25.0, count.width() + 20.0, 22.0);
            p.rect(pill, fade(t.scrim, appear), 11.0);
            p.text(&count, pill.x + 10.0, pill.y + (pill.h - count.height()) * 0.5, t.on_scrim);
        }
        if back || forward {
            viewer.step(forward);
        } else if outside || folder {
            // In the system's own viewer, or its folder: for keeping, printing, sending on.
            if let Some(path) = on_disk {
                let _ = if outside {
                    platform::open_url(path)
                } else {
                    std::path::Path::new(path).parent().map_or(Ok(()), platform::open_folder)
                };
            }
        } else if close || (ui.released && !on_button && !frame.contains(ui.mouse)) {
            self.close_overlay();
        }
    }
}

#[cfg(test)]
mod tests {
    use adelite_chat::Text;

    use super::*;

    #[test]
    fn a_picture_fits_its_room_and_the_atlas_at_every_scale() {
        let area = Rect::new(0.0, 36.0, 1388.0, 864.0);
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 3.0] {
            for size in [(2560, 1706), (1706, 2560), (5000, 300), (90, 60), (0, 0)] {
                let frame = fit(size, area, scale);
                // What the painter asks the atlas for: the frame in whole physical pixels.
                let (w, h) = ((frame.w * scale).round(), (frame.h * scale).round());
                assert!(w <= MAX_THUMB as f32 && h <= MAX_THUMB as f32, "{size:?} at {scale}: {w}x{h}");
                assert!(frame.w >= 1.0 && frame.h >= 1.0 && frame.x >= area.x && frame.bottom() <= area.bottom());
                assert!(frame.w <= size.0.max(1) as f32, "never larger than its own pixels");
            }
        }
        // No room at all is no reason to panic.
        let _ = fit((800, 600), Rect::new(0.0, 0.0, 0.0, 0.0), 1.0);
    }

    #[test]
    fn the_pictures_of_a_chat_are_walked_through_oldest_first() {
        let photo = |id, file| Message {
            id,
            body: Body::Photo {
                file: Some(file),
                full: Some(file + 100),
                width: 4,
                height: 3,
                caption: Text::default(),
            },
            ..Message::default()
        };
        let video = Message { id: 8, body: Body::default(), ..Message::default() };
        // Newest first, as a chat's sent pictures come.
        let sent = [photo(9, 3), video, photo(7, 2), photo(5, 1)];
        let mut viewer = Viewer::among(&sent, 7).expect("three pictures");
        assert_eq!((viewer.pictures.len(), viewer.at), (3, 1));
        assert_eq!(viewer.pictures[1].whole, Some(102));
        viewer.step(true);
        viewer.step(true);
        assert_eq!(viewer.at, 2, "the last stays the last");
        viewer.step(false);
        viewer.step(false);
        viewer.step(false);
        assert_eq!(viewer.at, 0);
        assert!(Viewer::among(&sent[1..2], 8).is_none(), "nothing to look at");
    }
}
