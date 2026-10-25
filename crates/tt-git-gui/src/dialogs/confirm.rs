//! A yes or no question before a git operation (`Modal::Confirm`): its
//! message and the keys that answer it (`y`, a second answer when it has
//! one, `n` or esc).

use gpui::AnyElement;

use super::DialogCx;

pub fn render(cx: &mut DialogCx) -> AnyElement {
    super::prompt::render(cx)
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use tt_git_app::modal::{GitOp, Modal};
    use tt_git_app::panes::PaneId;
    use tt_git_app::view::dialogs::line_text;
    use tt_git_testkit::Sandbox;

    use crate::dialogs::prompt::ID;
    use crate::dialogs::prompt::tests::{drawn, drawn_cols};
    use crate::testing::Gui;

    fn sandbox() -> Sandbox {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        sb.branch("topic");
        sb
    }

    /// Opens the confirm for deleting branch `topic`.
    fn ask_to_delete(gui: &mut Gui) {
        gui.press("3");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Branches);
        let rows = gui.read(|app| app.branches().entries().len());
        // Down to topic, whichever row it's on.
        for _ in 0..rows {
            let on_topic = gui.read(|app| app.branches().selected().is_some_and(|b| b.name == "topic"));
            if on_topic {
                break;
            }
            gui.press("j");
        }
        gui.keystrokes("d");
    }

    #[gpui::test]
    fn asks_before_deleting_a_branch(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        ask_to_delete(&mut gui);
        let (title, message, hint) = gui.read(|app| {
            let modal = app.modal().expect("a confirm");
            assert!(
                matches!(
                    modal,
                    Modal::Confirm {
                        op: GitOp::DeleteBranch { .. },
                        ..
                    }
                ),
                "{modal:?}"
            );
            let view = modal.dialog_view(200).unwrap();
            let body: Vec<String> = view.body.iter().map(|l| line_text(l)).collect();
            (view.title, body.join("\n"), view.hint.unwrap())
        });
        assert!(
            title.contains("topic") || message.contains("topic"),
            "{title} {message}"
        );
        assert!(hint.starts_with("y confirm"), "{hint}");
        assert_eq!(drawn_cols(&gui, ID), 60);

        gui.keystrokes("n");
        assert!(gui.read(|app| app.modal().is_none()), "n says no");
        assert!(drawn(&gui, ID).is_none());
        assert!(sb.git(&["branch", "--list", "topic"]).contains("topic"));

        ask_to_delete(&mut gui);
        gui.keystrokes("y");
        assert!(gui.read(|app| app.modal().is_none()));
        assert!(sb.git(&["branch", "--list", "topic"]).trim().is_empty(), "deleted");
    }
}
