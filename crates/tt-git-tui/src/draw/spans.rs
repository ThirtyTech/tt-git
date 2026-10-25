//! Styled text for ratatui: lines cut to a width, and code in its colors
//! with search matches picked out.

use std::ops::Range;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::syntax::Tokens;
use crate::tui_theme::Theme;
use crate::widgets::truncate;

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
    use crate::widgets::fit;
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
}
