//! A prompt for a line of text (`Modal::Prompt`), and what every dialog
//! draws with: the box ([`dialog_box`]), a dialog of wrapped text from its
//! [`DialogView`] ([`text_dialog`], which confirms, discard-all and the
//! error popup use too), and the width the view models fit to
//! ([`screen_cols`]).
//!
//! What's typed reaches the modal through tt-git's keymap, as every key
//! does; the cursor is the view model's block at the end of the input.

use gpui::{
    AnyElement, BoxShadow, Div, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled, div, point,
    prelude::FluentBuilder as _, px, rems,
};
use tt_git_app::view::dialogs::{DialogLook, DialogView, ListRow};
use tt_git_app::view::{StyleRole, TextSpan};

use super::DialogCx;
use crate::help_bar::key_chip;
use crate::look;
use crate::overlay;
use crate::rows::{StyledRow, row_box};
use crate::shell::MAIN;
use crate::text::{StyledLine, selection};
use crate::ui::Ui;

/// The id a prompt, confirm or discard-all is measured as.
pub const ID: &str = "dialog:prompt";

pub fn render(cx: &mut DialogCx) -> AnyElement {
    let cols = screen_cols(cx.ui);
    match cx.app.modal().and_then(|m| m.dialog_view(cols)) {
        Some(view) => text_dialog(cx.ui, ID, view),
        None => div().into_any_element(),
    }
}

/// The main area's width in columns: what the view models fit a dialog to.
/// Before the first frame measures it, wide enough for any dialog.
pub fn screen_cols(ui: &Ui) -> usize {
    match ui.fit(MAIN).cols {
        0 => 200,
        cols => cols,
    }
}

/// A layer over the main area with `dialog` in its middle, taking the
/// mouse from the panes behind it (the app ignores the mouse while a
/// dialog is open, as in the terminal). The native style dims the panes.
pub fn layer(ui: &Ui, dialog: impl IntoElement) -> Div {
    overlay::centered(dialog)
        .occlude()
        .when(ui.native(), |d| d.bg(ui.theme.surfaces.scrim))
}

/// A dialog's box, `width` columns across as the view model says (the
/// border included, as in the terminal, so `width - 2` columns of text fit
/// inside), with `title` on top in `look`'s colors. Measured as `id`; add
/// the body as children.
pub fn dialog_box(ui: &Ui, id: &'static str, look: DialogLook, width: usize, title: impl IntoElement) -> Div {
    let (theme, font) = (&ui.theme, &ui.font);
    let native = ui.native();
    let s = &theme.surfaces;
    let dialog = div()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        // The padding takes the border's two columns; a little more so a
        // line exactly as wide as the inside doesn't wrap.
        .w(font.cell_width * width as f32 + font.cell_width * 0.5)
        .max_w_full()
        .max_h_full()
        .overflow_hidden()
        .px(font.cell_width)
        .border_1()
        .child(ui.measure(id));
    let dialog = if native {
        // Raised over the dimmed panes, with a soft shadow; an error's
        // border keeps its color.
        let border = match look {
            DialogLook::Normal => s.hairline_strong,
            DialogLook::Error => theme.fg(look.border()).opacity(0.7),
        };
        dialog
            .pt(rems(0.625))
            .pb(rems(0.75))
            .rounded(look::DIALOG_RADIUS)
            .border_color(border)
            .bg(s.raised)
            .shadow(vec![
                BoxShadow {
                    color: s.shadow,
                    offset: point(px(0.), px(16.)),
                    blur_radius: px(40.),
                    spread_radius: px(0.),
                    inset: false,
                },
                BoxShadow {
                    color: s.shadow.opacity(0.6),
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(8.),
                    spread_radius: px(0.),
                    inset: false,
                },
            ])
    } else {
        dialog
            .pb(font.line_height / 2.)
            .rounded(rems(0.375))
            .border_color(theme.fg(look.border()))
            .bg(theme.background)
            .shadow_lg()
    };
    dialog.child(
        div()
            .flex()
            .flex_none()
            .h(font.line_height)
            .overflow_hidden()
            .whitespace_nowrap()
            .text_color(theme.fg(look.title()))
            .when(native, |d| {
                d.items_center()
                    .font_family(look::ui_font_family())
                    .font_weight(FontWeight::SEMIBOLD)
            })
            .child(title),
    )
}

/// One line of view-model spans, wrapped to the dialog's width; in the UI
/// font in the native style.
pub fn line(ui: &Ui, spans: &[TextSpan]) -> Div {
    div()
        .min_h(ui.font.line_height)
        .whitespace_normal()
        .when(ui.native(), |d| d.font_family(look::ui_font_family()))
        .child(StyledLine::new(&ui.theme, spans, None).element())
}

/// One line that's cut at the dialog's edge rather than wrapped.
pub fn single_line(ui: &Ui, spans: &[TextSpan]) -> Div {
    div()
        .h(ui.font.line_height)
        .flex_none()
        .overflow_hidden()
        .whitespace_nowrap()
        .child(StyledLine::new(&ui.theme, spans, None).element())
}

/// A blank line.
pub fn blank(ui: &Ui) -> Div {
    div().h(ui.font.line_height).flex_none()
}

