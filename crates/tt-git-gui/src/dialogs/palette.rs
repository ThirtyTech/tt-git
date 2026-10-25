//! The command palette and the pickers (Phase B7). A placeholder until B7 draws it.

use gpui::AnyElement;
use tt_git_app::modal::Modal;

use super::DialogCx;
use crate::placeholder;

pub fn render(cx: &mut DialogCx) -> AnyElement {
    placeholder::dialog(
        cx.ui,
        palette_title(cx),
        "B7 Dialogs",
        "the list to pick from, filtered as you type",
    )
}

fn palette_title(cx: &DialogCx) -> String {
    match cx.app.modal() {
        Some(Modal::Palette { title, .. }) => title.clone(),
        _ => String::new(),
    }
}
