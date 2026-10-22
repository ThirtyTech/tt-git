//! Small helpers shared by the panes.

use std::borrow::Cow;
use std::ops::Range;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::keymap::Command;
use crate::syntax::Tokens;
use crate::theme::Theme;

/// Cursor and scroll offset of a list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    pub selected: usize,
    pub offset: usize,
    /// Visible rows as of the last draw (used for paging).
    pub height: usize,
}

impl Cursor {
    /// Applies a movement command to a list of `len` items. Returns whether
    /// it was a movement command.
    pub fn apply(&mut self, cmd: Command, len: usize) -> bool {
        let page = self.height.max(1);
        let last = len.saturating_sub(1);
        self.selected = match cmd {
            Command::MoveUp => self.selected.saturating_sub(1),
            Command::MoveDown => (self.selected + 1).min(last),
            Command::PageUp => self.selected.saturating_sub(page),
            Command::PageDown => (self.selected + page).min(last),
            Command::Top => 0,
            Command::Bottom => last,
            _ => return false,
        };
        true
    }

    /// Selects the item shown on visible row `row`, if there is one.
    pub fn click(&mut self, row: usize, len: usize) -> bool {
        let idx = self.offset + row;
        if idx < len {
            self.selected = idx;
            true
        } else {
            false
        }
    }

    pub fn clamp(&mut self, len: usize) {
        self.selected = self.selected.min(len.saturating_sub(1));
    }

    /// Adjusts the offset so the selection is within `height` rows.
    pub fn scroll_into_view(&mut self, height: usize) {
        self.height = height;
        if height == 0 {
            return;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + height {
            self.offset = self.selected + 1 - height;
        }
    }
}

/// Truncates `s` to `width` display columns, ending with `…` when cut.
pub fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Like [`truncate`] for differently styled pieces of one line: cuts at
/// `width` columns, ending with `…` in the style of the piece it cut.
pub fn truncate_spans(parts: Vec<(String, Style)>, width: usize) -> Line<'static> {
    let total: usize = parts.iter().map(|(t, _)| t.width()).sum();
    if total <= width {
        return Line::from(parts.into_iter().map(|(t, s)| Span::styled(t, s)).collect::<Vec<_>>());
    }
    let mut spans = Vec::new();
    let mut left = width;
    for (text, style) in parts {
        if text.width() < left {
            left -= text.width();
            spans.push(Span::styled(text, style));
        } else {
            // Something comes after this piece even when it fits exactly,
            // so it always ends with `…`.
            spans.push(Span::styled(truncate(&format!("{text}…"), left), style));
            break;
        }
    }
    Line::from(spans)
}

/// Pads or truncates `s` to exactly `width` columns.
pub fn fit(s: &str, width: usize) -> String {
    let t = truncate(s, width);
    let pad = width.saturating_sub(t.width());
    format!("{t}{}", " ".repeat(pad))
}

/// Right-aligns `s` in `width` columns.
pub fn fit_right(s: &str, width: usize) -> String {
    let t = truncate(s, width);
    let pad = width.saturating_sub(t.width());
    format!("{}{t}", " ".repeat(pad))
}

/// `text`'s lines, each tab as four spaces.
pub fn split_lines(text: &str) -> Vec<String> {
    text.lines().map(|l| expand_tabs(l).into_owned()).collect()
}

/// `text` with each tab as four spaces.
pub fn expand_tabs(text: &str) -> Cow<'_, str> {
    if text.contains('\t') {
        text.replace('\t', "    ").into()
    } else {
        text.into()
    }
}

/// `text` as spans, `tokens` (if any) colored by `theme` over `base`, cut
/// to `width` columns like [`truncate`]. With `pad`, padded out to `width`
/// like [`fit`].
pub fn code_spans<'a>(
    text: &'a str,
    tokens: Option<&Tokens>,
    base: Style,
    theme: &Theme,
    width: usize,
    pad: bool,
) -> Vec<Span<'a>> {
    marked_code_spans(text, tokens, base, theme, width, pad, &[])
}

/// [`code_spans`], with the byte ranges `marks` of `text` (a search's
/// matches, in order and apart) in the theme's search match style.
pub fn marked_code_spans<'a>(
    text: &'a str,
    tokens: Option<&Tokens>,
    base: Style,
    theme: &Theme,
    width: usize,
    pad: bool,
    marks: &[Range<usize>],
) -> Vec<Span<'a>> {
    let mut pieces: Vec<(&'a str, Style)> = Vec::new();
    let mut at = 0;
    for (range, class) in tokens.into_iter().flatten() {
        // Tokens for other text (stale, or cut differently) are ignored.
        let (Some(_), Some(piece)) = (text.get(at..range.start), text.get(range.clone())) else {
            break;
        };
        pieces.push((&text[at..range.start], base));
        pieces.push((piece, base.patch(theme.code(*class))));
        at = range.end;
    }
    pieces.push((&text[at..], base));
    pieces.retain(|(s, _)| !s.is_empty());
    if !marks.is_empty() {
        pieces = mark(pieces, marks, theme.search_match);
    }
    let mut spans = Vec::with_capacity(pieces.len() + 1);
    let mut used = 0;
    if text.width() <= width {
        spans.extend(pieces.into_iter().map(|(s, style)| Span::styled(s, style)));
        used = text.width();
    } else if width > 0 {
        'cut: for (s, style) in pieces {
            for (i, c) in s.char_indices() {
                let w = c.width().unwrap_or(0);
                if used + w > width - 1 {
                    spans.push(Span::styled(&s[..i], style));
                    spans.push(Span::styled("…", style));
                    used += 1;
                    break 'cut;
                }
                used += w;
            }
            spans.push(Span::styled(s, style));
        }
    }
    if pad && used < width {
        spans.push(Span::styled(" ".repeat(width - used), base));
    }
    spans
}

