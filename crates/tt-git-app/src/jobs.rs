//! Background job tracking.
//!
//! Every job runs off the UI thread and reports back by sending an
//! [`Action`]. The tracker counts in-flight jobs so tests can wait until the
//! app is idle instead of sleeping.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedSender;

use crate::action::Action;

#[derive(Debug, Clone, Default)]
pub struct JobTracker(Arc<AtomicUsize>);

impl JobTracker {
    pub fn in_flight(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }

    fn start(&self) -> JobGuard {
        self.0.fetch_add(1, Ordering::SeqCst);
        JobGuard(self.0.clone())
    }
}

/// Decrements the in-flight count when dropped, even if the job panics.
struct JobGuard(Arc<AtomicUsize>);

impl Drop for JobGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Handle given to components so they can emit actions and start jobs
/// without holding a reference to the app.
#[derive(Debug, Clone)]
pub struct Ctx {
    tx: UnboundedSender<Action>,
    jobs: JobTracker,
    /// The runtime jobs run on, shared by every clone once set.
    runtime: Arc<OnceLock<Handle>>,
}

impl Ctx {
    pub fn new(tx: UnboundedSender<Action>) -> Self {
        Self {
            tx,
            jobs: JobTracker::default(),
            runtime: Arc::default(),
        }
    }

    /// Runs jobs on `runtime` from now on, so they can be started from any
    /// thread (a GUI's main thread isn't in a tokio runtime). Until then
    /// they run on the runtime of the thread that starts them. Only the
    /// first call counts.
    pub fn set_runtime(&self, runtime: Handle) {
        let _ = self.runtime.set(runtime);
    }

    pub fn jobs(&self) -> &JobTracker {
        &self.jobs
    }

    /// Queues an action for the next update.
    pub fn emit(&self, action: Action) {
        // The receiver only goes away during shutdown.
        let _ = self.tx.send(action);
    }

    /// Runs blocking work (git commands) on tokio's blocking pool and sends
    /// its resulting action back to the app. Must be called inside a tokio
    /// runtime unless [`set_runtime`](Self::set_runtime) gave it one.
    pub fn spawn<F>(&self, work: F)
    where
        F: FnOnce() -> Action + Send + 'static,
    {
        self.spawn_streaming(move |_| work());
    }

    /// Like [`spawn`](Self::spawn), and returns a flag that's set once the
    /// job has ended (even by panicking), before its action is sent.
    pub fn spawn_watched<F>(&self, work: F) -> JobDone
    where
        F: FnOnce() -> Action + Send + 'static,
    {
        let done = JobDone::default();
        let flag = done.clone();
        self.spawn_streaming(move |_| {
            let _ended = EndGuard(flag);
            work()
        });
        done
    }

    /// Like [`spawn`](Self::spawn), but the job also gets a `Ctx` so it can
    /// emit intermediate actions (e.g. batches of a long log) before its
    /// final one.
    pub fn spawn_streaming<F>(&self, work: F)
    where
        F: FnOnce(&Ctx) -> Action + Send + 'static,
    {
        let guard = self.jobs.start();
        let ctx = self.clone();
        let job = move || {
            let action = work(&ctx);
            // Send before the guard drops so an idle check never sees
            // "no jobs" while the result is still unsent.
            ctx.emit(action);
            drop(guard);
        };
        match self.runtime.get() {
            Some(runtime) => drop(runtime.spawn_blocking(job)),
            None => drop(tokio::task::spawn_blocking(job)),
        }
    }
}

/// Whether a job from [`Ctx::spawn_watched`] has ended.
#[derive(Debug, Clone, Default)]
pub struct JobDone(Arc<AtomicBool>);

impl JobDone {
    pub fn get(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

struct EndGuard(JobDone);

impl Drop for EndGuard {
    fn drop(&mut self) {
        self.0.0.store(true, Ordering::SeqCst);
    }
}

/// Runs loads that follow a selection one at a time, latest wins: while one
/// runs, newer requests replace each other in a single waiting slot, so
/// holding `j` doesn't start a git process per row.
#[derive(Debug)]
pub struct LatestJob<T> {
    running: Option<Running<T>>,
    waiting: Option<T>,
}

#[derive(Debug)]
struct Running<T> {
    item: T,
    done: JobDone,
    /// A forced request came in for `item` after it started, so its
    /// result may be out of date.
    outdated: bool,
}

impl<T> Default for LatestJob<T> {
    fn default() -> Self {
        Self {
            running: None,
            waiting: None,
        }
    }
}

impl<T: PartialEq> LatestJob<T> {
    /// Asks for `item`'s load. Returns it when it should start now (nothing
    /// is running); otherwise it waits for the running one, which also
    /// covers it when it's the same item and `force` is off.
    pub fn request(&mut self, item: T, force: bool) -> Option<T> {
        let Some(run) = self.running.as_mut().filter(|r| !r.done.get()) else {
            // Idle, or the job ended without reporting back (it panicked).
            self.running = None;
            self.waiting = None;
            return Some(item);
        };
        if run.item == item && force {
            run.outdated = true;
        }
        self.waiting = (run.item != item || run.outdated).then_some(item);
        None
    }

    /// Records that `item`'s load started.
    pub fn started(&mut self, item: T, done: JobDone) {
        self.running = Some(Running {
            item,
            done,
            outdated: false,
        });
    }

    /// The running load reported back, if `is_it` says this result is its.
    /// Returns the waiting item, to start next.
    pub fn finished(&mut self, is_it: impl FnOnce(&T) -> bool) -> Option<T> {
        if !self.running.as_ref().is_some_and(|r| is_it(&r.item)) {
            return None;
        }
        self.running = None;
        self.waiting.take()
    }

    /// Whether a load is running (or has ended without being reported).
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }
}

/// Runs a load where only the newest state matters (refs, status): one at a
/// time, and any number of requests while it runs make one more run after
/// it. Generations order the results, so an older one is never applied
/// after a newer one.
#[derive(Debug, Default)]
pub struct Reload {
    requested: u64,
    applied: u64,
    job: LatestJob<u64>,
}

impl Reload {
    /// Asks for a load. Returns its generation when it should start now.
    pub fn request(&mut self) -> Option<u64> {
        self.requested += 1;
        self.job.request(self.requested, false)
    }

