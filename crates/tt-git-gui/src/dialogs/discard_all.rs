//! Discarding every change (`Modal::DiscardAll`): what to discard, what's
//! lost and what's kept.

use gpui::AnyElement;

use super::DialogCx;

pub fn render(cx: &mut DialogCx) -> AnyElement {
    super::prompt::render(cx)
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use tt_git_app::modal::Modal;
    use tt_git_app::view::dialogs::line_text;
    use tt_git_core::DiscardScope;
    use tt_git_testkit::Sandbox;

    use crate::dialogs::prompt::ID;
    use crate::dialogs::prompt::tests::{drawn, drawn_cols};
    use crate::testing::Gui;

    #[gpui::test]
    fn tab_picks_what_to_discard_and_y_discards_it(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        sb.write("a.txt", "changed");
        sb.write("new.txt", "new");
        let mut gui = Gui::open(cx, &sb);
        gui.press("tab").keystrokes("shift-d");
        let lost = |gui: &Gui| {
            gui.read(|app| {
                let view = app.modal().unwrap().dialog_view(200).unwrap();
                line_text(&view.body[3])
            })
        };
        assert!(matches!(
            gui.read(|app| app.modal().cloned()),
            Some(Modal::DiscardAll {
                scope: DiscardScope::Everything,
                ..
            })
        ));
        assert_eq!(lost(&gui), "Lost: 1 unstaged file, 1 untracked file.");
        assert_eq!(drawn_cols(&gui, ID), 72, "wide, as in the terminal");

        // Untracked files only.
        gui.keystrokes("tab tab tab");
        assert_eq!(lost(&gui), "Lost: 1 untracked file.");
        gui.keystrokes("y");
        assert!(gui.read(|app| app.modal().is_none()));
        assert!(drawn(&gui, ID).is_none());
        assert!(!sb.path().join("new.txt").exists(), "the untracked file went");
        assert_eq!(
            std::fs::read_to_string(sb.path().join("a.txt")).unwrap(),
            "changed",
            "the change stayed"
        );
    }
}
