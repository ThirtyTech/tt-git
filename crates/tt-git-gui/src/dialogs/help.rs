//! The keys popup (Phase B7). A placeholder until B7 draws it.

use gpui::AnyElement;

use super::DialogCx;
use crate::placeholder;

pub fn render(cx: &mut DialogCx) -> AnyElement {
    placeholder::dialog(
        cx.ui,
        "Keybindings",
        "B7 Dialogs",
        "every key, narrowed to what's typed",
    )
}
