//! One row of a conversation: how large its bubble is ([`lay`]) and how it
//! is drawn ([`draw`]). Measuring and drawing share the layout, so a bubble
//! is exactly as tall as it was measured.
//!
//! A bubble is built from the top down: who wrote it, whose message it
//! passes on, what it answers, what it holds (a picture, an album, a video,
//! a file, a poll…), its words, the page its link leads to, what people
//! answered it with, and the time. Buttons hang under it.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use adelite_chat::{Body, Delivery, FileId, LinkPreview, Message, Motion, Poll, Press, Style, Text};
use adelite_ui::font::FontId;
use adelite_ui::image::Lookup;
use adelite_ui::paint::{Painter, Rect, fade, mix};
use adelite_ui::text::{BACKGROUND, Run, STRIKE, TextLayout, UNDERLINE};
use adelite_ui::theme;
use adelite_ui::ui::icon;

use crate::label;
use crate::store::Item;

/// Space between a bubble's edge and its words.
pub const PAD_X: f32 = 12.0;
const PAD_Y: f32 = 7.0;
/// Space between a bubble's edge and a picture in it.
const PHOTO_INSET: f32 = 4.0;
const NAME_H: f32 = 19.0;
const FORWARD_H: f32 = 19.0;
const REPLY_H: f32 = 44.0;
const FILE_H: f32 = 48.0;
const OTHER_H: f32 = 24.0;
/// Height of the line the time stands on when it has one of its own.
const META_H: f32 = 15.0;
/// The largest a picture is shown.
const PHOTO_MAX: (f32, f32) = (360.0, 380.0);
/// The largest side of a sticker, and of a round video.
const STICKER_MAX: f32 = 168.0;
const ROUND: f32 = 200.0;
/// Space between the pictures of an album.
const ALBUM_GAP: f32 = 3.0;
/// Height of a chip of reactions, and the space around chips.
const CHIP: (f32, f32) = (24.0, 4.0);
/// Height of the line under a post that leads to its comments.
const COMMENTS_H: f32 = 34.0;
/// Height of a button under a bubble, and the space between buttons.
const BUTTON: (f32, f32) = (34.0, 4.0);
/// The widest the card of a link's page gets, and the tallest its picture.
const LINK_MAX: (f32, f32) = (320.0, 170.0);

/// What a message shows besides words.
enum Media {
    None,
    /// A picture or a video's still, this large.
    Visual(f32, f32),
    /// Several of them, each in its cell (from the media's corner).
    Album(Vec<Rect>),
    /// A picture without a bubble: a sticker, or a round video.
    Bare(f32, f32),
    Audio,
    File,
    Other,
    /// A question, and its answers: where each starts and how tall it is.
    Poll {
        question: TextLayout,
        options: Vec<(TextLayout, f32, f32)>,
    },
}

/// The card of the page a link leads to, measured.
struct LinkLaid {
    y: f32,
    w: f32,
    h: f32,
    title: Option<TextLayout>,
    about: Option<TextLayout>,
    /// The picture's size, under the words.
    photo: Option<(f32, f32)>,
}

/// A measured bubble.
pub struct Laid {
    /// Its width.
    pub w: f32,
    /// Its height, with the buttons under it.
    pub h: f32,
    /// The bubble's own height.
    body_h: f32,
    name: bool,
    reply: bool,
    media: Media,
    /// Where the media starts, from the bubble's top.
    media_y: f32,
    /// The words (the text, or the caption) and where they start.
    text: Option<TextLayout>,
    text_y: f32,
    link: Option<LinkLaid>,
    /// The chips of reactions: where each lies in the bubble, and how wide it is.
    chips: Vec<(f32, f32, f32)>,
    /// Where the line to the comments starts.
    comments_y: Option<f32>,
    /// The time, with "edited" before it; and how often it was seen, before that.
    meta: String,
    views: String,
    meta_w: f32,
    /// The time lies over the picture, on a scrim.
    meta_over: bool,
}

impl Laid {
    /// The bubble itself, without the buttons under it, when its corner is at `at`.
    #[must_use]
    pub fn body(&self, at: (f32, f32)) -> Rect {
        Rect::new(at.0, at.1, self.w, self.body_h)
    }

    /// The laid-out words and their top-left corner, when the bubble's corner is at `at`.
    #[must_use]
    pub fn words_at(&self, at: (f32, f32)) -> Option<(&TextLayout, (f32, f32))> {
        self.text.as_ref().map(|layout| (layout, (at.0 + PAD_X, at.1 + self.text_y)))
    }
}

/// What of a bubble the mouse is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Over {
    /// Nothing that answers a click.
    #[default]
    Nothing,
    /// Styled words: the index of their span.
    Span(usize),
    /// A picture, a video, a recording or a file: its place in the row's album.
    Media(usize),
    /// The message it answers.
    Reply,
    /// The card of the page its link leads to.
    Preview,
    /// What it was answered with: the index of the reaction.
    Reaction(usize),
    /// An answer of its poll.
    Answer(usize),
    /// The line to its comments.
    Comments,
    /// A button under it: its row and its place there.
    Button(usize, usize),
}

/// The words of a row that are laid out: its text, or the caption of whichever of its album has one.
pub fn words(item: &Item) -> Option<&Text> {
    item.messages().filter_map(|message| message.body.words()).find(|words| !words.text.is_empty())
}

