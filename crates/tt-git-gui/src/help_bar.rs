//! The help bar along the bottom: the focused pane's key hints and the
//! global ones, and on the right the notice and any git operation running.
//! It fits them to its width in columns as the TUI's does.

use gpui::{AnyElement, InteractiveElement, IntoElement, ParentElement, Styled, div};
use tt_git_app::App;
use tt_git_app::app::StatusLook;
use tt_git_app::view::{StyleRole, TextSpan};
use tt_git_app::widgets::truncate;
use unicode_width::UnicodeWidthStr;

use crate::text::StyledLine;
use crate::ui::Ui;

/// The id the help bar is measured as.
pub const ID: &str = "help-bar";

pub fn render(app: &App, ui: &Ui) -> AnyElement {
    let width = ui.fit(ID).cols;
    let (keys, status) = spans(app, width);
    div()
        .id(ID)
        .relative()
        .flex_none()
        .h(ui.font.line_height)
        .w_full()
        .flex()
        .overflow_hidden()
        .whitespace_nowrap()
        .px(ui.font.cell_width / 2.)
        .child(ui.measure(ID))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(StyledLine::new(&ui.theme, &keys, None).element()),
        )
        .child(
            div()
                .flex_none()
                .child(StyledLine::new(&ui.theme, &status, None).element()),
        )
        .into_any_element()
}

/// The key hints and the status slot, fitted to `width` columns.
pub fn spans(app: &App, width: usize) -> (Vec<TextSpan<'static>>, Vec<TextSpan<'static>>) {
    // The notice and any running git operation get up to half the width,
    // so a long list of key hints can't push them off.
    let parts: Vec<TextSpan<'static>> = app
        .help_status()
        .into_iter()
        .map(|(text, look)| {
            let role = match look {
                StatusLook::Muted => StyleRole::Muted,
                StatusLook::Success => StyleRole::DiffAdd,
                StatusLook::Busy => StyleRole::Busy,
            };
            TextSpan::new(text, role)
        })
        .collect();
    let status_width = parts.iter().map(|s| s.text.width()).sum::<usize>().min(width / 2);
    let status = truncate_spans(parts, status_width);
    let keys_width = width.saturating_sub(status_width);

    // Global hints always show; pane hints fill what's left, in order,
    // skipping any that don't fit (the full list is under `?`).
    let (pane_hints, globals) = app.help_hints();
    let width_of = |(k, l): &(&str, &str)| k.width() + 1 + l.width() + 2;
    let mut budget = keys_width.saturating_sub(globals.iter().map(width_of).sum());
    let mut items = Vec::new();
    for item in pane_hints {
        if width_of(&item) <= budget {
            budget -= width_of(&item);
            items.push(item);
        }
    }
    items.extend(globals);
    let mut keys = Vec::new();
    for (key, label) in items {
        if !keys.is_empty() {
            keys.push(TextSpan::plain("  "));
        }
        keys.push(TextSpan::new(key.to_string(), StyleRole::HelpKey));
        keys.push(TextSpan::new(format!(" {label}"), StyleRole::HelpText));
    }
    (keys, status)
}

/// Cuts differently styled pieces of one line at `width` columns, ending
/// with `…` in the style of the piece it cut.
fn truncate_spans(parts: Vec<TextSpan<'static>>, width: usize) -> Vec<TextSpan<'static>> {
    if parts.iter().map(|s| s.text.width()).sum::<usize>() <= width {
        return parts;
    }
    let mut out = Vec::new();
    let mut left = width;
    for span in parts {
        if span.text.width() < left {
            left -= span.text.width();
            out.push(span);
        } else {
            // Something comes after this piece even when it fits exactly,
            // so it always ends with `…`.
            out.push(TextSpan::new(truncate(&format!("{}…", span.text), left), span.role));
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_status_is_cut_with_an_ellipsis() {
        let parts = vec![
            TextSpan::new(" │ ", StyleRole::Muted),
            TextSpan::new("pushing main", StyleRole::Busy),
        ];
        let cut = truncate_spans(parts.clone(), 8);
        let text: String = cut.iter().map(|s| s.text.as_ref()).collect();
        assert_eq!(text, " │ push…");
        assert_eq!(cut[1].role, StyleRole::Busy);
        assert_eq!(truncate_spans(parts.clone(), 40), parts);
    }
}
