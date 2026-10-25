//! The commands that apply to the focused pane, with their keys: what a
//! front end offers in a context menu (or a menu of its own) for the pane
//! and the row picked in it.

use super::App;
use crate::keymap::Command;
use crate::panes::PaneId;

/// A command the focused pane takes, for a menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneCommand {
    pub command: Command,
    /// What it does here, capitalized, as the palette names it (`Checkout`).
    pub label: String,
    /// The key bound to it in the pane's context, as the help bar shows it.
    pub key: String,
}

impl App {
    /// The focused pane's commands that have a key, in the order its help
    /// lists them, each with that key: the commands for what's picked in
    /// it, as a context menu shows them. Running one is
    /// `update(Action::Command(command))`, what its key does. Empty while a
    /// popup or a floating plugin pane owns the screen, and for a plugin
    /// pane split in place of details.
    pub fn pane_commands(&self) -> Vec<PaneCommand> {
        if self.modal.is_some() || self.error.is_some() || self.show_help || self.panes.iter().any(|p| p.is_modal()) {
            return Vec::new();
        }
        if self.focus == PaneId::Details && self.has_split_pane() {
            return Vec::new();
        }
        let pane = self.focused();
        let context = pane.context();
        pane.commands()
            .iter()
            .filter_map(|&command| {
                let key = self.keymap.display_key_text(context, command)?.to_string();
                let mut label = pane.label(command).to_string();
                if let Some(first) = label.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                Some(PaneCommand { command, label, key })
            })
            .collect()
    }
}
