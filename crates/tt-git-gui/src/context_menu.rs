//! The right-click menu on a pane: the commands that apply to it and the
//! row the click picked, each with its key from tt-git's keymap
//! ([`App::pane_commands`](tt_git_app::App::pane_commands)).
//!
//! [`PaneFrame`](crate::frame::PaneFrame) puts it on every pane. A right
//! click goes to the app first as [`MouseKind::RightClick`], which focuses
//! the pane and picks the row under the pointer, so the menu is built from
//! what the click picked. Choosing an item runs its command through the app
//! as its key would. The menu is gpui-component's: arrows and `enter` move
//! and choose, `esc` or a click elsewhere closes it, and other keys wait
//! until it's closed.
//!
//! [`MouseKind::RightClick`]: tt_git_app::mouse::MouseKind::RightClick

use gpui::component::ActiveTheme as _;
use gpui::component::menu::{PopupMenu, PopupMenuItem};
use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, rems};
use tt_git_app::Action;
use tt_git_app::app::PaneCommand;
use tt_git_app::keymap::Command;

use crate::ui::Ui;

/// Builds the focused pane's menu, for `ContextMenuExt::context_menu`. It
/// runs after the right click reached the app, so it lists the commands
/// for what that click picked; with none (a terminal pane, a popup open)
/// the menu doesn't open.
pub fn builder(ui: Ui) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |menu, _, cx| {
        let Some(model) = ui.model.upgrade() else {
            return menu;
        };
        let commands = model.read(cx).app().pane_commands();
        // 220 px at 100%, zoomed with the rem size.
        commands
            .into_iter()
            .fold(menu.min_w(cx.theme().font_size * 13.75), |menu, command| {
                menu.item(item(&ui, command))
            })
    }
}

/// One command: its label, and its key at the right.
fn item(ui: &Ui, PaneCommand { command, label, key }: PaneCommand) -> PopupMenuItem {
    let ui = ui.clone();
    PopupMenuItem::element(move |_, cx| {
        div()
            .w_full()
            .flex()
            .flex_row()
            .justify_between()
            .gap(rems(1.5))
            .child(label.clone())
            .child(div().text_color(cx.theme().muted_foreground).child(key.clone()))
            .into_any_element()
    })
    .on_click(move |_, _, cx| run(&ui, command, cx))
}

/// Runs `command` through the app, as its key does.
pub fn run(ui: &Ui, command: Command, cx: &mut gpui::App) {
    if let Some(model) = ui.model.upgrade() {
        model.update(cx, |model, cx| model.send(Action::Command(command), cx));
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Focusable as _, Modifiers, MouseButton, TestAppContext, point};
    use tt_git_app::panes::PaneId;
    use tt_git_testkit::Sandbox;

    use crate::panes::branches::ROWS;
    use crate::testing::Gui;

    fn menu_open(gui: &mut Gui) -> bool {
        let shell = gui.shell.clone();
        gui.cx.update(|window, cx| {
            let focus = shell.read(cx).focus_handle(cx);
            !focus.is_focused(window) && focus.contains_focused(window, cx)
        })
    }

    #[gpui::test]
    fn a_right_click_opens_the_rows_commands_and_runs_the_one_chosen(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        sb.git(&["branch", "alpha"]);
        let mut gui = Gui::open(cx, &sb);
        let row = gui.read(|app| app.branches().visible().position(|b| b.name == "alpha").unwrap());
        let (list, line_height) = gui.shell.read_with(&gui.cx, |shell, _| {
            let ui = shell.ui();
            (ui.measures.get(ROWS).expect("drawn"), ui.font.line_height)
        });
        let at = point(
            list.left() + gpui::px(20.),
            list.top() + line_height * (row as f32 + 0.5),
        );
        gui.cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
        gui.cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
        gui.wait_idle();
        // The click picked the row, then the menu opened over it.
        assert_eq!(gui.read(|app| app.focus()), PaneId::Branches);
        assert_eq!(gui.read(|app| app.branches().cursor().selected), row);
        assert!(menu_open(&mut gui), "the menu has the focus");
        // Keys go to the menu, not the pane, while it's open.
        gui.keystrokes("j");
        assert_eq!(gui.read(|app| app.branches().cursor().selected), row);

        // Its first item is the pane's first command: checkout.
        let first = gui.read(|app| app.pane_commands()[0].clone());
        assert_eq!((first.label.as_str(), first.key.as_str()), ("Checkout", "enter"));
        gui.keystrokes("down enter");
        assert!(!menu_open(&mut gui), "choosing closes it");
        gui.wait_idle();
        assert_eq!(sb.git(&["branch", "--show-current"]).trim(), "alpha");
        // And the keys reach the app again.
        gui.keystrokes("tab");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Graph);
    }

    #[gpui::test]
    fn escape_closes_the_menu_without_running_anything(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        let graph = gui.read(|app| app.pane_area(PaneId::Graph).unwrap());
        let at = point(
            gpui::px(f32::from(graph.x + 40)),
            gpui::px(f32::from(graph.y + graph.height / 2)),
        );
        gui.cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
        gui.wait_idle();
        assert!(menu_open(&mut gui));
        gui.keystrokes("escape");
        assert!(!menu_open(&mut gui));
        assert!(gui.read(|app| app.modal().is_none() && app.zoom() == tt_git_app::app::Zoom::Off));
    }
}
