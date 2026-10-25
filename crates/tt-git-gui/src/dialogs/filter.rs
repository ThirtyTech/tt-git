//! The graph's filter panel (Phase B7). A placeholder until B7 draws it.

use gpui::AnyElement;

use super::DialogCx;
use crate::placeholder;

pub fn render(cx: &mut DialogCx) -> AnyElement {
    placeholder::dialog(
        cx.ui,
        "Filter",
        "B7 Dialogs",
        "the authors and files to narrow the graph to",
    )
}
