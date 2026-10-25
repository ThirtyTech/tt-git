//! A prompt for a line of text (Phase B7). A placeholder until B7 draws it.

use gpui::AnyElement;
use tt_git_app::modal::Modal;

use super::DialogCx;
use crate::placeholder;

pub fn render(cx: &mut DialogCx) -> AnyElement {
    placeholder::dialog(cx.ui, prompt_title(cx), "B7 Dialogs", "the line being typed")
}

fn prompt_title(cx: &DialogCx) -> String {
    match cx.app.modal() {
        Some(Modal::Prompt { title, .. }) => title.clone(),
        _ => String::new(),
    }
}