/// Lays out styled words in a column `width` wide. A clickable run carries
/// the index of its span, so a click finds what it leads to without it
/// being copied. Hidden words keep their room and lose their ink until `revealed`.
fn rich(p: &Painter<'_>, words: &Text, width: f32, revealed: bool) -> TextLayout {
    let runs: Vec<(Range<usize>, Run)> = words
        .spans
        .iter()
        .enumerate()
        .map(|(index, span)| {
            let mut run = Run::plain(FontId::REGULAR);
            let link = index as u16 + 1;
            match &span.style {
                Style::Bold => run.font = FontId::SEMIBOLD,
                Style::Italic => run.font = FontId::ITALIC,
                Style::Code => (run.font, run.size, run.decoration) = (FontId::MONO, 0.92, BACKGROUND),
                Style::Strike => run.decoration = STRIKE,
                Style::Underline => run.decoration = UNDERLINE,
                Style::Link(_) => (run.ink, run.decoration, run.link) = (1, UNDERLINE, link),
                Style::Mention(target) if target.is_empty() => run.ink = 1,
                Style::Mention(_) | Style::Tag => (run.ink, run.link) = (1, link),
                Style::Spoiler if revealed => {}
                Style::Spoiler => (run.ink, run.decoration, run.link) = (2, BACKGROUND, link),
                Style::Quote => (run.font, run.ink) = (FontId::ITALIC, 3),
            }
            (span.range.clone(), run)
        })
        .collect();
    TextLayout::rich(p.fonts, &words.text, &runs, theme::BODY, Some(width), p.scale)
}

/// The size a picture of `width` by `height` pixels is shown at in a bubble at most `max_w` wide.
fn fit(width: u32, height: u32, max_w: f32) -> (f32, f32) {
    let (w, h) = if width == 0 || height == 0 { (320.0, 200.0) } else { (width as f32, height as f32) };
    let room = (max_w - 2.0 * PHOTO_INSET).min(PHOTO_MAX.0);
    let scale = (room / w).min(PHOTO_MAX.1 / h).min(1.0);
    ((w * scale).round().max(90.0).min(room), (h * scale).round().max(60.0))
}

/// The cells of an album of `count` pictures in a block `w` wide: rows of
/// one to three, the shorter rows first. Returns them and the block's height.
fn album(count: usize, w: f32) -> (Vec<Rect>, f32) {
    // Three are one large picture over a pair; more are rows of three, led by a pair or two where they do not divide.
    let rows: Vec<usize> = match (count, count % 3) {
        (0..=2, _) => vec![count],
        (3, _) => vec![1, 2],
        (4, _) => vec![2, 2],
        (_, 0) => vec![3; count / 3],
        (_, 1) => [2, 2].into_iter().chain(std::iter::repeat_n(3, (count - 4) / 3)).collect(),
        _ => std::iter::once(2).chain(std::iter::repeat_n(3, (count - 2) / 3)).collect(),
    };
    let mut cells = Vec::with_capacity(count);
    let mut y = 0.0;
    for columns in rows {
        let cell_w = (w - ALBUM_GAP * (columns as f32 - 1.0)) / columns as f32;
        let cell_h = (cell_w * if columns == 1 { 0.62 } else { 0.82 }).round().min(260.0);
        for column in 0..columns {
            cells.push(Rect::new((column as f32 * (cell_w + ALBUM_GAP)).round(), y, cell_w.round(), cell_h));
        }
        y += cell_h + ALBUM_GAP;
    }
    (cells, y - ALBUM_GAP)
}

