//! What a desktop front end shows on its taskbar button (Windows) while git
//! talks to a remote: busy while a push, pull or fetch runs, an error for a
//! moment after one fails. The app decides it, in ticks; the front end only
//! draws it, asking with [`App::take_job_progress`] after every input.
//!
//! With `[ui] notifications` on, the button's badge can also say the
//! repository needs a hand: [`App::take_repo_alert`].

use super::{App, TICKS_PER_SEC};
use tt_git_core::{InProgress, Status};

use crate::config::Alert;

use crate::modal::GitOp;

/// How long the taskbar shows a failed job before it clears.
const FAILED_HOLD: u64 = 2 * TICKS_PER_SEC;

/// The taskbar button's progress, as [`App::take_job_progress`] hands it out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum JobProgress {
    /// No network job running: no progress shown.
    #[default]
    Idle,
    /// A network job is running. git reports no percentage to tt-git, so
    /// it has no value.
    Busy,
    /// A network job just failed.
    Failed,
}

/// What the repository's badge warns of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RepoAlert {
    #[default]
    None,
    /// A merge, rebase, cherry-pick or revert stopped, waiting to be
    /// continued.
    Stopped(InProgress),
    /// Files with conflicts.
    Conflicts,
}

/// What `status` warns of, with the conflicts and stopped alerts on as
/// given. A conflicted rebase with only `stopped` on shows as stopped.
fn repo_alert(status: Option<&Status>, conflicts: bool, stopped: bool) -> RepoAlert {
    let Some(st) = status else {
        return RepoAlert::None;
    };
    if conflicts && st.entries.iter().any(|e| e.is_conflicted()) {
        return RepoAlert::Conflicts;
    }
    match st.in_progress {
        Some(op) if stopped => RepoAlert::Stopped(op),
        _ => RepoAlert::None,
    }
}

/// What the taskbar was last told, and when a network job last failed.
#[derive(Debug, Default)]
pub(super) struct JobState {
    shown: JobProgress,
    failed_at: Option<u64>,
    alert: RepoAlert,
}

/// Whether `op` talks to a remote (or downloads tt-git), and so shows on the
/// taskbar while it runs. Local writes are over before it would be seen.
pub(super) fn is_network(op: &GitOp) -> bool {
    matches!(
        op,
        GitOp::Push
            | GitOp::Pull
            | GitOp::FetchAll
            | GitOp::PruneGone
            | GitOp::PushTag { .. }
            | GitOp::PushBranch { .. }
            | GitOp::FastForward { .. }
            | GitOp::DeleteRemoteBranch { .. }
            | GitOp::DeleteTag { remote: Some(_), .. }
            | GitOp::SelfUpdate { .. }
    )
}

impl App {
    /// The taskbar's progress if it changed since the last call: busy while
    /// a network job runs, [`JobProgress::Failed`] for two seconds after one
    /// fails (over any still running), then idle again.
    pub fn take_job_progress(&mut self) -> Option<JobProgress> {
        let failed = self
            .jobs
            .failed_at
            .is_some_and(|at| self.ticks.saturating_sub(at) < FAILED_HOLD);
        let now = if failed {
            JobProgress::Failed
        } else if self.busy.iter().any(|b| b.network) {
            JobProgress::Busy
        } else {
            JobProgress::Idle
        };
        if now == self.jobs.shown {
            return None;
        }
        self.jobs.shown = now;
        Some(now)
    }

    /// The repository's alert if it changed since the last call: conflicts,
    /// or an operation stopped part way, as the last status load found and
    /// `[ui.alerts]` lets through. Always [`RepoAlert::None`] with
    /// `[ui] notifications` off.
    pub fn take_repo_alert(&mut self) -> Option<RepoAlert> {
        let ui = &self.config.ui;
        let now = repo_alert(
            self.status.status(),
            ui.alerts_on(Alert::Conflicts),
            ui.alerts_on(Alert::Stopped),
        );
        if now == self.jobs.alert {
            return None;
        }
        self.jobs.alert = now;
        Some(now)
    }

    /// `op` failed: the taskbar shows it, if it's a network job.
    pub(super) fn job_failed(&mut self, op: &GitOp) {
        if is_network(op) {
            self.jobs.failed_at = Some(self.ticks);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_jobs_show_and_local_ones_dont() {
        assert!(is_network(&GitOp::Push));
        assert!(is_network(&GitOp::FetchAll));
        assert!(is_network(&GitOp::PushTag { name: "v1".into() }));
        assert!(is_network(&GitOp::PushBranch {
            name: "feature".into(),
            upstream: None,
        }));
        assert!(is_network(&GitOp::SelfUpdate { tag: "v9".into() }));
        assert!(is_network(&GitOp::DeleteTag {
            name: "v1".into(),
            remote: Some("origin".into()),
        }));
        assert!(!is_network(&GitOp::DeleteTag {
            name: "v1".into(),
            remote: None,
        }));
        assert!(!is_network(&GitOp::Stage(vec!["a.txt".into()])));
    }

    #[test]
    fn each_repo_alert_can_be_turned_off() {
        use tt_git_core::{FileState, StatusEntry};
        let status = Status {
            entries: vec![StatusEntry {
                path: "a.txt".into(),
                orig_path: None,
                index: FileState::Unmerged,
                worktree: FileState::Unmerged,
            }],
            in_progress: Some(InProgress::Rebase),
            ..Status::default()
        };
        let st = Some(&status);
        assert_eq!(repo_alert(st, true, true), RepoAlert::Conflicts);
        assert_eq!(repo_alert(st, false, true), RepoAlert::Stopped(InProgress::Rebase));
        assert_eq!(repo_alert(st, false, false), RepoAlert::None);
        assert_eq!(repo_alert(None, true, true), RepoAlert::None);
    }
}
