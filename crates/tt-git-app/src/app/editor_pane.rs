//! A terminal editor (vim, nano) run in a pane over the main area, for a
//! front end that can't hand the editor its own terminal (the desktop
//! app). The TUI steps aside and runs it in the terminal instead.
//!
//! The pane takes every key until the editor exits, then closes, and the
//! app gets [`Action::EditorFinished`] with the exit code, as it does from
//! a front end that ran the editor itself.

use super::App;
use crate::action::Action;
use crate::editor::EditorLaunch;
use crate::keymap::KeyCombo;
use crate::pty_pane::{self, PtyPane};

impl App {
    /// Starts `launch` (a terminal editor from [`take_editor`](App::take_editor))
    /// in a pane over the main area. When it exits the pane closes and the
    /// app applies [`Action::EditorFinished`] itself. On an error nothing
    /// was opened: report it with `EditorFinished(Err(..))`.
    pub fn open_editor_pane(&mut self, launch: &EditorLaunch) -> Result<pty_pane::PaneId, String> {
        let (main, _) = self.layout_rects;
        let (rows, cols) = crate::view::terminal::pty_size(main, self.cell_size);
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        let pane = PtyPane::spawn_editor(id, launch, rows, cols, self.ctx.clone())?;
        self.panes.push(pane);
        self.dirty = true;
        Ok(id)
    }

    /// Whether an editor is running in a pane.
    pub fn editor_running(&self) -> bool {
        self.panes.iter().any(|p| p.editor.is_some() && p.exited.is_none())
    }

    /// A key while the editor's pane is on top goes to the editor. The
    /// release key doesn't leave it, since that would lose the edits: the
    /// first press says so, a second in a row stops the editor. Returns
    /// whether the key was taken.
    pub(super) fn editor_pane_key(&mut self, key: KeyCombo) -> bool {
        let release = self.release_keys.contains(&key);
        let hint = self.release_hint();
        let Some(pane) = self.panes.iter_mut().rev().find(|p| p.is_modal()) else {
            return false;
        };
        let Some(run) = pane.editor.as_mut() else {
            return false;
        };
        if !release {
            run.stop_armed = false;
            pane.send_key(key);
            return true;
        }
        if run.stop_armed {
            run.stopped = true;
            pane.kill();
        } else {
            run.stop_armed = true;
            let msg = format!(
                "{} is still running: quit it to come back, or press {hint} again to stop it",
                run.program
            );
            self.notice = Some(msg);
        }
        true
    }

    /// The editor in pane `id` exited: closes its pane and reports how it
    /// ended. Returns false when `id` isn't an editor's pane.
    pub(super) fn editor_pane_exited(&mut self, id: pty_pane::PaneId) -> bool {
        let Some(pane) = self.panes.iter().find(|p| p.id == id) else {
            return false;
        };
        let Some(run) = pane.editor.clone() else {
            return false;
        };
        let code = pane.exit_code();
        // Not `close_pane`: that's for closing it before it exits.
        self.panes.retain(|p| p.id != id);
        self.flush_details();
        if run.stopped {
            self.notice = Some(format!("stopped {}", run.program));
            self.update(Action::EditorFinished(Ok(None)));
        } else {
            self.update(Action::EditorFinished(Ok(code)));
        }
        true
    }

    /// Pane `id` is being closed before its process exits (a plugin closed
    /// it). If it's the editor's, the editor is stopped with it: say so,
    /// and report it finished, since its exit won't be.
    pub(super) fn editor_pane_closing(&mut self, id: pty_pane::PaneId) {
        if let Some(run) = self.panes.iter().find(|p| p.id == id).and_then(|p| p.editor.as_ref()) {
            self.notice = Some(format!("stopped {}", run.program));
            self.ctx.emit(Action::EditorFinished(Ok(None)));
        }
    }
}