/// Measures the bubble of the row `item` in a column where bubbles are at
/// most `max_w` wide. `name`: the sender is named at its top. `revealed`:
/// its hidden words are shown.
pub fn lay(p: &Painter<'_>, item: &Item, name: bool, max_w: f32, revealed: bool) -> Laid {
    let message = &item.message;
    let inner_max = (max_w - 2.0 * PAD_X).max(40.0);
    let width_of = |text: &str, style| p.layout(text, style, None).width().min(inner_max);
    let mut inner_w: f32 = 0.0;
    let mut y = PAD_Y;
    if name {
        inner_w = inner_w.max(width_of(&message.sender_name, theme::CAPTION));
        y += NAME_H;
    }
    if !message.forwarded_from.is_empty() {
        inner_w = inner_w.max(width_of(&message.forwarded_from, theme::TINY) + 96.0).min(inner_max);
        y += FORWARD_H;
    }
    let reply = message.reply_to.is_some();
    if reply {
        inner_w = inner_w.max(180.0_f32.min(inner_max));
        y += REPLY_H;
    }
    let mut media_y = y;
    // A picture at the top of a bubble sits closer to its edge than words do.
    let mut picture = |w: f32, h: f32, y: &mut f32, inner_w: &mut f32| {
        if *y <= PAD_Y {
            media_y = PHOTO_INSET;
        }
        *inner_w = inner_w.max(w + 2.0 * PHOTO_INSET - 2.0 * PAD_X);
        *y = media_y + h + 6.0;
    };
    let media = match &message.body {
        Body::Photo { .. } | Body::Video { motion: Motion::Video | Motion::Gif, .. } if !item.more.is_empty() => {
            let room = (max_w - 2.0 * PHOTO_INSET).min(PHOTO_MAX.0);
            let (cells, h) = album(item.more.len() + 1, room);
            picture(room, h, &mut y, &mut inner_w);
            Media::Album(cells)
        }
        Body::Video { motion: Motion::Round, .. } => Media::Bare(ROUND, ROUND),
        Body::Photo { width, height, .. } | Body::Video { width, height, .. } => {
            let (w, h) = fit(*width, *height, max_w);
            picture(w, h, &mut y, &mut inner_w);
            Media::Visual(w, h)
        }
        Body::Sticker { width, height, .. } => {
            let (w, h) = ((*width).max(1) as f32, (*height).max(1) as f32);
            let scale = (STICKER_MAX / w.max(h)).min(1.0);
            Media::Bare((w * scale).round().max(48.0), (h * scale).round().max(48.0))
        }
        Body::Audio { title, voice, .. } => {
            let line = if *voice { 150.0 } else { width_of(title, theme::LABEL).max(150.0) };
            inner_w = inner_w.max((line + 48.0).min(inner_max));
            y += FILE_H;
            Media::Audio
        }
        Body::File { name, size, .. } => {
            let line = width_of(name, theme::LABEL).max(width_of(&label::file_size(*size), theme::TINY) + 110.0);
            inner_w = inner_w.max((line + 48.0).min(inner_max));
            y += FILE_H;
            Media::File
        }
        Body::Other { label, .. } => {
            inner_w = inner_w.max(width_of(label, theme::SMALL) + 24.0);
            y += OTHER_H;
            Media::Other
        }
        Body::Poll(poll) => {
            let (poll, h) = lay_poll(p, poll, inner_max.min(340.0));
            inner_w = inner_w.max(inner_max.min(340.0));
            y += h;
            poll
        }
        Body::Text(_) | Body::Service(_) => Media::None,
    };
    if let Media::Bare(w, h) = media {
        // No bubble: the picture is the row, with the time in its corner.
        let meta = meta_of(p, item);
        return Laid {
            w,
            h,
            body_h: h,
            name: false,
            reply: false,
            media,
            media_y: 0.0,
            text: None,
            text_y: 0.0,
            link: None,
            chips: Vec::new(),
            comments_y: None,
            meta: meta.0,
            views: meta.1,
            meta_w: meta.2,
            meta_over: true,
        };
    }

    let (meta, views, meta_w) = meta_of(p, item);
    // Words under a picture are as wide as the picture: the bubble hugs it.
    let inner_max = match &media {
        Media::Visual(w, _) => (w + 2.0 * PHOTO_INSET - 2.0 * PAD_X).max(meta_w).min(inner_max),
        Media::Album(_) => inner_w.max(meta_w).min(inner_max),
        _ => inner_max,
    };
    let body_words = words(item);
    let text = body_words.map(|words| rich(p, words, inner_max, revealed));
    let text_y = y;
    // Where the last line of what is laid so far ends, for the time to stand beside it.
    let mut tail: Option<f32> = None;
    if let (Some(layout), Some(words)) = (&text, body_words) {
        inner_w = inner_w.max(layout.width());
        y += layout.height();
        tail = Some(layout.caret(words.text.len()).0);
    }
    let link = message.link.as_ref().map(|preview| {
        let link = lay_link(p, preview, y + 6.0, inner_max.min(LINK_MAX.0));
        inner_w = inner_w.max(link.w);
        y = link.y + link.h;
        tail = None;
        link
    });
    let mut chips = Vec::with_capacity(message.reactions.len());
    if !message.reactions.is_empty() {
        let (mut x, top) = (0.0, y + if text.is_some() || link.is_some() { 6.0 } else { 0.0 });
        y = top;
        for reaction in &message.reactions {
            let w = width_of(&reaction.emoji, theme::SMALL) + width_of(&label::count(reaction.count), theme::CAPTION);
            let w = (w + 20.0).round();
            if x > 0.0 && x + w > inner_max {
                (x, y) = (0.0, y + CHIP.0 + CHIP.1);
            }
            chips.push((x, y, w));
            x += w + CHIP.1;
            inner_w = inner_w.max(x - CHIP.1);
        }
        // The time stands beside the last chip where there is room.
        tail = Some(x - CHIP.1);
        y += CHIP.0;
    }
    let visual = matches!(media, Media::Visual(..) | Media::Album(_));
    let mut meta_over = false;
    match tail {
        // The time shares the last line.
        Some(end) if end + 10.0 + meta_w <= inner_max => {
            inner_w = inner_w.max(end + 10.0 + meta_w);
            y += PAD_Y;
        }
        Some(_) => y += META_H + PAD_Y,
        None if visual && link.is_none() && message.comments.is_none() => {
            // The time lies over the picture.
            meta_over = true;
            y += PHOTO_INSET - 6.0;
        }
        None => y += META_H + PAD_Y - 2.0,
    }
    let comments_y = message.comments.map(|_| {
        inner_w = inner_w.max(150.0_f32.min(inner_max));
        let at = y;
        y += COMMENTS_H;
        at
    });
    let mut w = (inner_w.max(meta_w) + 2.0 * PAD_X).min(max_w);
    let body_h = y;
    if !message.buttons.is_empty() {
        w = w.max(220.0_f32.min(max_w));
        y += message.buttons.len() as f32 * (BUTTON.0 + BUTTON.1);
    }
    Laid {
        w,
        h: y,
        body_h,
        name,
        reply,
        media,
        media_y,
        text,
        text_y,
        link,
        chips,
        comments_y,
        meta,
        views,
        meta_w,
        meta_over,
    }
}

