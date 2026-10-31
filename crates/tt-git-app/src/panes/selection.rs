//! Text picked out with the mouse in the details pane or the error popup,
//! for the front ends that draw it (the GUI) to copy.
//!
//! A selection is kept by line and column of the text it was made in, not
//! by screen row, so it stays put as the text scrolls. It holds on to that
//! text: once the pane shows other text, it no longer applies.

use std::ops::Range;
use std::sync::Arc;

use unicode_width::UnicodeWidthChar;

/// A place in the text: a line and the column before which it falls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPos {
    pub line: usize,
    pub col: usize,
}

/// From where the mouse went down (`anchor`) to where it is (`head`).
#[derive(Debug, Clone)]
pub struct TextSelection {
    lines: Arc<Vec<String>>,
    anchor: TextPos,
    head: TextPos,
    /// The button is still down: moves extend it.
    pub dragging: bool,
}

impl TextSelection {
    /// An empty selection at `at` in `lines`, the mouse down.
    pub fn new(lines: Arc<Vec<String>>, at: TextPos) -> Self {
        Self {
            lines,
            anchor: at,
            head: at,
            dragging: true,
        }
    }

    /// Moves the end that follows the mouse to `at`.
    pub fn extend(&mut self, at: TextPos) {
        self.head = at;
    }

    /// Whether it was made in `lines`, the text now shown.
    pub fn is_for(&self, lines: &Arc<Vec<String>>) -> bool {
        Arc::ptr_eq(&self.lines, lines)
    }

    /// Whether it was made in `text`, the lines of a message rather than a
    /// pane's: the same words count.
    pub fn is_of(&self, text: &str) -> bool {
        self.lines.iter().map(String::as_str).eq(text.lines())
    }

    /// Whether it picks out nothing (a click without a drag).
    pub fn is_empty(&self) -> bool {
        let (start, end) = self.bounds();
        start == end
    }

    /// The two ends, first one first, each on a character boundary.
    fn bounds(&self) -> (Range<usize>, Range<usize>) {
        let (start, end) = if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        let at = |p: TextPos| {
            let line = p.line.min(self.lines.len().saturating_sub(1));
            let byte = self.lines.get(line).map_or(0, |l| byte_at(l, p.col));
            line..byte
        };
        (at(start), at(end))
    }

    /// The bytes of line `line` it covers, if any. A line it runs past the
    /// end of is covered to its end.
    pub fn bytes_on(&self, line: usize) -> Option<Range<usize>> {
        let (start, end) = self.bounds();
        if line < start.start || line > end.start || start == end {
            return None;
        }
        let len = self.lines.get(line).map_or(0, String::len);
        let from = if line == start.start { start.end } else { 0 };
        let to = if line == end.start { end.end } else { len };
        (from < to).then_some(from..to)
    }

    /// What it picks out, its lines joined with `\n`.
    pub fn text(&self) -> String {
        let (start, end) = self.bounds();
        (start.start..=end.start)
            .filter_map(|line| {
                let text = self.lines.get(line)?;
                let from = if line == start.start { start.end } else { 0 };
                let to = if line == end.start { end.end } else { text.len() };
                Some(&text[from..to.max(from)])
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The byte in `line` where column `col` starts: a character it falls in
/// the middle of is left out, and a column past the end is the end.
fn byte_at(line: &str, col: usize) -> usize {
    let mut width = 0;
    for (at, ch) in line.char_indices() {
        let w = ch.width().unwrap_or(0);
        if width + w > col {
            return at;
        }
        width += w;
    }
    line.len()
}

/// The column byte `byte` of `line` starts at: the other way from
/// [`byte_at`], for a front end that hit-tests by byte.
pub fn col_at(line: &str, byte: usize) -> usize {
    line[..byte].chars().map(|ch| ch.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Arc<Vec<String>> {
        Arc::new(text.lines().map(str::to_string).collect())
    }

    fn pos(line: usize, col: usize) -> TextPos {
        TextPos { line, col }
    }

    #[test]
    fn a_selection_within_a_line() {
        let text = lines("✗ plugin x: boom\nsecond");
        let mut sel = TextSelection::new(text.clone(), pos(0, 2));
        assert!(sel.is_empty() && sel.is_for(&text));
        assert!(
            !sel.is_for(&lines("✗ plugin x: boom\nsecond")),
            "other text, same words"
        );
        sel.extend(pos(0, 8));
        assert_eq!(sel.text(), "plugin");
        // ✗ is three bytes but one column.
        assert_eq!(sel.bytes_on(0), Some(4..10));
        assert_eq!(sel.bytes_on(1), None);
    }

    #[test]
    fn a_selection_dragged_back_up_over_lines() {
        let text = lines("one\ntwo\nthree");
        let mut sel = TextSelection::new(text, pos(2, 2));
        sel.extend(pos(0, 1));
        assert_eq!(sel.text(), "ne\ntwo\nth");
        assert_eq!(sel.bytes_on(0), Some(1..3));
        assert_eq!(sel.bytes_on(1), Some(0..3));
        assert_eq!(sel.bytes_on(2), Some(0..2));
    }

    #[test]
    fn past_the_end_is_the_end() {
        let text = lines("short\nlonger line");
        let mut sel = TextSelection::new(text, pos(0, 40));
        sel.extend(pos(9, 99));
        assert_eq!(sel.text(), "\nlonger line");
        assert_eq!(sel.bytes_on(0), None, "nothing of the first line");
        assert_eq!(sel.bytes_on(1), Some(0..11));
    }

    #[test]
    fn wide_characters_are_whole_or_not_at_all() {
        assert_eq!(byte_at("日本語", 0), 0);
        assert_eq!(byte_at("日本語", 1), 0, "inside the first character");
        assert_eq!(byte_at("日本語", 2), 3);
        assert_eq!(byte_at("日本語", 7), 9);
    }

    #[test]
    fn a_byte_has_a_column_and_back() {
        assert_eq!(col_at("✗ plugin", 4), 2, "✗ is three bytes but one column");
        assert_eq!(col_at("日本語", 6), 4);
        assert_eq!(byte_at("日本語", col_at("日本語", 6)), 6);
    }

    #[test]
    fn a_selection_knows_the_message_by_its_words() {
        let sel = TextSelection::new(lines("one\ntwo"), pos(0, 0));
        assert!(sel.is_of("one\ntwo"));
        assert!(!sel.is_of("one\ntwo\nthree"));
        assert!(!sel.is_of("one"));
    }
}
