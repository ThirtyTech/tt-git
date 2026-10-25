//! Laying things over the main area: dialogs, popups, floating panes.

use gpui::{Div, IntoElement, ParentElement, Styled, div};

/// A layer over the whole main area (the `.relative()` container dialogs
/// are drawn in) with `content` in its middle.
pub fn centered(content: impl IntoElement) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(content)
}