/// The time as a row says it ("edited 14:32"), how often it was seen, and the width of both with their marks.
fn meta_of(p: &Painter<'_>, item: &Item) -> (String, String, f32) {
    let message = &item.message;
    let meta = if message.edited { format!("edited {}", item.time) } else { item.time.clone() };
    let views = if message.views > 0 { label::count(message.views) } else { String::new() };
    let mut w = p.layout(&meta, theme::TINY, None).width();
    if !views.is_empty() {
        w += p.layout(&views, theme::TINY, None).width() + 24.0;
    }
    if message.pinned {
        w += 15.0;
    }
    if message.outgoing {
        w += 19.0;
    }
    (meta, views, w)
}

/// Measures a poll in a column `width` wide: its parts, and its height.
fn lay_poll(p: &Painter<'_>, poll: &Poll, width: f32) -> (Media, f32) {
    let question = TextLayout::new(p.fonts, &poll.question, theme::LABEL, Some(width), p.scale);
    let mut y = question.height() + 22.0;
    let options = poll
        .options
        .iter()
        .map(|option| {
            let words = TextLayout::new(p.fonts, &option.text, theme::SMALL, Some((width - 44.0).max(40.0)), p.scale);
            let h = (words.height() + 16.0).max(34.0);
            let at = y;
            y += h;
            (words, at, h)
        })
        .collect();
    (Media::Poll { question, options }, y + 22.0)
}

/// Measures the card of the page a link leads to, `width` wide at most, starting at `y`.
fn lay_link(p: &Painter<'_>, preview: &LinkPreview, y: f32, width: f32) -> LinkLaid {
    let room = (width - 22.0).max(40.0);
    let mut h = 8.0 + 17.0;
    let mut w: f32 = p.layout(&preview.site, theme::CAPTION, None).width().min(room);
    let title = (!preview.title.is_empty()).then(|| {
        let mut title = p.layout(&preview.title, theme::LABEL, None);
        title.truncate(p.fonts, room);
        h += 19.0;
        w = w.max(title.width());
        title
    });
    let about = (!preview.description.is_empty()).then(|| {
        // A few lines of it: the page itself has the rest.
        let short: String = preview.description.chars().take(150).collect();
        let about = TextLayout::new(p.fonts, &short, theme::TINY, Some(room), p.scale);
        h += about.height() + 3.0;
        w = w.max(about.width());
        about
    });
    let photo = preview.photo.filter(|_| preview.width > 0 && preview.height > 0).map(|_| {
        let scale = (room / preview.width as f32).min(LINK_MAX.1 / preview.height as f32).min(1.0);
        let size =
            ((preview.width as f32 * scale).round().max(60.0), (preview.height as f32 * scale).round().max(40.0));
        h += size.1 + 7.0;
        w = w.max(size.0);
        size
    });
    LinkLaid { y, w: (w + 22.0).min(width), h: h + 8.0, title, about, photo }
}

/// What a bubble is drawn with besides the row.
pub struct Scene<'a> {
    /// Where files are, and the ones this frame misses.
    pub files: (&'a HashMap<FileId, String>, &'a mut Vec<FileId>),
    /// The files that were asked for and have not come.
    pub fetching: &'a HashSet<FileId>,
    /// The message it answers, if it is known.
    pub replied: Option<&'a Message>,
    /// The other side has read it (the user's own).
    pub read: bool,
    /// Where the mouse is, if it is over this bubble's row and nothing covers it.
    pub mouse: Option<(f32, f32)>,
    /// The bytes of its words the user picked out, and whether the pick runs on past them.
    pub marked: Option<(std::ops::Range<usize>, bool)>,
}