/// `pieces`, which run through a text end to end, split where the byte
/// ranges `marks` of that text start and end, the marked parts in `style`.
fn mark<'a>(pieces: Vec<(&'a str, Style)>, marks: &[Range<usize>], style: Style) -> Vec<(&'a str, Style)> {
    let mut out = Vec::with_capacity(pieces.len() + 2 * marks.len());
    let mut at = 0;
    for (s, base) in pieces {
        let end = at + s.len();
        let mut cut = 0;
        for m in marks.iter().filter(|m| m.start < end && m.end > at) {
            let (from, to) = (m.start.max(at) - at, m.end.min(end) - at);
            out.push((&s[cut..from], base));
            out.push((&s[from..to], base.patch(style)));
            cut = to;
        }
        out.push((&s[cut..], base));
        at = end;
    }
    out.retain(|(s, _)| !s.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn styled_pieces_are_cut_in_the_piece_that_overflows() {
        let red = Style::new().fg(Color::Red);
        let blue = Style::new().fg(Color::Blue);
        let parts = || vec![("abc".to_string(), red), ("defgh".to_string(), blue)];
        let line = truncate_spans(parts(), 8);
        assert_eq!(
            line,
            Line::from(vec![Span::styled("abc", red), Span::styled("defgh", blue)])
        );
        let line = truncate_spans(parts(), 6);
        assert_eq!(
            line,
            Line::from(vec![Span::styled("abc", red), Span::styled("de…", blue)])
        );
        let line = truncate_spans(parts(), 3);
        assert_eq!(line, Line::from(vec![Span::styled("ab…", red)]));
    }

    #[test]
    fn marks_split_across_pieces() {
        let red = Style::new().fg(Color::Red);
        let hit = Style::new().bg(Color::Yellow);
        // "let x" in red, then " = 1": marks on "t x =" and "1".
        let pieces = vec![("let x", red), (" = 1", Style::new())];
        let marked = mark(pieces, &[2..7, 8..9], hit);
        assert_eq!(
            marked,
            vec![
                ("le", red),
                ("t x", red.patch(hit)),
                (" =", hit),
                (" ", Style::new()),
                ("1", hit),
            ]
        );
    }

    #[test]
    fn marked_code_is_cut_like_the_rest() {
        let theme = Theme::default();
        let spans = marked_code_spans(
            "abcdef",
            None,
            Style::new(),
            &theme,
            4,
            false,
            std::slice::from_ref(&(1..5)),
        );
        let shown: Vec<_> = spans.iter().map(|s| (s.content.as_ref(), s.style)).collect();
        assert_eq!(
            shown,
            vec![
                ("a", Style::new()),
                ("bc", theme.search_match),
                ("…", theme.search_match)
            ]
        );
    }

    #[test]
    fn truncates_by_display_width() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate("日本語テキスト", 5), "日本…");
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit_right("ab", 4), "  ab");
    }

    #[test]
    fn split_lines_expands_tabs_as_before() {
        let old = |text: &str| -> Vec<String> { text.lines().map(|l| l.replace('\t', "    ")).collect() };
        for text in [
            "",
            "a",
            "a\n",
            "a\r\nb\tc\n\t\n",
            "no tabs\nat all\n\n",
            "\t\t",
            "x\ty\r",
        ] {
            assert_eq!(split_lines(text), old(text), "{text:?}");
        }
    }

    #[test]
    fn code_spans_style_tokens_and_cut_like_truncate() {
        use crate::syntax::Class;
        let theme = Theme::default();
        let tokens = vec![(0..2, Class::Keyword), (8..11, Class::Number)];
        let text = |spans: &[Span]| spans.iter().map(|s| s.content.as_ref()).collect::<String>();
        let spans = code_spans("fn x = 42;", Some(&tokens), Style::new(), &theme, 20, false);
        assert_eq!(text(&spans), "fn x = 42;");
        assert_eq!(spans[0].style, theme.code(Class::Keyword));
        // A range past the end is left plain rather than panicking.
        assert_eq!(text(&spans[1..]), " x = 42;");
        assert_eq!(spans[1].style, Style::new());
        let spans = code_spans("fn x = 42;", Some(&tokens), Style::new(), &theme, 6, true);
        assert_eq!(text(&spans), truncate("fn x = 42;", 6));
        let spans = code_spans("ab", None, Style::new(), &theme, 4, true);
        assert_eq!(text(&spans), fit("ab", 4));
        assert!(code_spans("ab", None, Style::new(), &theme, 0, true).is_empty());
    }

    #[test]
    fn cursor_moves_and_scrolls() {
        let mut c = Cursor::default();
        c.scroll_into_view(3);
        for _ in 0..5 {
            c.apply(Command::MoveDown, 10);
        }
        c.scroll_into_view(3);
        assert_eq!((c.selected, c.offset), (5, 3));
        c.apply(Command::Bottom, 10);
        c.scroll_into_view(3);
        assert_eq!((c.selected, c.offset), (9, 7));
        c.apply(Command::PageUp, 10);
        c.scroll_into_view(3);
        assert_eq!((c.selected, c.offset), (6, 6));
        c.apply(Command::Top, 10);
        c.scroll_into_view(3);
        assert_eq!((c.selected, c.offset), (0, 0));
        assert!(!c.apply(Command::Quit, 10));
        c.apply(Command::MoveDown, 0);
        assert_eq!(c.selected, 0);
    }
}
