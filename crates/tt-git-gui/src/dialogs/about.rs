//! The About dialog (`Modal::About`, from the app menu's About or the
//! palette): the app icon, the version, who made it, and links to the
//! source, the terminal app and GPUI. A link opens in the system browser.
//!
//! The text it shares with the terminal's is in
//! [`tt_git_app::view::dialogs::about`]. Its keys go to the app as every
//! key does: esc, enter, `q` or space close it, other keys wait. A click
//! outside the box closes it too, by sending the app esc.

use gpui::{AnyElement, Div, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString, Styled, div};
use tt_git_app::keymap::{KeyCode, KeyCombo};
use tt_git_app::view::StyleRole;
use tt_git_app::view::dialogs::DialogLook;
use tt_git_app::view::dialogs::about::{
    ABOUT_DEVELOPER, ABOUT_HINT, ABOUT_TAGLINE, ABOUT_TITLE, REPO_URL, about_heading,
};

use super::prompt::{blank, dialog_box, layer, title};
use crate::menus::icon;
use crate::ui::Ui;

/// The id the dialog is measured as.
pub const ID: &str = "dialog:about";

/// The id the GitHub link is measured as.
pub const REPO_LINK: &str = "dialog:about:repo";

/// The id the GPUI link is measured as.
pub const GPUI_LINK: &str = "dialog:about:gpui";

/// The UI framework the app is drawn with.
pub const GPUI_URL: &str = "https://gpui.rs/";

/// The dialog's width in columns, the border included.
const WIDTH: usize = 56;

pub fn render(ui: &Ui) -> AnyElement {
    let look = DialogLook::Normal;
    let muted = ui.theme.fg(StyleRole::Muted);
    let dialog = dialog_box(ui, ID, look, WIDTH, title(ui, ABOUT_TITLE, look).element())
        // A click inside isn't one outside.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(content(ui))
        .child(blank(ui))
        .child(div().text_color(muted).child(ABOUT_HINT));
    let model = ui.model.clone();
    layer(dialog)
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            if let Some(model) = model.upgrade() {
                model.update(cx, |m, cx| m.key(KeyCombo::plain(KeyCode::Esc), cx));
            }
        })
        .into_any_element()
}

/// What About says, in the About dialog and on the keys popup's About tab:
/// the icon, the version, who made it and the links.
pub fn content(ui: &Ui) -> Div {
    let theme = &ui.theme;
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
                .child(about_heading()),
        )
        .child(div().text_center().text_color(muted).child(ABOUT_TAGLINE));

    div()
        .flex()
        .flex_col()
        .child(heading)
        .child(blank(ui))
        .child(line(ABOUT_DEVELOPER.into()))
        .child(
            div()
                .flex()
                .child("Source: ")
                .child(link(ui, REPO_LINK, "github.com/thirtytech/tt-git", REPO_URL)),
        )
        .child(blank(ui))
        .child(line("Prefer the terminal? Run `tt-git` for the TUI.".into()))
        .child(
            div()
                .flex()
                .child("Built with GPUI: ")
                .child(link(ui, GPUI_LINK, "gpui.rs", GPUI_URL)),
        )
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
    use tt_git_app::modal::Modal;
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
        assert_eq!(gui.read(|app| app.modal().cloned()), Some(Modal::About));

        // Keys wait while it's open.
        gui.keystrokes("tab");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Graph);
        assert!(drawn(&gui, ID).is_some());

        gui.keystrokes("escape");
        assert!(drawn(&gui, ID).is_none());
        assert_eq!(gui.read(|app| app.modal().cloned()), None);
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

    #[gpui::test]
    fn the_palette_opens_it_too(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        gui.keystrokes(": a b o u t enter");
        assert!(drawn(&gui, ID).is_some());
        gui.keystrokes("q");
        assert!(drawn(&gui, ID).is_none(), "q closes it");
        assert!(gui.is_open(), "and doesn't quit");
    }
}
