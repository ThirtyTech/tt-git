//! The pane abstraction.

use crate::jobs::Ctx;
use crate::keymap::{Command, Context, KeyCombo};
use crate::mouse::PanePart;

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

    /// What the help bar calls `cmd` here, when what it does depends on
    /// what the pane shows.
    fn label(&self, cmd: Command) -> &'static str {
        cmd.label()
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

    /// What a double click on the `row`-th visible line of the pane's body
    /// runs, after the first click picked it: the command `enter` runs on
    /// it, when there's something there.
    fn double_click(&self, _row: usize) -> Option<Command> {
        None
    }

    /// A click on `part` of the pane, tried before [`on_click`](Self::on_click)
    /// for panes with more than one list, or tabs in their title. Returns
    /// whether it was handled.
    fn on_click_part(&mut self, _part: PanePart, _ctx: &Ctx) -> bool {
        false
    }

    /// The mouse wheel over `part` of the pane, tried before the pane gets
    /// [`Command::MoveUp`] or [`Command::MoveDown`]. Returns whether it was
    /// handled.
    fn on_scroll_part(&mut self, _part: PanePart, _down: bool, _ctx: &Ctx) -> bool {
        false
    }
}