    /// Records that load `generation` started.
    pub fn started(&mut self, generation: u64, done: JobDone) {
        self.job.started(generation, done);
    }

    /// The newest load asked for.
    pub fn requested(&self) -> u64 {
        self.requested
    }

    /// The newest load applied.
    pub fn applied(&self) -> u64 {
        self.applied
    }

    /// Load `generation` reported back. Returns whether to apply its result
    /// (it's newer than any applied), and the waiting load to start next.
    pub fn finished(&mut self, generation: u64) -> (bool, Option<u64>) {
        let next = self.job.finished(|g| *g == generation);
        let apply = generation > self.applied;
        if apply {
            self.applied = generation;
        }
        (apply, next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reloads_run_one_at_a_time_with_one_more_after() {
        let mut reload = Reload::default();
        let first = reload.request().unwrap();
        let done = JobDone::default();
        reload.started(first, done.clone());
        assert_eq!(reload.request(), None);
        assert_eq!(reload.request(), None);
        let (apply, next) = reload.finished(first);
        assert!(apply);
        let next = next.expect("one more runs");
        reload.started(next, JobDone::default());
        assert_eq!(reload.finished(next), (true, None));
        // Nothing waits now.
        assert!(reload.request().is_some());
    }

    #[test]
    fn an_older_reload_is_not_applied_after_a_newer_one() {
        let mut reload = Reload::default();
        let first = reload.request().unwrap();
        let done = JobDone::default();
        reload.started(first, done.clone());
        // It ended without being reported yet (or panicked): the next starts.
        done.0.store(true, Ordering::SeqCst);
        let second = reload.request().unwrap();
        reload.started(second, JobDone::default());
        assert_eq!(reload.finished(second), (true, None));
        assert_eq!(reload.finished(first), (false, None));
    }

    fn start(job: &mut LatestJob<u32>, item: u32, force: bool) -> Option<(u32, JobDone)> {
        let item = job.request(item, force)?;
        let done = JobDone::default();
        job.started(item, done.clone());
        Some((item, done))
    }

    #[test]
    fn latest_request_waits_for_the_running_one() {
        let mut job = LatestJob::default();
        assert_eq!(start(&mut job, 1, false).map(|s| s.0), Some(1));
        for i in 2..=5 {
            assert!(start(&mut job, i, false).is_none(), "{i} waits");
        }
        // A result that isn't the running load's changes nothing.
        assert_eq!(job.finished(|i| *i == 4), None);
        assert_eq!(job.finished(|i| *i == 1), Some(5));
        assert!(!job.is_running());
    }

    #[test]
    fn going_back_to_the_running_item_drops_the_waiting_one() {
        let mut job = LatestJob::default();
        start(&mut job, 1, false);
        assert!(start(&mut job, 2, false).is_none());
        assert!(start(&mut job, 1, false).is_none());
        assert_eq!(job.finished(|i| *i == 1), None);
    }

    #[test]
    fn forcing_the_running_item_runs_it_again() {
        let mut job = LatestJob::default();
        start(&mut job, 1, false);
        assert!(start(&mut job, 1, true).is_none());
        assert_eq!(job.finished(|i| *i == 1), Some(1));

        // Still out of date after moving away and back without forcing.
        start(&mut job, 1, false);
        start(&mut job, 1, true);
        start(&mut job, 2, false);
        start(&mut job, 1, false);
        assert_eq!(job.finished(|i| *i == 1), Some(1));
    }

    #[test]
    fn a_job_that_ended_without_reporting_does_not_block() {
        let mut job = LatestJob::default();
        let (_, done) = start(&mut job, 1, false).unwrap();
        assert!(start(&mut job, 2, false).is_none());
        done.0.store(true, Ordering::SeqCst);
        assert_eq!(start(&mut job, 3, false).map(|s| s.0), Some(3));
        assert_eq!(job.finished(|i| *i == 1), None);
        assert_eq!(job.finished(|i| *i == 3), None);
    }
}
