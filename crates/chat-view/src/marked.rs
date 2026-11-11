//! Words picked out of the conversation with the mouse: dragged over, a
//! word by a double click, a whole message by a triple one. A pick may run
//! over several messages; each end is a message and a byte in its words.

use std::ops::Range;

use adelite_chat::MessageId;

use crate::bubble;
use crate::store::Thread;

/// One end of a pick: the row (by its first message) and a byte in its words.
pub type End = (MessageId, usize);

/// Words picked with the mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marked {
    /// Where the pick began.
    pub anchor: End,
    /// Where it ends now: under the mouse while it is dragged.
    pub focus: End,
    /// The button is still held: the focus follows the mouse.
    pub dragging: bool,
    /// The mouse has gone far enough from where it was pressed for the drag to count.
    pub moved: bool,
}

impl Marked {
    /// A pick of `range` of the words of `row`, held by the button.
    #[must_use]
    pub fn new(row: MessageId, range: Range<usize>) -> Self {
        Self { anchor: (row, range.start), focus: (row, range.end), dragging: true, moved: false }
    }

    /// Nothing is picked yet: a click, not a drag.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }

    /// Both ends as `(row, byte)` in the order they come in `thread`, or
    /// `None` when either row is gone.
    #[must_use]
    pub fn ordered(&self, thread: &Thread) -> Option<((usize, usize), (usize, usize))> {
        let a = (thread.find(self.anchor.0)?, self.anchor.1);
        let f = (thread.find(self.focus.0)?, self.focus.1);
        Some(if a <= f { (a, f) } else { (f, a) })
    }
}

/// What of the words of the row at `row` (`len` bytes long) lies between
/// `from` and `to`: the range, and whether the pick runs on past it.
#[must_use]
pub fn part(from: (usize, usize), to: (usize, usize), row: usize, len: usize) -> Option<(Range<usize>, bool)> {
    if row < from.0 || row > to.0 {
        return None;
    }
    let start = if row == from.0 { from.1.min(len) } else { 0 };
    let end = if row == to.0 { to.1.min(len) } else { len };
    (start < end || (row < to.0 && start <= end)).then_some((start..end, row < to.0))
}

/// The picked words of `thread` between `from` and `to`, a blank line between messages.
#[must_use]
pub fn text(thread: &Thread, from: (usize, usize), to: (usize, usize)) -> String {
    let mut out = String::new();
    for row in from.0..=to.0 {
        let Some(words) = thread.items.get(row).and_then(bubble::words) else { continue };
        let Some((range, _)) = part(from, to, row, words.text.len()) else { continue };
        let Some(piece) = words.text.get(range) else { continue };
        if piece.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(piece);
    }
    out
}

/// The word around byte `at` of `text`: letters, digits and underscores;
/// on anything else, that one character.
#[must_use]
pub fn word_at(text: &str, at: usize) -> Range<usize> {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let start =
        text[..at].rfind(|c: char| !is_word(c)).map_or(0, |i| i + text[i..].chars().next().map_or(1, char::len_utf8));
    let end = text[at..].find(|c: char| !is_word(c)).map_or(text.len(), |i| at + i);
    if start < end {
        return start..end;
    }
    // Not in a word: the character there, or the one before at the very end.
    match text[at..].chars().next() {
        Some(c) => at..at + c.len_utf8(),
        None => text[..at].chars().next_back().map_or(at..at, |c| at - c.len_utf8()..at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_is_letters_and_digits() {
        let text = "Hello, wörld_2 !";
        assert_eq!(&text[word_at(text, 2)], "Hello");
        assert_eq!(&text[word_at(text, 0)], "Hello");
        assert_eq!(&text[word_at(text, 9)], "wörld_2");
        // Just after a word, still that word; between words, the space.
        assert_eq!(&text[word_at(text, 5)], "Hello");
        assert_eq!(&text[word_at(text, 6)], " ");
        assert_eq!(&text[word_at(text, text.len())], "!");
        // Inside a character: the word it belongs to.
        assert_eq!(&text[word_at(text, 9)], &text[word_at(text, 10)]);
        assert_eq!(word_at("", 0), 0..0);
    }

    #[test]
    fn a_pick_over_rows_takes_each_its_share() {
        let (from, to) = ((2, 4), (4, 3));
        assert_eq!(part(from, to, 1, 10), None);
        assert_eq!(part(from, to, 2, 10), Some((4..10, true)));
        assert_eq!(part(from, to, 3, 7), Some((0..7, true)));
        assert_eq!(part(from, to, 4, 10), Some((0..3, false)));
        assert_eq!(part(from, to, 5, 10), None);
        // Within one row; nothing when both ends meet.
        assert_eq!(part((2, 1), (2, 5), 2, 10), Some((1..5, false)));
        assert_eq!(part((2, 5), (2, 5), 2, 10), None);
        // Ends past the words are held to them.
        assert_eq!(part((2, 1), (2, 50), 2, 10), Some((1..10, false)));
    }

    #[test]
    fn a_pick_is_empty_until_its_ends_part() {
        let mut marked = Marked::new(7, 3..3);
        assert!(marked.is_empty() && marked.dragging);
        marked.focus = (8, 0);
        assert!(!marked.is_empty());
    }
}
