//! The error popup: the message as it is, in a red box. Esc, enter or `q`
//! closes it.
//!
//! The mouse selects the message, for ctrl+c, cmd+c on macOS, or `c` to
//! copy; with nothing selected they copy all of it. The lines wrap, and in
//! the native style they're in a proportional font, so where the mouse is
//! comes from each line's text layout rather than from rows and columns as
//! in [`StyledRows`](crate::rows::StyledRows).

use std::rc::Rc;

use gpui::{
    AnyElement, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Point, SharedString, Styled, TextLayout, div,
};
use tt_git_app::Action;
use tt_git_app::panes::{TextPos, col_at};
use tt_git_app::view::dialogs::error_view;

use super::DialogCx;
use super::prompt::{layer, screen_cols, text_box, wrapped_line};
use crate::text::StyledLine;

/// The id the error popup is measured as.
pub const ID: &str = "dialog:error";

/// The id the message is measured as.
pub const TEXT: &str = "dialog:error:text";

/// A line of the message and where it was laid out.
struct DrawnLine {
    text: SharedString,
    layout: TextLayout,
}

pub fn render(cx: &mut DialogCx) -> AnyElement {
    let cols = screen_cols(cx.ui);
    let selection = cx.app.error_selection().cloned();
    let Some(message) = cx.app.error() else {
        return div().into_any_element();
    };
    let ui = cx.ui;
    let view = error_view(message, cols);
    let mut drawn = Vec::with_capacity(view.body.len());
    let mut rows = Vec::with_capacity(view.body.len());
    for (i, spans) in view.body.iter().enumerate() {
        let mut line = StyledLine::new(&ui.theme, spans, None);
        if let Some(bytes) = selection.as_ref().and_then(|s| s.bytes_on(i)) {
            line.select(&ui.theme, bytes);
        }
        let text = line.element();
        drawn.push(DrawnLine {
            layout: text.layout().clone(),
            text: line.text,
        });
        rows.push(wrapped_line(ui, text));
    }
    let drawn = Rc::new(drawn);
    let press = {
        let (ui, drawn) = (ui.clone(), drawn.clone());
        move |ev: &MouseDownEvent, _: &mut gpui::Window, cx: &mut gpui::App| {
            cx.stop_propagation();
            if let Some(at) = text_pos(&drawn, ev.position) {
                ui.send(Action::SelectError { at, extend: false }, cx);
            }
        }
    };
    let drag = {
        let ui = ui.clone();
        move |ev: &MouseMoveEvent, _: &mut gpui::Window, cx: &mut gpui::App| {
            if ev.pressed_button != Some(MouseButton::Left) || !ui.selecting(cx) {
                return;
            }
            if let Some(at) = text_pos(&drawn, ev.position) {
                ui.send(Action::SelectError { at, extend: true }, cx);
            }
        }
    };
    let release = {
        let ui = ui.clone();
        move |_: &MouseUpEvent, _: &mut gpui::Window, cx: &mut gpui::App| {
            if ui.selecting(cx) {
                ui.send(Action::SelectEnd, cx);
            }
        }
    };
    let body = div()
        .relative()
        .flex()
        .flex_col()
        .cursor_text()
        .child(ui.measure(TEXT))
        .children(rows)
        .on_mouse_down(MouseButton::Left, press);
    // The whole layer follows the drag, so it can run past the text.
    layer(ui, text_box(ui, ID, &view, body))
        .on_mouse_move(drag)
        .on_mouse_up(MouseButton::Left, release.clone())
        .on_mouse_up_out(MouseButton::Left, release)
        .into_any_element()
}

/// The place in the message nearest window point `at`: above the text is its
/// start and below it its end.
fn text_pos(lines: &[DrawnLine], at: Point<Pixels>) -> Option<TextPos> {
    let line = lines
        .iter()
        .position(|l| at.y < l.layout.bounds().bottom())
        .or(lines.len().checked_sub(1))?;
    let drawn = &lines[line];
    let byte = drawn.layout.index_for_position(at).unwrap_or_else(|nearest| nearest);
    Some(TextPos {
        line,
        col: col_at(&drawn.text, byte.min(drawn.text.len())),
    })
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, TestAppContext, point, px};
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::dialogs::prompt::tests::{drawn, drawn_cols};
    use crate::testing::Gui;

    #[gpui::test]
    fn a_failed_push_shows_its_error_until_a_key(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        // No remote to push to.
        gui.keystrokes("shift-p");
        if gui.read(|app| app.modal().is_some()) {
            gui.keystrokes("y");
        }
        let error = gui.read(|app| app.error().map(str::to_string));
        assert!(error.is_some(), "push fails without a remote");
        assert_eq!(drawn_cols(&gui, ID), 70);
        gui.keystrokes("escape");
        assert!(gui.read(|app| app.error().is_none()));
        assert!(drawn(&gui, ID).is_none());
    }

    #[gpui::test]
    fn the_mouse_selects_the_message_to_copy(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        let message = "first line\nsecond line\nthird";
        gui.model
            .update(&mut gui.cx, |m, cx| m.send(Action::Error(message.into()), cx));
        gui.draw();
        let (text, line) = gui.shell.read_with(&gui.cx, |shell, _| {
            let ui = shell.ui();
            (ui.measures.get(TEXT).expect("drawn"), ui.font.line_height)
        });
        let at = |row: f32, x: Pixels| point(text.left() + x, text.top() + line * (row + 0.5));
        let picked = |gui: &Gui| gui.read(|app| app.error_selection().map(|s| s.text()));
        let clipboard = |gui: &Gui| gui.cx.read_from_clipboard().and_then(|c| c.text());

        // From the start of the first line to the end of the second, the
        // last of it dragged out past the box's edge.
        gui.cx
            .simulate_mouse_down(at(0., px(0.)), MouseButton::Left, Modifiers::none());
        gui.cx
            .simulate_mouse_move(at(1., text.size.width + px(20.)), MouseButton::Left, Modifiers::none());
        gui.cx
            .simulate_mouse_up(at(1., text.size.width + px(20.)), MouseButton::Left, Modifiers::none());
        gui.wait_idle();
        assert_eq!(picked(&gui).as_deref(), Some("first line\nsecond line"));

        // ctrl+c (cmd+c on macOS) copies it and leaves the popup up.
        gui.keystrokes(if cfg!(target_os = "macos") { "cmd-c" } else { "ctrl-c" });
        assert!(gui.is_open(), "still open");
        assert_eq!(clipboard(&gui).as_deref(), Some("first line\nsecond line"));
        assert!(gui.read(|app| app.error().is_some()));

        // A click without a drag clears it, and c copies all of the message.
        gui.cx.simulate_click(at(0., px(0.)), Modifiers::none());
        gui.wait_idle();
        assert_eq!(picked(&gui), None);
        gui.press("c");
        assert_eq!(clipboard(&gui).as_deref(), Some(message));

        gui.keystrokes("escape");
        assert_eq!(picked(&gui), None);
        assert!(gui.read(|app| app.error().is_none()));
    }
}
