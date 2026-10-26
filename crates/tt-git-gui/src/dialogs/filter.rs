//! The graph's filter panel (`Modal::Filter`): the author and file tabs,
//! what's typed and the choices that match. A fixed height, as in the
//! terminal, so it doesn't jump about while typing.
//!
//! It takes the mouse as the terminal's does: a click on a tab switches to
//! it, on a choice applies it, outside the panel closes it, and the wheel
//! moves the selection. Each goes to the app as a `MouseTarget::Filter`.

use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, ScrollDelta,
    ScrollWheelEvent, Styled, Window, div,
};
use tt_git_app::filter::ROWS;
use tt_git_app::modal::Modal;
use tt_git_app::mouse::{FilterSpot, MouseInput, MouseKind, MouseTarget};
use tt_git_app::view::dialogs::{DialogLook, FILTER_HINT, FILTER_TITLE};
use tt_git_app::view::{StyleRole, TextSpan};

use super::DialogCx;
use super::prompt::{blank, dialog_box, layer, list_row, screen_cols, single_line, title};
use crate::rows::WHEEL_STEP_LINES;
use crate::text::StyledLine;
use crate::ui::Ui;

/// The id the panel is measured as.
pub const ID: &str = "dialog:filter";

/// The id tab `n` is measured as.
pub fn tab_id(n: usize) -> String {
    format!("dialog:filter:tab:{n}")
}

/// The id the rows of choices are measured as.
pub const CHOICES: &str = "dialog:filter:choices";

pub fn render(cx: &mut DialogCx) -> AnyElement {
    let ui = cx.ui;
    let Some(Modal::Filter(panel)) = cx.app.modal() else {
        return div().into_any_element();
    };
    let view = panel.view(screen_cols(ui));
    let look = DialogLook::Normal;
    let line_height = ui.font.line_height;

    let mut tabs = div()
        .flex()
        .flex_none()
        .h(line_height)
        .overflow_hidden()
        .whitespace_nowrap();
    for (i, (tab, span)) in view.tabs.iter().enumerate() {
        if i > 0 {
            tabs = tabs.child(StyledLine::new(&ui.theme, &[TextSpan::new(" · ", StyleRole::Muted)], None).element());
        }
        tabs = tabs.child(
            div()
                .id(tab_id(i))
                .relative()
                .child(ui.measure(tab_id(i)))
                .on_mouse_down(MouseButton::Left, click(ui, FilterSpot::Tab(*tab)))
                .child(StyledLine::new(&ui.theme, std::slice::from_ref(span), None).element()),
        );
    }

    let mut choices = div()
        .id(CHOICES)
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .h(line_height * ROWS as f32)
        .overflow_hidden()
        .child(ui.measure(CHOICES));
    if let Some(message) = &view.message {
        choices = choices.child(single_line(ui, std::slice::from_ref(message)));
    } else {
        // A slot per row, so a click under the last choice is a choice
        // row too, as the terminal counts it.
        for n in 0..ROWS {
            let row = match view.rows.get(n) {
                Some(row) => list_row(ui, row),
                None => blank(ui),
            };
            choices = choices.child(
                div()
                    .id(("filter-choice", n))
                    .on_mouse_down(MouseButton::Left, click(ui, FilterSpot::Choice(n)))
                    .child(row),
            );
        }
    }

    let panel = dialog_box(ui, ID, look, view.width, title(ui, FILTER_TITLE, look).element())
        .on_mouse_down(MouseButton::Left, click(ui, FilterSpot::Nothing))
        .on_scroll_wheel(wheel(ui))
        .child(tabs)
        .child(single_line(ui, &view.input))
        .child(blank(ui))
        .child(choices)
        .child(blank(ui))
        .child(single_line(ui, &[TextSpan::new(FILTER_HINT, StyleRole::Muted)]));
    layer(ui, panel)
        .on_mouse_down(MouseButton::Left, click(ui, FilterSpot::Outside))
        .into_any_element()
}