/// Draws the bubble `laid` of the row `item` with its top-left corner at
/// `at`. Returns what the mouse is over.
#[expect(clippy::many_single_char_names, reason = "geometry: the painter, the theme, and x, y, w, h")]
pub fn draw(p: &mut Painter<'_>, laid: &Laid, item: &Item, at: (f32, f32), scene: Scene<'_>) -> Over {
    let t = p.theme;
    let message = &item.message;
    let bubble = laid.body(at);
    let mut over = Over::Nothing;
    let inside = |rect: Rect| scene.mouse.is_some_and(|mouse| rect.contains(mouse));
    let (files, want) = scene.files;
    let mut path_of = |file: Option<FileId>| {
        let path = file.and_then(|file| files.get(&file));
        if path.is_none() {
            want.extend(file);
        }
        path
    };
    let bare = matches!(laid.media, Media::Bare(..));
    if bare {
        // A sticker or a round video: the picture alone.
    } else if message.outgoing {
        p.rect(bubble, t.active, theme::RADIUS_LG);
    } else {
        p.bordered(bubble, t.surface, theme::RADIUS_LG, 1.0, t.border);
    }
    let (x, inner_w) = (bubble.x + PAD_X, bubble.w - 2.0 * PAD_X);
    let mut y = bubble.y + PAD_Y;
    if laid.name {
        let mut name = p.layout(&message.sender_name, theme::CAPTION, None);
        name.truncate(p.fonts, inner_w);
        p.text(&name, x, y, theme::name_color(&message.sender_name));
        y += NAME_H;
    }
    if !message.forwarded_from.is_empty() && !bare {
        icon::draw(p, icon::FORWARD, (x + 6.0, y + 8.0), 12.0, t.text_faint);
        let mut from = p.layout(&format!("Forwarded from {}", message.forwarded_from), theme::TINY, None);
        from.truncate(p.fonts, inner_w - 18.0);
        p.text(&from, x + 18.0, y - 1.0, t.text_muted);
        y += FORWARD_H;
    }
    if laid.reply {
        let block = Rect::new(x, y + 1.0, inner_w, REPLY_H - 7.0);
        p.rect(block, fade(t.text, 0.06), 6.0);
        p.rect(Rect::new(block.x, block.y, 2.5, block.h), t.text_muted, 1.0);
        let (who, what) = match scene.replied {
            Some(replied) => (replied.sender_name.as_str(), replied.body.preview()),
            None => ("", "An earlier message".to_owned()),
        };
        let who = if scene.replied.is_some_and(|replied| replied.outgoing) { "You" } else { who };
        let mut name = p.layout(who, theme::CAPTION, None);
        name.truncate(p.fonts, block.w - 18.0);
        p.text(&name, block.x + 10.0, block.y + 3.0, if who.is_empty() { t.text_faint } else { t.text });
        let mut what = p.layout(&what, theme::TINY, None);
        what.truncate(p.fonts, block.w - 18.0);
        p.text(&what, block.x + 10.0, block.y + if who.is_empty() { 9.0 } else { 18.0 }, t.text_muted);
        if inside(block) {
            over = Over::Reply;
        }
    }

    let media_y = bubble.y + laid.media_y;
    let radius = theme::RADIUS_LG - PHOTO_INSET;
    match (&message.body, &laid.media) {
        (_, Media::Album(cells)) => {
            for (index, (cell, part)) in cells.iter().zip(item.messages()).enumerate() {
                let rect = Rect::new(bubble.x + PHOTO_INSET + cell.x, media_y + cell.y, cell.w, cell.h);
                let (still, play) = match &part.body {
                    Body::Photo { file, .. } => (*file, false),
                    Body::Video { still, .. } => (*still, true),
                    _ => (None, false),
                };
                visual(p, rect, 6.0, path_of(still), play.then_some(icon::PLAY));
                if inside(rect) {
                    over = Over::Media(index);
                }
            }
        }
        (Body::Photo { file, .. }, Media::Visual(w, h)) => {
            let rect = Rect::new(bubble.x + PHOTO_INSET, media_y, *w, *h);
            if visual(p, rect, radius, path_of(*file), None) && inside(rect) {
                over = Over::Media(0);
            }
        }
        (Body::Video { still, file, motion, seconds, .. }, Media::Visual(w, h)) => {
            let rect = Rect::new(bubble.x + PHOTO_INSET, media_y, *w, *h);
            let waiting = file.is_some_and(|file| scene.fetching.contains(&file) && !files.contains_key(&file));
            visual(p, rect, radius, path_of(*still), Some(if waiting { icon::CLOCK } else { icon::PLAY }));
            let length = if *motion == Motion::Gif { "GIF".to_owned() } else { label::duration(*seconds) };
            let length = p.layout(&length, theme::TINY, None);
            p.rect(Rect::new(rect.x + 7.0, rect.y + 7.0, length.width() + 12.0, 19.0), t.scrim, 9.5);
            p.text(&length, rect.x + 13.0, rect.y + 7.0 + (19.0 - length.height()) * 0.5, t.on_scrim);
            if inside(rect) {
                over = Over::Media(0);
            }
        }
        (Body::Video { still, file, .. }, Media::Bare(w, _)) => {
            let rect = Rect::new(bubble.x, bubble.y, *w, *w);
            let waiting = file.is_some_and(|file| scene.fetching.contains(&file) && !files.contains_key(&file));
            visual(p, rect, w * 0.5, path_of(*still), Some(if waiting { icon::CLOCK } else { icon::PLAY }));
            if inside(rect) {
                over = Over::Media(0);
            }
        }
        (Body::Sticker { file, emoji, .. }, Media::Bare(w, h)) => {
            let rect = Rect::new(bubble.x, bubble.y, *w, *h);
            let shown = path_of(*file).is_some_and(|path| matches!(p.image(path, rect, 0.0), Lookup::Ready(_)));
            if !shown {
                // Until the picture is there (or where it moves and cannot be shown): the emoji it stands for.
                let style = adelite_ui::text::Style::regular((h * 0.55).round());
                p.label_centered(emoji, style, rect, t.text);
            }
        }
        (Body::Audio { file, title, voice, seconds, .. }, Media::Audio) => {
            let row = Rect::new(x, media_y, inner_w, FILE_H - 6.0);
            let on_disk = file.is_some_and(|file| files.contains_key(&file));
            let knob = Rect::new(row.x, row.y + 3.0, 36.0, 36.0);
            p.rect(knob, t.accent, 18.0);
            let glyph = match file {
                Some(file) if !on_disk && scene.fetching.contains(file) => icon::CLOCK,
                _ if *voice => icon::MIC,
                _ => icon::MUSIC,
            };
            icon::draw(p, glyph, (knob.x + 18.0, knob.y + 18.0), 16.0, t.on_accent);
            let mut name = p.layout(if *voice { "Voice message" } else { title }, theme::LABEL, None);
            name.truncate(p.fonts, row.w - 46.0);
            p.text(&name, row.x + 46.0, row.y + 3.0, t.text);
            let state = if on_disk { "Play" } else { "Download" };
            let mut detail = p.layout(&format!("{} · {state}", label::duration(*seconds)), theme::TINY, None);
            detail.truncate(p.fonts, row.w - 46.0);
            p.text(&detail, row.x + 46.0, row.y + 22.0, t.text_faint);
            if file.is_some() && inside(row) {
                over = Over::Media(0);
            }
        }
        (Body::File { file, name, size, .. }, Media::File) => {
            let row = Rect::new(x, media_y, inner_w, FILE_H - 6.0);
            // A file may be large: it is fetched when asked for, not when it scrolls by.
            let on_disk = file.is_some_and(|file| files.contains_key(&file));
            let badge = Rect::new(row.x, row.y + 3.0, 36.0, 36.0);
            p.bordered(badge, t.bg, 10.0, 1.0, t.border_strong);
            let glyph = if on_disk { icon::FILE } else { icon::DOWNLOAD };
            icon::draw(p, glyph, (badge.x + 18.0, badge.y + 18.0), 16.0, t.text_muted);
            let mut name = p.layout(name, theme::LABEL, None);
            name.truncate(p.fonts, row.w - 46.0);
            p.text(&name, row.x + 46.0, row.y + 3.0, t.text);
            let state = match file {
                _ if on_disk => "Show in folder",
                Some(file) if scene.fetching.contains(file) => "Downloading…",
                Some(_) => "Download",
                None => "Not available",
            };
            let mut detail = p.layout(&format!("{} · {state}", label::file_size(*size)), theme::TINY, None);
            detail.truncate(p.fonts, row.w - 46.0);
            p.text(&detail, row.x + 46.0, row.y + 22.0, t.text_faint);
            if file.is_some() && inside(row) {
                over = Over::Media(0);
            }
        }
        (Body::Other { label, .. }, Media::Other) => {
            icon::draw(p, icon::PAPERCLIP, (x + 7.0, media_y + 11.0), 14.0, t.text_faint);
            let mut label = p.layout(label, theme::SMALL, None);
            label.truncate(p.fonts, inner_w - 22.0);
            p.text(&label, x + 22.0, media_y + 1.0, t.text_muted);
        }
        (Body::Poll(poll), Media::Poll { question, options }) => {
            if let Some(answer) = draw_poll(p, poll, (question, options), (x, media_y, inner_w), scene.mouse) {
                over = Over::Answer(answer);
            }
        }
        _ => {}
    }

    if let Some(layout) = &laid.text {
        let origin = (x, bubble.y + laid.text_y);
        if let Some((range, on)) = &scene.marked {
            let line = layout.line_height();
            for (x0, x1, top) in layout.selection_spans(range.start, range.end, *on) {
                p.rect(Rect::new(origin.0 + x0, origin.1 + top, x1 - x0, line), t.selection, 2.0);
            }
        }
        // Hidden words have no ink until they are asked for; a quotation is quieter than what is said.
        p.rich(layout, origin.0, origin.1, &[t.text, t.link, fade(t.text, 0.0), t.text_muted], t.code_bg);
        let span = scene.mouse.and_then(|mouse| layout.link_at(mouse.0 - origin.0, mouse.1 - origin.1));
        if let Some(span) = span {
            over = Over::Span(usize::from(span));
        }
    }
    if let (Some(link), Some(preview)) = (&laid.link, &message.link) {
        let card = Rect::new(x, bubble.y + link.y, inner_w, link.h);
        p.rect(card, fade(t.text, 0.05), 6.0);
        p.rect(Rect::new(card.x, card.y, 2.5, card.h), t.text_muted, 1.0);
        let (left, mut top) = (card.x + 12.0, card.y + 8.0);
        let mut site = p.layout(&preview.site, theme::CAPTION, None);
        site.truncate(p.fonts, card.w - 22.0);
        p.text(&site, left, top, t.link);
        top += 17.0;
        if let Some(title) = &link.title {
            p.text(title, left, top, t.text);
            top += 19.0;
        }
        if let Some(about) = &link.about {
            p.text(about, left, top, t.text_muted);
            top += about.height() + 3.0;
        }
        if let Some((w, h)) = link.photo {
            let rect = Rect::new(left, top + 4.0, w, h);
            let shown = path_of(preview.photo).is_some_and(|path| matches!(p.image(path, rect, 6.0), Lookup::Ready(_)));
            if !shown {
                p.rect(rect, t.hover, 6.0);
            }
        }
        if inside(card) {
            over = Over::Preview;
        }
    }
    for (index, (reaction, (cx, cy, w))) in message.reactions.iter().zip(&laid.chips).enumerate() {
        let chip = Rect::new(x + cx, bubble.y + cy, *w, CHIP.0);
        let hovered = inside(chip);
        let fill = match (reaction.chosen, hovered) {
            (true, _) => mix(fade(t.text, 0.08), t.accent, 0.22),
            (false, true) => fade(t.text, 0.12),
            (false, false) => fade(t.text, 0.07),
        };
        p.bordered(
            chip,
            fill,
            CHIP.0 * 0.5,
            1.0,
            if reaction.chosen { fade(t.accent, 0.55) } else { fade(t.text, 0.0) },
        );
        let emoji = p.layout(&reaction.emoji, theme::SMALL, None);
        p.text(&emoji, chip.x + 8.0, chip.y + (chip.h - emoji.height()) * 0.5, t.text);
        let count = p.layout(&label::count(reaction.count), theme::CAPTION, None);
        let ink = if reaction.chosen { t.text } else { t.text_muted };
        p.text(&count, chip.x + 12.0 + emoji.width(), chip.y + (chip.h - count.height()) * 0.5, ink);
        if hovered {
            over = Over::Reaction(index);
        }
    }

    // The time, how often it was seen, and how far the user's own message got.
    let over_picture = laid.meta_over;
    let foot = bubble.bottom() - laid.comments_y.map_or(0.0, |_| COMMENTS_H);
    let right = bubble.right() - if over_picture { PHOTO_INSET + 8.0 } else { PAD_X };
    let top = foot - if over_picture { PHOTO_INSET + 23.0 } else { PAD_Y + 17.0 };
    let ink = if over_picture {
        p.rect(Rect::new(right - laid.meta_w - 7.0, top - 1.0, laid.meta_w + 14.0, 20.0), t.scrim, 10.0);
        t.on_scrim
    } else {
        t.text_faint
    };
    let mut pen = right - laid.meta_w;
    if message.pinned {
        icon::draw(p, icon::PIN, (pen + 6.0, top + 9.0), 11.0, ink);
        pen += 15.0;
    }
    if !laid.views.is_empty() {
        icon::draw(p, icon::EYE, (pen + 6.0, top + 9.0), 12.0, ink);
        pen += 16.0 + p.label(&laid.views, theme::TINY, pen + 16.0, top, ink) + 8.0;
    }
    p.label(&laid.meta, theme::TINY, pen, top, ink);
    if message.outgoing {
        let (glyph, color) = match message.delivery {
            Delivery::Sending => (icon::CLOCK, ink),
            Delivery::Failed => (icon::ALERT, t.danger),
            Delivery::Sent if scene.read => (icon::CHECK_CHECK, if over_picture { ink } else { t.text_muted }),
            Delivery::Sent => (icon::CHECK, ink),
        };
        icon::draw(p, glyph, (right - 6.5, top + 9.0), 13.0, color);
    }

    if let (Some(at), Some(count)) = (laid.comments_y, message.comments) {
        let row = Rect::new(bubble.x, bubble.y + at, bubble.w, COMMENTS_H);
        p.rect(Rect::new(row.x + 1.0, row.y, row.w - 2.0, 1.0), t.border, 0.0);
        let words = match count {
            0 => "Leave a comment".to_owned(),
            1 => "1 comment".to_owned(),
            count => format!("{} comments", label::count(count)),
        };
        icon::draw(p, icon::COMMENT, (row.x + PAD_X + 7.0, row.y + row.h * 0.5), 14.0, t.link);
        p.label(&words, theme::CAPTION, row.x + PAD_X + 21.0, row.y + 10.0, t.link);
        icon::draw(p, icon::CHEVRON_RIGHT, (row.right() - PAD_X - 5.0, row.y + row.h * 0.5), 13.0, t.text_faint);
        if inside(row) {
            over = Over::Comments;
        }
    }
    let mut y = bubble.bottom() + BUTTON.1;
    for (at, buttons) in message.buttons.iter().enumerate() {
        let w = (bubble.w - BUTTON.1 * (buttons.len().max(1) as f32 - 1.0)) / buttons.len().max(1) as f32;
        for (index, button) in buttons.iter().enumerate() {
            let rect = Rect::new(bubble.x + index as f32 * (w + BUTTON.1), y, w, BUTTON.0);
            let hovered = inside(rect) && button.press != Press::Nothing;
            p.bordered(rect, if hovered { t.hover } else { t.surface }, theme::RADIUS_SM, 1.0, t.border);
            let mut words = p.layout(&button.label, theme::SMALL, None);
            words.truncate(p.fonts, rect.w - 16.0);
            let ink = if button.press == Press::Nothing { t.text_faint } else { t.text };
            p.text(&words, rect.x + (rect.w - words.width()) * 0.5, rect.y + (rect.h - words.height()) * 0.5, ink);
            if matches!(button.press, Press::Link(_)) {
                icon::draw(p, icon::EXTERNAL, (rect.right() - 9.0, rect.y + 9.0), 9.0, t.text_faint);
            }
            if hovered {
                over = Over::Button(at, index);
            }
        }
        y += BUTTON.0 + BUTTON.1;
    }
    over
}

