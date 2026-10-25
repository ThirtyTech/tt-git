//! Code as spans: highlighted tokens in their [`StyleRole::Code`] roles,
//! search matches laid over them, cut to a width. For the view models that
//! show code (the details diff, staging and blame), as the TUI's
//! `draw/spans.rs` draws it.

use std::ops::Range;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{StyleRole, TextSpan};
use crate::syntax::{Class, Tokens};

/// `text` as spans: `tokens` (if any) in their code roles over `base`,
/// the byte ranges `marks` (a search's matches, in order and apart) under
/// [`StyleRole::SearchMatch`], cut to `width` columns ending with `…` like
/// [`truncate`](crate::widgets::truncate). With `pad`, padded with `base`
/// spaces out to `width` like [`fit`](crate::widgets::fit).
///
/// A highlighted token on a `base` other than [`StyleRole::Plain`] takes
/// `base` as its role and the code role [`over`](TextSpan::over) it; marked,
/// it keeps only the code role, with the match over that.
pub fn code_spans<'a>(
    text: &'a str,
    tokens: Option<&Tokens>,
    base: StyleRole,
    width: usize,
    pad: bool,
    marks: &[Range<usize>],
) -> Vec<TextSpan<'a>> {
    let mut pieces: Vec<(&'a str, Option<Class>)> = Vec::new();
    let mut at = 0;
    for (range, class) in tokens.into_iter().flatten() {
        // Tokens for other text (stale, or cut differently) are ignored.
        let (Some(_), Some(piece)) = (text.get(at..range.start), text.get(range.clone())) else {
            break;
        };
        pieces.push((&text[at..range.start], None));
        pieces.push((piece, Some(*class)));
        at = range.end;
    }
    pieces.push((&text[at..], None));
    pieces.retain(|(s, _)| !s.is_empty());
    let pieces = mark(pieces, marks);
    let span = |s: &'a str, class: Option<Class>, marked: bool| {
        let (role, over) = match (class, marked) {
            (None, false) => (base, None),
            (None, true) => (base, Some(StyleRole::SearchMatch)),
            (Some(c), false) if base == StyleRole::Plain => (StyleRole::Code(c), None),
            (Some(c), false) => (base, Some(StyleRole::Code(c))),
            (Some(c), true) => (StyleRole::Code(c), Some(StyleRole::SearchMatch)),
        };
        TextSpan {
            text: s.into(),
            role,
            over,
        }
    };

    let mut spans = Vec::with_capacity(pieces.len() + 1);
    let mut used = 0;
    if text.width() <= width {
        spans.extend(pieces.into_iter().map(|(s, class, marked)| span(s, class, marked)));
        used = text.width();
    } else if width > 0 {
        'cut: for (s, class, marked) in pieces {
            for (i, c) in s.char_indices() {
                let w = c.width().unwrap_or(0);
                if used + w > width - 1 {
                    if i > 0 {
                        spans.push(span(&s[..i], class, marked));
                    }
                    spans.push(span("…", class, marked));
                    used += 1;
                    break 'cut;
                }
                used += w;
            }
            spans.push(span(s, class, marked));
        }
    }
    if pad && used < width {
        spans.push(TextSpan::new(" ".repeat(width - used), base));
    }
    spans
}

/// `pieces`, which run through a text end to end, split where the byte
/// ranges `marks` of that text start and end, each saying if it's marked.
fn mark<'a>(pieces: Vec<(&'a str, Option<Class>)>, marks: &[Range<usize>]) -> Vec<(&'a str, Option<Class>, bool)> {
    let mut out = Vec::with_capacity(pieces.len() + 2 * marks.len());
    let mut at = 0;
    for (s, class) in pieces {
        let end = at + s.len();
        let mut cut = 0;
        for m in marks.iter().filter(|m| m.start < end && m.end > at) {
            let (from, to) = (m.start.max(at) - at, m.end.min(end) - at);
            out.push((&s[cut..from], class, false));
            out.push((&s[from..to], class, true));
            cut = to;
        }
        out.push((&s[cut..], class, false));
        at = end;
    }
    out.retain(|(s, ..)| !s.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{fit, truncate};

    fn text(spans: &[TextSpan]) -> String {
        spans.iter().map(|s| s.text.as_ref()).collect()
    }

    #[test]
    fn tokens_take_their_code_roles_and_cut_like_truncate() {
        let tokens = vec![(0..2, Class::Keyword), (8..11, Class::Number)];
        let spans = code_spans("fn x = 42;", Some(&tokens), StyleRole::Plain, 20, false, &[]);
        assert_eq!(text(&spans), "fn x = 42;");
        assert_eq!(spans[0].role, StyleRole::Code(Class::Keyword));
        // A range past the end is left plain rather than panicking.
        assert_eq!(text(&spans[1..]), " x = 42;");
        assert_eq!(spans[1].role, StyleRole::Plain);
        let spans = code_spans("fn x = 42;", Some(&tokens), StyleRole::Plain, 6, true, &[]);
        assert_eq!(text(&spans), truncate("fn x = 42;", 6));
        let spans = code_spans("ab", None, StyleRole::Plain, 4, true, &[]);
        assert_eq!(text(&spans), fit("ab", 4));
        assert!(code_spans("ab", None, StyleRole::Plain, 0, true, &[]).is_empty());
    }

    #[test]
    fn marks_lie_over_code_and_split_across_tokens() {
        // "let" a keyword; marks on "t x =" and "1".
        let tokens = vec![(0..3, Class::Keyword)];
        let spans = code_spans("let x = 1", Some(&tokens), StyleRole::Plain, 20, false, &[2..7, 8..9]);
        let shown: Vec<_> = spans.iter().map(|s| (s.text.as_ref(), s.role, s.over)).collect();
        let kw = StyleRole::Code(Class::Keyword);
        let hit = Some(StyleRole::SearchMatch);
        assert_eq!(
            shown,
            vec![
                ("le", kw, None),
                ("t", kw, hit),
                (" x =", StyleRole::Plain, hit),
                (" ", StyleRole::Plain, None),
                ("1", StyleRole::Plain, hit),
            ]
        );
    }

    #[test]
    fn code_on_another_base_lies_over_it() {
        let tokens = vec![(0..2, Class::Keyword)];
        let spans = code_spans("fn", Some(&tokens), StyleRole::DiffAdd, 20, false, &[]);
        assert_eq!(spans[0].role, StyleRole::DiffAdd);
        assert_eq!(spans[0].over, Some(StyleRole::Code(Class::Keyword)));
    }

    #[test]
    fn a_cut_keeps_the_look_of_the_piece_it_cuts() {
        let spans = code_spans(
            "abcdef",
            None,
            StyleRole::Plain,
            4,
            false,
            std::slice::from_ref(&(1..5)),
        );
        let shown: Vec<_> = spans.iter().map(|s| (s.text.as_ref(), s.over)).collect();
        let hit = Some(StyleRole::SearchMatch);
        assert_eq!(shown, vec![("a", None), ("bc", hit), ("…", hit)]);
    }
}
