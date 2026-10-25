//! The error popup: the message as it is, in a red box. Any key closes it.

use gpui::{AnyElement, IntoElement, div};
use tt_git_app::view::dialogs::error_view;

use super::DialogCx;
use super::prompt::{screen_cols, text_dialog};

/// The id the error popup is measured as.
pub const ID: &str = "dialog:error";

pub fn render(cx: &mut DialogCx) -> AnyElement {
    let cols = screen_cols(cx.ui);
    match cx.app.error() {
        Some(message) => text_dialog(cx.ui, ID, error_view(message, cols)),
        None => div().into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
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
}
