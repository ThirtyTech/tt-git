//! The app's side of fetching on a timer ([`crate::bg_fetch`]): when to
//! check, and what to make of the answer. A background fetch says nothing
//! when it works and only logs when it fails; it shows no spinner and
//! nothing on the taskbar.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use serde_json::json;

use super::{App, TICKS_PER_SEC};
use crate::action::Action;
use crate::bg_fetch::{self, AsConfigured, Conserve, Outcome, Policy, Store};
use crate::modal::GitOp;

pub(super) struct BackgroundFetch {
    /// Where the lock and the time of the last attempt go.
    dir: PathBuf,
    /// Set by [`App::set_fetch_policy`]; otherwise it follows
    /// `[fetch] conserve`.
    policy: Option<Arc<dyn Policy>>,
    /// [`App::ticks`] of the next check.
    next_check: u64,
    /// A check is running.
    running: bool,
    /// The user asked to fetch while the check was running, so its fetch
    /// answers for theirs.
    joined: bool,
}

impl Default for BackgroundFetch {
    fn default() -> Self {
        Self {
            dir: tt_git_core::paths::state_dir().join("fetch"),
            policy: None,
            next_check: bg_fetch::FIRST_CHECK.as_secs() * TICKS_PER_SEC,
            running: false,
            joined: false,
        }
    }
}

impl App {
    /// Keeps background fetching's lock and timestamps in `dir` instead of
    /// `<state_dir>/fetch` (tests use a sandbox's).
    pub fn set_fetch_state_dir(&mut self, dir: PathBuf) {
        self.background_fetch.dir = dir;
    }

    /// Stretches or pauses background fetching by `policy` instead of
    /// `[fetch] conserve`'s.
    pub fn set_fetch_policy(&mut self, policy: Arc<dyn Policy>) {
        self.background_fetch.policy = Some(policy);
    }

    /// On a tick: starts a check when one is due, unless one is running
    /// or the user's own fetch, pull or push is.
    pub(super) fn poll_background_fetch(&mut self) {
        let Some(base) = self.config.fetch.background_interval() else {
            return;
        };
        let bg = &mut self.background_fetch;
        if self.ticks < bg.next_check || bg.running {
            return;
        }
        bg.next_check = self.ticks + bg_fetch::CHECK_EVERY.as_secs() * TICKS_PER_SEC;
        if self.busy.iter().any(|b| b.network) {
            return;
        }
        bg.running = true;
        let repo = self.repo.clone();
        let policy: Arc<dyn Policy> = match &bg.policy {
            Some(policy) => policy.clone(),
            None if self.config.fetch.conserve => Arc::new(Conserve::system()),
            None => Arc::new(AsConfigured),
        };
        let store = Store::new(&bg.dir, self.repo.common_dir());
        self.ctx
            .spawn(move || Action::BackgroundFetched(bg_fetch::run(&repo, base, &*policy, &store, SystemTime::now())));
    }

    /// The user's fetch, already on the busy list, waits for the check
    /// that's running rather than racing it. `false` when none is.
    pub(super) fn join_background_fetch(&mut self) -> bool {
        let bg = &mut self.background_fetch;
        bg.joined = bg.running;
        bg.joined
    }

    pub(super) fn background_fetched(&mut self, outcome: Outcome) {
        self.background_fetch.running = false;
        if std::mem::take(&mut self.background_fetch.joined) {
            match outcome {
                Outcome::Fetched(Ok(out)) => self.op_done(GitOp::FetchAll, Ok(out), None),
                // No fetch, or a quiet one that failed: the user's fetch
                // runs now as any other does (and may ask to sign in).
                _ => {
                    self.finish_busy(&GitOp::FetchAll);
                    self.run_op(GitOp::FetchAll);
                }
            }
            return;
        }
        match outcome {
            Outcome::Fetched(Ok(_)) => {
                self.emit_event("fetch.completed", json!({ "background": true }));
                self.update(Action::Refresh);
            }
            Outcome::Fetched(Err(e)) => tracing::warn!("background fetch failed: {e}"),
            Outcome::NotDue | Outcome::Paused => {}
        }
    }
}