/// A picture in `rect`, or a quiet block until it is there; with `badge`,
/// a mark at its middle that says it plays. Returns whether the picture is shown.
fn visual(p: &mut Painter<'_>, rect: Rect, radius: f32, path: Option<&String>, badge: Option<char>) -> bool {
    let t = p.theme;
    let state = path.map_or(Lookup::Loading, |path| p.image(path, rect, radius));
    let shown = matches!(state, Lookup::Ready(_));
    let mid = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    if !shown {
        // On its way: a quiet block the picture takes the place of. Only one that cannot be shown says so.
        p.rect(rect, t.hover, radius);
        if badge.is_none() && state == Lookup::Failed {
            icon::draw(p, icon::IMAGE, mid, 22.0, t.text_faint);
        }
    }
    if let Some(glyph) = badge {
        let side = 44.0_f32.min(rect.w.min(rect.h) * 0.6);
        p.rect(Rect::new(mid.0 - side * 0.5, mid.1 - side * 0.5, side, side), t.scrim, side * 0.5);
        icon::draw(p, glyph, mid, side * 0.42, t.on_scrim);
    }
    shown
}

/// Draws a poll in the column at `(x, y)`, `w` wide. Returns the answer the mouse is over, where one can still be given.
#[expect(clippy::many_single_char_names, reason = "geometry: the painter, the theme, and x, y, w, h")]
fn draw_poll(
    p: &mut Painter<'_>,
    poll: &Poll,
    (question, options): (&TextLayout, &[(TextLayout, f32, f32)]),
    (x, y, w): (f32, f32, f32),
    mouse: Option<(f32, f32)>,
) -> Option<usize> {
    let t = p.theme;
    p.text(question, x, y, t.text);
    let kind = match (poll.quiz, poll.multiple) {
        (true, _) => "Quiz",
        (false, true) => "Poll · several answers",
        (false, false) => "Poll",
    };
    p.label(kind, theme::TINY, x, y + question.height() + 2.0, t.text_faint);
    let open = !poll.closed && !poll.answered();
    let mut over = None;
    for (index, (option, (words, at, h))) in poll.options.iter().zip(options).enumerate() {
        let row = Rect::new(x - 4.0, y + at, w + 8.0, *h);
        let hovered = open && mouse.is_some_and(|mouse| row.contains(mouse));
        if hovered {
            p.rect(row, fade(t.text, 0.06), theme::RADIUS_SM);
            over = Some(index);
        }
        let mid = row.y + row.h * 0.5;
        if open {
            icon::draw(p, icon::CIRCLE, (x + 9.0, mid), 15.0, if hovered { t.text } else { t.text_faint });
        } else {
            let percent = p.layout(&format!("{}%", option.percent), theme::CAPTION, None);
            p.text(&percent, x + 30.0 - percent.width(), mid - percent.height() * 0.5 - 2.0, t.text);
            // How many picked it, as a bar under its words.
            let track = Rect::new(x + 40.0, row.bottom() - 6.0, w - 40.0, 3.0);
            p.rect(track, fade(t.text, 0.08), 1.5);
            let share = f32::from(option.percent.min(100)) / 100.0;
            let fill = if option.chosen { t.accent } else { t.text_muted };
            p.rect(Rect::new(track.x, track.y, (track.w * share).max(3.0), 3.0), fill, 1.5);
            if option.chosen {
                icon::draw(p, icon::CHECK, (x + w - 6.0, mid - 2.0), 13.0, t.text);
            }
        }
        p.text(words, x + 40.0, row.y + 6.0, t.text);
    }
    let foot = options.last().map_or(question.height() + 22.0, |(_, at, h)| at + h);
    let voters = match poll.voters {
        0 => "No answers yet".to_owned(),
        1 => "1 answer".to_owned(),
        count => format!("{} answers", label::count(count)),
    };
    let voters = if poll.closed { format!("{voters} · closed") } else { voters };
    p.label(&voters, theme::TINY, x, y + foot + 5.0, t.text_faint);
    over
}

