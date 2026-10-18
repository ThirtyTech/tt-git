//! Background job tracking.
//!
//! Every job runs off the UI thread and reports back by sending an
//! [`Action`]. The tracker counts in-flight jobs so tests can wait until the
//! app is idle instead of sleeping.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
}

impl Ctx {
    pub fn new(tx: UnboundedSender<Action>) -> Self {
        Self {
            tx,
            jobs: JobTracker::default(),
        }
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
    /// runtime.
    pub fn spawn<F>(&self, work: F)
    where
        F: FnOnce() -> Action + Send + 'static,
    {
        self.spawn_streaming(move |_| work());
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
        tokio::task::spawn_blocking(move || {
            let action = work(&ctx);
            // Send before the guard drops so an idle check never sees
            // "no jobs" while the result is still unsent.
            ctx.emit(action);
            drop(guard);
        });
    }
}
