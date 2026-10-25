//! What opens over the panes: the help popup, the app's modal dialog and
//! the error popup. Each kind draws in its own file (all Phase B7); this
//! one only says which file draws what, so nobody needs to edit it.
//!
//! | File | Draws |
//! |---|---|
//! | `help.rs` | the keys popup (`?`) |
//! | `palette.rs` | `Modal::Palette`: the command palette and the pickers |
//! | `prompt.rs` | `Modal::Prompt`: a line of text to type |
//! | `confirm.rs` | `Modal::Confirm`: a yes or no question |
//! | `discard_all.rs` | `Modal::DiscardAll` |
//! | `filter.rs` | `Modal::Filter`: the graph's filter panel |
//! | `error.rs` | the error popup |
//! | `about.rs` | the About dialog, which the shell opens (the app has no `Modal` for it) |
//!
//! Each returns an element laid over the main area (it's inside a
//! `.relative()` container the size of the area under the help bar).

use gpui::AnyElement;
use tt_git_app::App;
use tt_git_app::modal::Modal;

use crate::ui::Ui;

pub mod about;
pub mod confirm;
pub mod discard_all;
pub mod error;
pub mod filter;
pub mod help;
pub mod palette;
pub mod prompt;

/// What a dialog draws with.
pub struct DialogCx<'a> {
    pub app: &'a mut App,
    pub ui: &'a Ui,
}

/// The help popup, while it's open. Drawn below plugin popups.
pub fn help(cx: &mut DialogCx) -> Option<AnyElement> {
    cx.app.shows_help().then(|| help::render(cx))
}

/// The modal dialog, while one is open.
pub fn modal(cx: &mut DialogCx) -> Option<AnyElement> {
    Some(match cx.app.modal()? {
        Modal::Palette { .. } => palette::render(cx),
        Modal::Prompt { .. } => prompt::render(cx),
        Modal::Confirm { .. } => confirm::render(cx),
        Modal::Filter(_) => filter::render(cx),
        Modal::DiscardAll { .. } => discard_all::render(cx),
    })
}

/// The error popup, over everything, while there's an error.
pub fn error(cx: &mut DialogCx) -> Option<AnyElement> {
    cx.app.error().is_some().then(|| error::render(cx))
}