/// A list entry, the picked one in the selection's look across the whole
/// row.
pub fn list_row(ui: &Ui, row: &ListRow) -> Div {
    if ui.native() {
        return native_list_row(ui, row);
    }
    let row = StyledRow::new(&ui.theme, &row.spans, selection(row.picked, true));
    let mut d = div()
        .h(ui.font.line_height)
        .flex_none()
        .w_full()
        .overflow_hidden()
        .whitespace_nowrap();
    if let Some(bg) = row.background {
        d = d.bg(bg);
    }
    d.child(row.line.element())
}

/// The native style's list entry: rounded when picked, the label in the
/// UI font and its hint (a key, a plugin) on a chip at the right.
fn native_list_row(ui: &Ui, row: &ListRow) -> Div {
    let n = row.spans.len();
    let (label, hint) = match row.hint {
        true if n >= 2 => (&row.spans[..n - 2], row.spans[n - 1].text.trim()),
        _ => (&row.spans[..], ""),
    };
    let styled = StyledRow::new(&ui.theme, label, selection(row.picked, true));
    row_box(ui, &styled)
        .flex_none()
        .flex()
        .items_center()
        .gap(rems(0.75))
        .whitespace_nowrap()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .font_family(look::ui_font_family())
                .child(styled.line.element()),
        )
        .when(!hint.is_empty(), |d| d.child(key_chip(ui, hint.to_string())))
}

/// The title as the view model's look has it. The native style draws an
/// ordinary dialog's title in the text color (an error's stays red).
pub fn title(ui: &Ui, text: &str, look: DialogLook) -> StyledLine {
    let role = match look {
        DialogLook::Normal if ui.native() => StyleRole::Plain,
        _ => look.title(),
    };
    StyledLine::new(&ui.theme, &[TextSpan::new(text, role)], None)
}

/// A dialog's keys (`y confirm · n/esc cancel`): muted text, or in the
/// native style each key on a chip before its label.
fn keys_line(ui: &Ui, hint: &str) -> Div {
    if !ui.native() {
        return line(ui, &[TextSpan::new(hint, StyleRole::Muted)]);
    }
    let muted = ui.theme.fg(StyleRole::Muted);
    div()
        .flex()
        .flex_wrap()
        .justify_end()
        .gap_x(rems(1.))
        .gap_y(rems(0.25))
        .font_family(look::ui_font_family())
        .children(hint.split(" · ").map(|item| {
            let (key, label) = item.split_once(' ').unwrap_or(("", item));
            div()
                .flex()
                .items_center()
                .gap(rems(0.375))
                .whitespace_nowrap()
                .when(!key.is_empty(), |d| d.child(key_chip(ui, key.to_string())))
                .child(div().text_color(muted).child(label.to_string()))
        }))
}

/// A dialog of text: the title, the body wrapped to the width, then a
/// blank line and the keys.
pub fn text_dialog(ui: &Ui, id: &'static str, view: DialogView) -> AnyElement {
    let title = title(ui, &view.title, view.look).element();
    let mut dialog = dialog_box(ui, id, view.look, view.width, title)
        .child(div().flex().flex_col().children(view.body.iter().map(|l| line(ui, l))));
    if let Some(hint) = &view.hint {
        dialog = dialog.child(blank(ui)).child(keys_line(ui, hint));
    }
    layer(ui, dialog).into_any_element()
}

#[cfg(test)]
pub(crate) mod tests {
    use gpui::TestAppContext;
    use tt_git_app::modal::{Modal, PromptAction};
    use tt_git_app::view::dialogs::line_text;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    fn sandbox() -> Sandbox {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        sb
    }

    /// Where dialog `id` was drawn in the last frame, if it was.
    pub(crate) fn drawn(gui: &Gui, id: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        gui.shell
            .read_with(&gui.cx, |shell, _| shell.ui().measures.this_frame(id))
    }

    /// How many columns wide dialog `id` was drawn.
    pub(crate) fn drawn_cols(gui: &Gui, id: &str) -> usize {
        let bounds = drawn(gui, id).expect("drawn");
        let cell = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().font.cell_width);
        (bounds.size.width / cell).floor() as usize
    }

    #[gpui::test]
    fn the_new_branch_prompt_takes_typing_and_tab(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        assert!(drawn(&gui, ID).is_none());
        gui.keystrokes("n");
        assert!(matches!(
            gui.read(|app| app.modal().cloned()),
            Some(Modal::Prompt {
                action: PromptAction::CreateBranch { worktree: false, .. },
                ..
            })
        ));
        assert_eq!(drawn_cols(&gui, ID), 60, "as wide as in the terminal");

        // Typed through GPUI's own key dispatch into the modal.
        gui.keystrokes("t o p i c tab");
        let lines = gui.read(|app| {
            let view = app.modal().unwrap().dialog_view(200).unwrap();
            view.body.iter().map(|l| line_text(l)).collect::<Vec<_>>()
        });
        assert_eq!(lines[1], "topic█");
        assert_eq!(lines[2], "checked out in .worktrees/topic");
        assert!(drawn(&gui, ID).is_some());

        gui.keystrokes("escape");
        assert!(gui.read(|app| app.modal().is_none()));
        assert!(drawn(&gui, ID).is_none(), "gone once closed");
    }

    #[gpui::test]
    fn enter_creates_the_branch(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        // The prompt opens through the app's action queue, so it's open
        // only once that's applied.
        gui.keystrokes("n");
        gui.keystrokes("t o p i c enter");
        assert!(gui.read(|app| app.modal().is_none() && app.error().is_none()));
        assert_eq!(sb.current_branch(), "topic");
    }
}
