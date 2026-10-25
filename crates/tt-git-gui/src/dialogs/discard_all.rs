//! Discarding every change (Phase B7). A placeholder until B7 draws it.

use gpui::AnyElement;

use super::DialogCx;
use crate::placeholder;

pub fn render(cx: &mut DialogCx) -> AnyElement {
    placeholder::dialog(cx.ui, "Discard all", "B7 Dialogs", "what will be discarded")
}
