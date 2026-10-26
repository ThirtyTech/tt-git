//! The graph's filter panel: the author and file tabs, what's typed, and
//! the choices that match.

use unicode_width::UnicodeWidthStr;

use super::{Line, ListRow, choice, cursor, separator};
use crate::filter::{FilterPanel, FilterTab, ROWS};
use crate::view::{StyleRole, TextSpan};
use crate::widgets::{fit, truncate};

pub const FILTER_TITLE: &str = "Filter graph";
pub const FILTER_HINT: &str = "enter filter · tab author/file · ↑↓ pick · esc cancel";

/// What the filter panel shows. The panel is a fixed height (the tabs, the
/// input, a blank line, [`ROWS`] of choices, a blank line and the keys), so
/// it doesn't jump about while typing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterView<'a> {
    /// Columns across, the border included.
    pub width: usize,
    /// Each tab with its text (`Author: any`), the current one underlined.
    pub tabs: Vec<(FilterTab, TextSpan<'a>)>,
    /// `> `, what's typed, the cursor, and on the right how many match.
    pub input: Line<'a>,
    /// Said in place of the choices: loading, an error, no matches.
    pub message: Option<TextSpan<'a>>,
    /// The choices on screen, at most [`ROWS`].
    pub rows: Vec<ListRow<'a>>,
}

impl<'a> FilterView<'a> {
    /// The tabs as one line, ` · ` between them.
    pub fn tab_line(&self) -> Line<'a> {
        let mut line = Vec::new();
        for (_, span) in &self.tabs {
            if !line.is_empty() {
                line.push(separator());
            }
            line.push(span.clone());
        }
        line
    }
}

impl FilterPanel {
    /// What the panel shows in a screen `cols` columns across.
    pub fn view(&self, cols: usize) -> FilterView<'_> {
        let width = 70.min(cols);
        let inner_width = width.saturating_sub(2);
        let list = self.list();
        let tabs = FilterTab::ALL
            .into_iter()
            .map(|tab| (tab, choice(self.tab_text(tab), tab == self.tab)))
            .collect();

        let count = match &list.items {
            Some(Ok(items)) => format!("{}/{}", list.matches.len(), items.len()),
            _ => String::new(),
        };
        let typed = truncate(&list.input, inner_width.saturating_sub(count.width() + 4));
        let pad = inner_width.saturating_sub(typed.width() + count.width() + 3);
        let input = vec![
            TextSpan::new("> ", StyleRole::HelpKey),
            TextSpan::plain(typed),
            cursor(),
            TextSpan::plain(" ".repeat(pad)),
            TextSpan::new(count, StyleRole::Muted),
        ];

        let mut rows = Vec::new();
        let message = match &list.items {
            None => Some(TextSpan::new("loading…", StyleRole::Muted)),
            Some(Err(e)) => Some(TextSpan::new(e.as_str(), StyleRole::Error)),
            Some(Ok(_)) if list.matches.is_empty() => {
                let typed = list.input.trim();
                let msg = if self.tab == FilterTab::File && !typed.is_empty() {
                    format!("no matches · enter shows commits that changed {typed}")
                } else {
                    "no matches".into()
                };
                Some(TextSpan::new(msg, StyleRole::Muted))
            }
            Some(Ok(items)) => {
                for (pos, &i) in list.matches.iter().enumerate().skip(list.first_shown()).take(ROWS) {
                    let choice = &items[i];
                    let label_w = inner_width.saturating_sub(choice.hint.width() + 1);
                    // The entry that clears the filter is muted.
                    let label_role = if choice.value.is_none() {
                        StyleRole::Muted
                    } else {
                        StyleRole::Plain
                    };
                    rows.push(ListRow {
                        picked: pos == list.selected,
                        spans: vec![
                            TextSpan::new(fit(&choice.label, label_w), label_role),
                            TextSpan::plain(" "),
                            TextSpan::new(choice.hint.as_str(), StyleRole::Muted),
                        ],
                        hint: true,
                    });
                }
                None
            }
        };
        FilterView {
            width,
            tabs,
            input,
            message,
            rows,
        }
    }
}

