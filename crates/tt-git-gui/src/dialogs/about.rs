//! The About dialog (the app menu's About): the app icon, the version, who
//! made it, and links to the source, the terminal app and GPUI. A link
//! opens in the system browser.
//!
//! The TUI has no About, so the app has no `Modal` for it: the shell keeps
//! whether it's open and closes it. esc, enter, `q` or space close it, as
//! does a click outside the box; other keys wait until it's closed.

use std::rc::Rc;

use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString, Styled, Window, div,
};
use tt_git_app::view::StyleRole;
use tt_git_app::view::dialogs::DialogLook;

use super::prompt::{blank, dialog_box, layer, title};
use crate::menus::icon;
use crate::ui::Ui;

/// The id the dialog is measured as.
pub const ID: &str = "dialog:about";

/// The id the GitHub link is measured as.
pub const REPO_LINK: &str = "dialog:about:repo";

/// The id the GPUI link is measured as.
pub const GPUI_LINK: &str = "dialog:about:gpui";

/// The dialog's title.
pub const TITLE: &str = "About tt-git";

/// tt-git's source.
pub const REPO_URL: &str = "https://github.com/thirtytech/tt-git";

/// The UI framework the app is drawn with.
pub const GPUI_URL: &str = "https://gpui.rs/";

/// The dialog's width in columns, the border included.
const WIDTH: usize = 56;

/// What closes the dialog.
pub type Close = Rc<dyn Fn(&mut Window, &mut App)>;

/// Whether `key` (a GPUI key name) closes the dialog.
pub fn closes(key: &str) -> bool {
    matches!(key, "escape" | "enter" | "q" | "space")
}

pub fn render(ui: &Ui, close: Close) -> AnyElement {
    let theme = &ui.theme;
    let look = DialogLook::Normal;
    let muted = theme.fg(StyleRole::Muted);
    let line = |text: SharedString| div().min_h(ui.font.line_height).whitespace_normal().child(text);

    let heading = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(ui.font.line_height / 2.)
        .pt(ui.font.line_height / 2.)
        .children(icon::about_icon())
        .child(
            div()
                .text_color(theme.fg(StyleRole::TitleFocused))
                .child(format!("tt-git {}", env!("CARGO_PKG_VERSION"))),
        )
        .child(
            div()
                .text_center()
                .text_color(muted)
                .child("A git client with a commit graph, unified branches and plugins."),
        );

    let dialog = dialog_box(ui, ID, look, WIDTH, title(ui, TITLE, look).element())
        // A click inside isn't one outside.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(heading)
        .child(blank(ui))
        .child(line("Developer: Jonathan Sheely @ ThirtyTech Inc".into()))
        .child(
            div()
                .flex()
                .child("Source: ")
                .child(link(ui, REPO_LINK, "github.com/thirtytech/tt-git", REPO_URL)),
        )
        .child(blank(ui))
        .child(line("Prefer the terminal? tt-git is a TUI too: run `tt-git`.".into()))
        .child(
            div()
                .flex()
                .child("Built with GPUI: ")
                .child(link(ui, GPUI_LINK, "gpui.rs", GPUI_URL)),
        )
        .child(blank(ui))
        .child(div().text_color(muted).child("esc close"));
    layer(dialog)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            close(window, cx);
        })
        .into_any_element()
}

/// `text`, underlined, that opens `url` in the system browser when clicked.
/// Measured as `id`.
fn link(ui: &Ui, id: &'static str, text: &'static str, url: &'static str) -> impl IntoElement {
    let color = ui.theme.fg(StyleRole::HelpKey);
    div()
        .id(id)
        .relative()
        .cursor_pointer()
        .underline()
        .text_color(color)
        .child(ui.measure(id))
        .child(text)
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            cx.open_url(url);
        })
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, TestAppContext, point, px};
    use tt_git_app::panes::PaneId;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::dialogs::prompt::tests::{drawn, drawn_cols};
    use crate::menus::About;
    use crate::testing::Gui;

    fn sandbox() -> Sandbox {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        sb
    }

    fn open_about(gui: &mut Gui) {
        gui.cx.dispatch_action(About);
        gui.wait_idle();
    }

    fn click(gui: &mut Gui, at: gpui::Point<gpui::Pixels>) {
        gui.cx.simulate_click(at, Modifiers::none());
        gui.wait_idle();
    }

    #[gpui::test]
    fn about_opens_the_dialog_and_esc_closes_it(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        assert!(drawn(&gui, ID).is_none());
        open_about(&mut gui);
        assert_eq!(drawn_cols(&gui, ID), WIDTH);
        assert!(gui.cx.debug_bounds("about icon").is_some(), "the icon is drawn");

        // Keys wait while it's open.
        gui.keystrokes("tab");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Graph);
        assert!(drawn(&gui, ID).is_some());

        gui.keystrokes("escape");
        assert!(drawn(&gui, ID).is_none());
        gui.keystrokes("tab");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Status, "keys reach the app again");
    }

    #[gpui::test]
    fn the_links_open_in_the_browser(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        open_about(&mut gui);
        let repo = drawn(&gui, REPO_LINK).expect("the GitHub link");
        click(&mut gui, repo.center());
        assert_eq!(gui.cx.opened_url().as_deref(), Some(REPO_URL));
        assert!(drawn(&gui, ID).is_some(), "a click inside leaves it open");

        let gpui_link = drawn(&gui, GPUI_LINK).expect("the GPUI link");
        click(&mut gui, gpui_link.center());
        assert_eq!(gui.cx.opened_url().as_deref(), Some(GPUI_URL));
    }

    #[gpui::test]
    fn a_click_outside_closes_it(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        open_about(&mut gui);
        let dialog = drawn(&gui, ID).unwrap();
        click(&mut gui, point(dialog.left() - px(20.), dialog.top()));
        assert!(drawn(&gui, ID).is_none());
        assert_eq!(
            gui.read(|app| app.focus()),
            PaneId::Graph,
            "the panes behind didn't get it"
        );
    }
}