/// Sends a click on `spot` to the app, and nothing behind it.
fn click(ui: &Ui, spot: FilterSpot) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    let ui = ui.clone();
    move |ev, _, cx| {
        cx.stop_propagation();
        ui.send_mouse(
            MouseInput {
                kind: MouseKind::Click,
                ctrl: ev.modifiers.control,
                target: MouseTarget::Filter(spot),
            },
            cx,
        );
    }
}

/// Turns the wheel into scroll steps for the app, which moves the selection.
fn wheel(ui: &Ui) -> impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static {
    let ui = ui.clone();
    move |ev, _, cx| {
        cx.stop_propagation();
        let lines = match ev.delta {
            ScrollDelta::Lines(d) => d.y,
            ScrollDelta::Pixels(d) => d.y / ui.font.line_height,
        };
        let steps = ui.measures.wheel_steps(&ID.into(), lines, WHEEL_STEP_LINES);
        let kind = if steps > 0 {
            MouseKind::ScrollUp
        } else {
            MouseKind::ScrollDown
        };
        for _ in 0..steps.unsigned_abs() {
            ui.send_mouse(
                MouseInput {
                    kind,
                    ctrl: ev.modifiers.control,
                    target: MouseTarget::Filter(FilterSpot::Nothing),
                },
                cx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, TestAppContext, point, px};
    use tt_git_app::filter::FilterTab;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::dialogs::prompt::tests::{drawn, drawn_cols};
    use crate::testing::Gui;

    fn sandbox() -> Sandbox {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        sb.commit_file("b.txt", "b", "second");
        sb
    }

    fn tab(gui: &Gui) -> Option<FilterTab> {
        gui.read(|app| match app.modal() {
            Some(Modal::Filter(panel)) => Some(panel.tab()),
            _ => None,
        })
    }

    fn click(gui: &mut Gui, at: gpui::Point<gpui::Pixels>) {
        gui.cx.simulate_click(at, Modifiers::none());
        gui.wait_idle();
    }

    #[gpui::test]
    fn opens_on_the_authors_and_tab_switches(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        gui.keystrokes("f");
        assert_eq!(tab(&gui), Some(FilterTab::Author));
        assert_eq!(drawn_cols(&gui, ID), 70);
        let size = drawn(&gui, ID).unwrap().size;
        let rows = gui.read(|app| match app.modal() {
            Some(Modal::Filter(panel)) => panel.view(200).rows.len(),
            _ => 0,
        });
        assert!(rows >= 1, "the authors loaded");
        gui.keystrokes("tab");
        assert_eq!(tab(&gui), Some(FilterTab::File));
        assert_eq!(drawn(&gui, ID).unwrap().size, size, "a fixed size");
        gui.keystrokes("escape");
        assert_eq!(tab(&gui), None);
        assert!(drawn(&gui, ID).is_none());
    }

    #[gpui::test]
    fn clicks_pick_a_tab_a_choice_or_close_it(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        gui.keystrokes("f");
        let file_tab = drawn(&gui, &tab_id(1)).expect("the file tab");
        click(&mut gui, file_tab.center());
        assert_eq!(tab(&gui), Some(FilterTab::File));

        // The first choice on the file tab is a.txt.
        let choices = drawn(&gui, CHOICES).unwrap();
        let line = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().font.line_height);
        click(&mut gui, point(choices.center().x, choices.top() + line / 2.));
        assert_eq!(tab(&gui), None, "picking a choice closes the panel");
        let path = gui.read(|app| app.graph().filter.path.clone());
        assert_eq!(path.as_deref(), Some("a.txt"));

        // A click outside closes it without changing the filter.
        gui.keystrokes("f");
        assert!(tab(&gui).is_some());
        let panel = drawn(&gui, ID).unwrap();
        click(&mut gui, point(panel.left() - px(20.), panel.top()));
        assert_eq!(tab(&gui), None);
        let path = gui.read(|app| app.graph().filter.path.clone());
        assert_eq!(path.as_deref(), Some("a.txt"));
    }
}