#[cfg(test)]
mod tests {
    use tt_git_core::LogFilter;

    use super::*;
    use crate::filter::{author_choices, file_choices};
    use crate::keymap::KeyCombo;
    use crate::view::dialogs::line_text;

    fn panel() -> FilterPanel {
        FilterPanel::new(LogFilter::default(), None)
    }

    #[test]
    fn loading_then_the_choices() {
        let mut p = panel();
        let view = p.view(200);
        assert_eq!(view.width, 70);
        assert_eq!(line_text(&view.tab_line()), "Author: any · File: any");
        assert_eq!(view.tabs[0].1.over, Some(StyleRole::Underlined), "on the author tab");
        assert_eq!(view.tabs[1].1.role, StyleRole::Muted);
        assert_eq!(view.message.as_ref().map(|m| m.text.as_ref()), Some("loading…"));
        let input = line_text(&view.input);
        assert_eq!(input.width(), 68, "the count is on the right");
        assert!(input.starts_with("> █"));

        p.set_choices(
            FilterTab::Author,
            Ok(author_choices(vec![("Ann <a@x>".into(), 3), ("Bob <b@x>".into(), 1)])),
        );
        let view = p.view(200);
        assert_eq!(view.message, None);
        assert_eq!(view.rows.len(), 2);
        assert!(view.rows[0].picked);
        let first = line_text(&view.rows[0].spans);
        assert!(first.starts_with("Ann <a@x>") && first.ends_with(" 3"), "{first:?}");
        assert!(line_text(&view.input).ends_with("2/2"));
    }

    #[test]
    fn errors_and_no_matches() {
        let mut p = panel();
        p.set_choices(FilterTab::Author, Err("git failed".into()));
        let view = p.view(70);
        let msg = view.message.unwrap();
        assert_eq!((msg.text.as_ref(), msg.role), ("git failed", StyleRole::Error));

        p.handle_key("tab".parse().unwrap());
        p.set_choices(FilterTab::File, Ok(file_choices(vec!["a.txt".into()])));
        for c in "zz".chars() {
            p.handle_key(KeyCombo::char(c));
        }
        let view = p.view(70);
        assert_eq!(view.tabs[1].1.over, Some(StyleRole::Underlined));
        assert_eq!(
            view.message.unwrap().text,
            "no matches · enter shows commits that changed zz"
        );
        assert!(view.rows.is_empty());
    }

    #[test]
    fn the_clear_entry_is_muted() {
        let current = LogFilter {
            author: Some("Ann <a@x>".into()),
            ..LogFilter::default()
        };
        let mut p = FilterPanel::new(current, None);
        p.set_choices(FilterTab::Author, Ok(author_choices(vec![("Ann <a@x>".into(), 3)])));
        let view = p.view(70);
        assert_eq!(line_text(&view.tab_line()), "Author: Ann · File: any");
        assert_eq!(view.rows[0].spans[0].role, StyleRole::Muted);
        assert!(line_text(&view.rows[0].spans).ends_with(" clear"));
        assert_eq!(view.rows[1].spans[0].role, StyleRole::Plain);
    }

    #[test]
    fn a_long_list_shows_rows_around_the_selection() {
        let mut p = panel();
        let authors = (0..30).map(|i| (format!("A{i:02} <a@x>"), 1)).collect();
        p.set_choices(FilterTab::Author, Ok(author_choices(authors)));
        p.handle_key("pagedown".parse().unwrap());
        p.handle_key("pagedown".parse().unwrap());
        let view = p.view(70);
        assert_eq!(view.rows.len(), ROWS);
        assert!(view.rows.last().unwrap().picked);
        assert!(line_text(&view.rows[0].spans).starts_with("A13"), "{:?}", view.rows[0]);
    }
}
