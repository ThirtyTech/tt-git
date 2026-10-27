//! The help bar along the bottom: the focused pane's key hints and the
//! global ones, and on the right the notice and any git operation running.
//! It fits them to its width in columns as the TUI's does. The native style
//! draws it as a status bar: each key on a chip, the labels in the UI font.
//! A plugin's failure in the notice opens the plugin logs when clicked.

use gpui::{
    AnyElement, Div, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, Stateful, Styled, div,
    prelude::FluentBuilder, rems,
};
use tt_git_app::app::StatusLook;
use tt_git_app::view::{StyleRole, TextSpan};
use tt_git_app::widgets::truncate;
use tt_git_app::{Action, App};
use unicode_width::UnicodeWidthStr;

use crate::look;
use crate::text::StyledLine;
use crate::ui::Ui;

/// The id the help bar is measured as.
pub const ID: &str = "help-bar";

pub fn render(app: &App, ui: &Ui) -> AnyElement {
    let width = ui.fit(ID).cols;
    let (keys, status) = spans(app, width);
    let status = status_slot(ui, &status, app.notice_clickable());
    if ui.native() {
        return status_bar(ui, &keys, status);
    }
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
        .child(status)
        .into_any_element()
}

/// The notice and what's running; clicked, a plugin's failure opens the
/// plugin logs, where the whole of it is to read and copy.
fn status_slot(ui: &Ui, status: &[TextSpan], clickable: bool) -> Stateful<Div> {
    let ui = ui.clone();
    div()
        .id("help-bar:status")
        .flex_none()
        .child(StyledLine::new(&ui.theme, status, None).element())
        .when(clickable, |d| {
            d.cursor_pointer().on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                ui.send(Action::NoticeClicked, cx);
            })
        })
}

/// The native help bar: the hints as key chips and labels, the status on
/// the right.
fn status_bar(ui: &Ui, keys: &[TextSpan], status: Stateful<Div>) -> AnyElement {
    let theme = &ui.theme;
    let mut items: Vec<Div> = Vec::new();
    let mut key = None;
    for span in keys {
        match span.role {
            StyleRole::HelpKey => key = Some(span.text.to_string()),
            StyleRole::HelpText => {
                let label = span.text.trim_start().to_string();
                items.push(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(rems(0.375))
                        .children(key.take().map(|k| key_chip(ui, k)))
                        .child(div().text_color(theme.fg(StyleRole::HelpText)).child(label)),
                );
            }
            _ => {}
        }
    }
    div()
        .id(ID)
        .relative()
        .flex_none()
        .h(look::STATUS_BAR_HEIGHT)
        .w_full()
        .flex()
        .items_center()
        .gap(rems(1.))
        .overflow_hidden()
        .whitespace_nowrap()
        .px(look::HEADER_PAD)
        .font_family(look::ui_font_family())
        .child(ui.measure(ID))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .items_center()
                .gap(rems(1.))
                .children(items),
        )
        .child(status)
        .into_any_element()
}

/// A key on a chip, as the native style shows keys: in the status bar, in
/// dialogs, on the palette's rows.
pub fn key_chip(ui: &Ui, key: impl Into<gpui::SharedString>) -> Div {
    let theme = &ui.theme;
    let s = &theme.surfaces;
    div()
        .flex_none()
        .flex()
        .items_center()
        .h(rems(1.125))
        .px(rems(0.3125))
        .rounded(look::CHIP_RADIUS)
        .border_1()
        .border_color(s.hairline)
        .bg(s.tint)
        .text_color(theme.fg(StyleRole::HelpKey))
        .font_family(ui.font.family.clone())
        .font_weight(FontWeight::MEDIUM)
        .text_size(ui.font.size * 0.85)
        .child(key.into())
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
