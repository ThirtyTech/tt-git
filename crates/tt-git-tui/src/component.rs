//! The pane abstraction.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};

use crate::jobs::Ctx;
use crate::keymap::{Command, Context, KeyCombo};
use crate::theme::Theme;

/// Whether a component handled a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventState {
    Consumed,
    NotConsumed,
}

/// A pane. Components receive already-resolved [`Command`]s (never raw keys)
/// and talk to the rest of the app only through [`Ctx`].
pub trait Component {
    /// Key binding context used while this component is focused.
    fn context(&self) -> Context;

    /// Handles a command while focused.
    fn on_command(&mut self, _cmd: Command, _ctx: &Ctx) -> EventState {
        EventState::NotConsumed
    }

    /// Commands shown in the help bar while focused.
    fn commands(&self) -> &[Command] {
        &[]
    }

    /// While true, raw key presses go to [`on_text_key`](Self::on_text_key)
    /// instead of being resolved through the keymap (e.g. a filter being typed).
    fn wants_text_input(&self) -> bool {
        false
    }

    fn on_text_key(&mut self, _key: KeyCombo, _ctx: &Ctx) {}

    /// A key press tried before the keymap, for keys the pane reads itself
    /// (blame's vim-style line count). Returns whether it was used.
    fn on_key(&mut self, _key: KeyCombo, _ctx: &Ctx) -> bool {
        false
    }

    /// A click on the `row`-th visible line of the pane's body.
    fn on_click(&mut self, _row: usize, _ctx: &Ctx) {}

    /// A click at screen cell `pos`, tried before [`on_click`](Self::on_click)
    /// for panes with more than one list, or tabs in their title. Returns
    /// whether it was handled.
    fn on_click_at(&mut self, _pos: Position, _ctx: &Ctx) -> bool {
        false
    }

    /// The mouse wheel at screen cell `pos`, tried before the pane gets
    /// [`Command::MoveUp`] or [`Command::MoveDown`]. Returns whether it was
    /// handled.
    fn on_scroll_at(&mut self, _pos: Position, _down: bool, _ctx: &Ctx) -> bool {
        false
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme);
}
