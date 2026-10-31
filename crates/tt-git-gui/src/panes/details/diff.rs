//! The details pane's text (Phase B4): a commit, a diff or other text, or in
//! the files view the selected file's diff, with syntax highlighting and
//! search matches, from the details view model's
//! [`text_rows`](tt_git_app::panes::DetailsPane::text_rows). The mouse
//! selects it (the app keeps the selection, by line and column), for
//! ctrl+c, cmd+c on macOS, or `c` to copy.

use gpui::{AnyElement, HighlightStyle, IntoElement};
use tt_git_app::mouse::{MouseTarget, PanePart};
use tt_git_app::panes::{DetailsPane, PaneId};
use tt_git_app::style::Style;
use tt_git_app::view::details::TextRow;

use crate::rows::{StyledRow, StyledRows};
use crate::text::StyledLine;
use crate::theme::GuiTheme;
use crate::ui::Ui;

/// The id the text is measured as.
pub const TEXT: &str = "details:text";

/// The text, filling its parent (which must be `.relative()`), its first
/// line on pane body row `first_body_row`. A click or the wheel on it is
/// over `part` too: [`PanePart::Diff`] in the files view, so the wheel
/// scrolls the diff rather than moving through the files.
pub fn render(ui: &Ui, pane: &mut DetailsPane, first_body_row: usize, part: Option<PanePart>) -> AnyElement {
    let fit = ui.fit(TEXT);
    pane.set_viewport(fit.rows);
    let first = pane.cursor.offset;
    let selection = pane.selection().cloned();
    let rows = pane
        .text_rows(fit.rows, fit.cols)
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut row = text_row(&ui.theme, row);
            if let Some(bytes) = selection.as_ref().and_then(|s| s.bytes_on(first + i)) {
                row.line.select(&ui.theme, bytes);
            }
            row
        })
        .collect();
    StyledRows::new(ui, TEXT, PaneId::Details, rows)
        .selectable()
        .on_target(move |line| MouseTarget::Pane {
            pane: PaneId::Details,
            body_row: Some(first_body_row + line),
            part,
        })
        .into_any_element()
}

/// A text row as the TUI draws it: the spans over the row's
/// [`pick`](TextRow::pick) laid over its [`back`](TextRow::back).
pub fn text_row(theme: &GuiTheme, row: &TextRow) -> StyledRow {
    let (back, pick) = (row.back, row.pick);
    let Some(role) = back.or(pick) else {
        return StyledRow::new(theme, &row.spans, None);
    };
    if back.is_none() || pick.is_none() {
        return StyledRow::new(theme, &row.spans, Some(role));
    }
    let app = theme.app_theme();
    let look = [back, pick]
        .into_iter()
        .flatten()
        .fold(Style::new(), |look, role| look.patch(app.role(role)));
    let mut text = String::new();
    let mut highlights = Vec::with_capacity(row.spans.len());
    for span in &row.spans {
        let start = text.len();
        text.push_str(&span.text);
        let style = theme.style(look.patch(app.span(span)));
        if start < text.len() && style != HighlightStyle::default() {
            highlights.push((start..text.len(), style));
        }
    }
    StyledRow {
        line: StyledLine {
            text: text.into(),
            highlights,
        },
        background: theme.style(look).background_color,
        ..StyledRow::default()
    }
}

#[cfg(test)]
mod tests {
    use tt_git_app::theme::Theme;
    use tt_git_app::view::{StyleRole, TextSpan};

    use super::*;

    #[test]
    fn a_picked_changed_line_lays_the_pick_over_the_tint() {
        let theme = GuiTheme::new(&Theme::default());
        let app = theme.app_theme();
        let spans = vec![
            TextSpan::new("+", StyleRole::DiffAdd),
            TextSpan::plain("x"),
            TextSpan::plain("y").with_over(StyleRole::SearchMatch),
        ];
        let row = |back, pick| {
            text_row(
                &theme,
                &TextRow {
                    spans: spans.clone(),
                    back,
                    pick,
                },
            )
        };
        let tinted = row(Some(StyleRole::DiffAddLine), None);
        assert_eq!(tinted.background, theme.bg(StyleRole::DiffAddLine));
        assert_eq!(tinted, StyledRow::new(&theme, &spans, Some(StyleRole::DiffAddLine)));
        assert_eq!(row(None, None), StyledRow::new(&theme, &spans, None));

        let both = row(Some(StyleRole::DiffAddLine), Some(StyleRole::SelectedUnfocused));
        let look = app
            .role(StyleRole::DiffAddLine)
            .patch(app.role(StyleRole::SelectedUnfocused));
        assert_eq!(both.background, theme.style(look).background_color);
        assert_eq!(&*both.line.text, "+xy");
        let style_of = |at: usize| {
            both.line
                .highlights
                .iter()
                .find(|(r, _)| r.contains(&at))
                .map(|(_, s)| *s)
                .unwrap_or_default()
        };
        assert_eq!(style_of(0), theme.style(look.patch(app.role(StyleRole::DiffAdd))));
        assert_eq!(style_of(1), theme.style(look));
        assert_eq!(style_of(2), theme.style(look.patch(app.span(&spans[2]))));
    }
}
