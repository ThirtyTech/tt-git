//! A yes or no question before a git operation (Phase B7). A placeholder until B7 draws it.

use gpui::AnyElement;
use tt_git_app::modal::Modal;

use super::DialogCx;
use crate::placeholder;

pub fn render(cx: &mut DialogCx) -> AnyElement {
    placeholder::dialog(cx.ui, confirm_title(cx), "B7 Dialogs", "the question and its answers")
}

fn confirm_title(cx: &DialogCx) -> String {
    match cx.app.modal() {
        Some(Modal::Confirm { title, .. }) => title.clone(),
        _ => String::new(),
    }
}