/// A line the chat says itself ("Mara joined"), or a day between messages:
/// small words on a pill, centred in `row`.
pub fn pill(p: &mut Painter<'_>, row: Rect, words: &str) {
    let t = p.theme;
    let mut words = p.layout(words, theme::TINY, None);
    words.truncate(p.fonts, row.w - 40.0);
    let pill = Rect::new(
        (row.x + (row.w - words.width()) * 0.5 - 10.0).round(),
        row.y + (row.h - 22.0) * 0.5,
        words.width() + 20.0,
        22.0,
    );
    p.rect(pill, fade(t.text, 0.05), 11.0);
    p.text(&words, pill.x + 10.0, pill.y + (pill.h - words.height()) * 0.5, t.text_muted);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_album_fills_its_block_in_rows_of_two_or_three() {
        for count in 2..=10 {
            let (cells, h) = album(count, 352.0);
            assert_eq!(cells.len(), count);
            let rows: Vec<f32> = cells.iter().map(|cell| cell.y).collect();
            assert!(rows.windows(2).all(|pair| pair[0] <= pair[1]), "{count}: cells run row by row");
            for cell in &cells {
                assert!(cell.x >= 0.0 && cell.right() <= 353.0 && cell.bottom() <= h + 0.5, "{count}: {cell:?}");
            }
            // Every row ends at the block's right edge.
            let last = cells.last().map_or(0.0, |cell| cell.right());
            assert!((last - 352.0).abs() < 2.0, "{count}: the last cell ends at {last}");
        }
        assert_eq!(album(5, 300.0).0.iter().filter(|cell| cell.y < 1.0).count(), 2, "the pair comes first");
        assert_eq!(album(3, 300.0).0.iter().filter(|cell| cell.y < 1.0).count(), 1, "one large picture over a pair");
        assert_eq!(album(7, 300.0).0.iter().filter(|cell| cell.y < 1.0).count(), 2);
        // A picture never grows past its pixels, and one without a size still has a place.
        assert_eq!(fit(100, 80, 540.0), (100.0, 80.0));
        assert_eq!(fit(0, 0, 540.0), (320.0, 200.0));
        assert!(fit(4000, 3000, 540.0).0 <= PHOTO_MAX.0);
    }
}
