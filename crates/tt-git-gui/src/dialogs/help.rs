//! The keys popup (`?`): every key for the focused pane and the global
//! ones, narrowed to what's typed, and an About tab with what the About
//! dialog says. Sized for the whole list, so it holds still while typing
//! and on either tab.

use gpui::{AnyElement, IntoElement, ParentElement, Styled, div};
use tt_git_app::view::dialogs::{DialogLook, HELP_TITLE, HelpTab};
use tt_git_app::view::{StyleRole, TextSpan};

use super::DialogCx;
use super::about;
use super::prompt::{blank, dialog_box, layer, single_line};
use crate::text::StyledLine;

/// The id the keys popup is measured as.
pub const ID: &str = "dialog:help";

pub fn render(cx: &mut DialogCx) -> AnyElement {
    let ui = cx.ui;
    let view = cx.app.help_view();
    let look = DialogLook::Normal;
    let theme = &ui.theme;
    let title = div()
        .flex()
        .w_full()
        .justify_between()
        .child(StyledLine::new(theme, &[TextSpan::new(HELP_TITLE, look.title())], None).element())
        .child(StyledLine::new(theme, &[TextSpan::new(view.version.as_str(), StyleRole::Muted)], None).element());
    let mut body = div()
        .flex()
        .flex_col()
        .h(ui.font.line_height * view.height.saturating_sub(2) as f32)
        .overflow_hidden()
        .child(single_line(ui, &view.tabs))
        .child(blank(ui));
    if view.tab == HelpTab::About {
        body = body.child(about::content(ui));
    } else {
        body = body.child(single_line(ui, &view.search));
        if let Some(empty) = view.empty {
            body = body.child(single_line(ui, &[TextSpan::new(empty, StyleRole::Muted)]));
        }
        body = body.children(view.rows.iter().map(|row| single_line(ui, row)));
    }
    layer(dialog_box(ui, ID, look, view.width, title).child(body)).into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::dialogs::prompt::tests::{drawn, drawn_cols};
    use crate::testing::Gui;

    #[gpui::test]
    fn the_keys_popup_narrows_and_holds_still(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        gui.keystrokes("?");
        assert!(gui.read(|app| app.shows_help()));
        assert_eq!(drawn_cols(&gui, ID), 50);
        let before = drawn(&gui, ID).unwrap();
        let all = gui.read(|app| app.help_view().rows.len());

        gui.keystrokes("q u i t");
        let rows = gui.read(|app| app.help_view().rows);
        assert!(!rows.is_empty() && rows.len() < all, "{rows:?}");
        assert_eq!(drawn(&gui, ID).unwrap(), before, "the same size and place");

        gui.keystrokes("escape");
        assert!(!gui.read(|app| app.shows_help()));
        assert!(drawn(&gui, ID).is_none());
    }

    #[gpui::test]
    fn tab_shows_about_in_the_same_box(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        gui.keystrokes("?");
        let before = drawn(&gui, ID).unwrap();
        assert!(drawn(&gui, about::REPO_LINK).is_none());

        gui.keystrokes("tab");
        assert_eq!(gui.read(|app| app.help_tab()), HelpTab::About);
        assert!(drawn(&gui, about::REPO_LINK).is_some(), "the About links");
        assert_eq!(drawn(&gui, ID).unwrap(), before, "the same size and place");

        gui.keystrokes("escape");
        assert!(!gui.read(|app| app.shows_help()));
    }
}
