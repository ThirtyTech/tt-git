//! The command palette and the pickers: what's typed, and the entries that
//! match it.

use unicode_width::UnicodeWidthStr;

use super::{Line, ListRow, search_line};
use crate::modal::{Modal, PALETTE_ROWS};
use crate::view::{StyleRole, TextSpan};
use crate::widgets::fit;

/// What the palette shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteView<'a> {
    pub title: &'a str,
    /// Columns across, the border included.
    pub width: usize,
    /// `> `, what's typed and the cursor.
    pub input: Line<'a>,
    /// The entries on screen, each its label fitted to the width and its
    /// hint (a key, a plugin) on the right.
    pub rows: Vec<ListRow<'a>>,
    /// What it says when nothing matches.
    pub empty: Option<&'static str>,
    /// The rows of entries to leave room for: as many as match, at least 1
    /// and at most [`PALETTE_ROWS`].
    pub list_rows: usize,
}

impl Modal {
    /// What the palette shows in a screen `cols` columns across; `None` for
    /// any other modal.
    pub fn palette_view(&self, cols: usize) -> Option<PaletteView<'_>> {
        let Modal::Palette {
            title,
            input,
            items,
            selected,
            lines,
        } = self
        else {
            return None;
        };
        // (label, hint) per row. A line number is the one thing to do, in
        // place of the commands.
        let (matches, selected) = match Modal::palette_line(input, *lines) {
            Some(line) => (vec![(format!("Go to line {line}"), format!("of {lines}"))], 0),
            None => (
                Modal::palette_matches(items, input)
                    .into_iter()
                    .map(|i| (i.label.clone(), i.hint.clone()))
                    .collect(),
                *selected,
            ),
        };
        let width = 70.min(cols);
        let inner_width = width.saturating_sub(2);
        let list_rows = matches.len().clamp(1, PALETTE_ROWS);
        let empty = matches.is_empty().then_some(if items.is_empty() && *lines > 0 {
            "type a line number"
        } else {
            "no matches"
        });
        let start = selected.saturating_sub(PALETTE_ROWS - 1);
        let rows = matches
            .into_iter()
            .enumerate()
            .skip(start)
            .take(PALETTE_ROWS)
            .map(|(i, (label, hint))| {
                let label_w = inner_width.saturating_sub(hint.width() + 1);
                ListRow {
                    picked: i == selected,
                    spans: vec![
                        TextSpan::plain(fit(&label, label_w)),
                        TextSpan::plain(" "),
                        TextSpan::new(hint, StyleRole::Muted),
                    ],
                    hint: true,
                }
            })
            .collect();
        Some(PaletteView {
            title,
            width,
            input: search_line(input),
            rows,
            empty,
            list_rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{Command, KeyCombo};
    use crate::modal::{PaletteItem, PaletteTarget};
    use crate::view::dialogs::line_text;

    fn item(label: &str, hint: &str) -> PaletteItem {
        PaletteItem {
            label: label.into(),
            hint: hint.into(),
            target: PaletteTarget::Command(Command::Quit),
        }
    }

    fn labels(view: &PaletteView) -> Vec<String> {
        view.rows
            .iter()
            .map(|r| line_text(&r.spans).trim_end().to_string())
            .collect()
    }

    #[test]
    fn rows_fit_the_label_and_put_the_hint_right() {
        let modal = Modal::palette(vec![item("Fetch", "F"), item("Push", "P")]);
        let view = modal.palette_view(30).unwrap();
        assert_eq!(view.title, "Commands");
        assert_eq!(view.width, 30);
        assert_eq!(line_text(&view.input), "> █");
        let first = line_text(&view.rows[0].spans);
        assert_eq!(first.width(), 28, "fills the inside");
        assert!(first.starts_with("Fetch ") && first.ends_with(" F"), "{first:?}");
        assert_eq!(view.rows[0].spans[2].role, StyleRole::Muted);
        assert!(view.rows[0].picked && !view.rows[1].picked);
        assert_eq!(view.list_rows, 2);
        assert_eq!(view.empty, None);
    }

    #[test]
    fn typing_narrows_and_nothing_matching_says_so() {
        let mut modal = Modal::palette(vec![item("Fetch", "F"), item("Push", "P")]);
        modal.handle_key(KeyCombo::char('u'));
        let view = modal.palette_view(70).unwrap();
        assert_eq!(labels(&view).len(), 1);
        assert!(labels(&view)[0].starts_with("Push"));
        modal.handle_key(KeyCombo::char('z'));
        let view = modal.palette_view(70).unwrap();
        assert!(view.rows.is_empty());
        assert_eq!(view.empty, Some("no matches"));
        assert_eq!(view.list_rows, 1, "room for the message");
    }

    #[test]
    fn a_number_goes_to_a_line() {
        let mut modal = Modal::go_to_line(40);
        let view = modal.palette_view(70).unwrap();
        assert_eq!(view.title, "Go to line (1-40)");
        assert_eq!(view.empty, Some("type a line number"));
        modal.handle_key(KeyCombo::char('7'));
        let view = modal.palette_view(70).unwrap();
        let row = &labels(&view)[0];
        assert!(row.starts_with("Go to line 7 ") && row.ends_with(" of 40"), "{row:?}");
        assert!(view.rows[0].picked);
    }

    #[test]
    fn a_long_list_scrolls_to_the_selection() {
        let items: Vec<PaletteItem> = (0..20).map(|i| item(&format!("item {i:02}"), "")).collect();
        let modal = Modal::picker("Pick", items, 15);
        let view = modal.palette_view(70).unwrap();
        assert_eq!(view.rows.len(), PALETTE_ROWS);
        assert_eq!(view.list_rows, PALETTE_ROWS);
        assert!(labels(&view)[0].starts_with("item 04"), "{:?}", labels(&view));
        assert!(view.rows.last().unwrap().picked, "the selection is the last row");
    }
}
