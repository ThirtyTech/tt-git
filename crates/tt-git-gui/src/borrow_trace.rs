//! A backtrace each time GPUI drops a window event because the app was
//! already borrowed. `main` installs it in debug builds.
//!
//! GPUI hands a window's events (a repaint, a move, input) to the app with
//! `try_borrow_mut`. When one arrives while something else is updating the
//! app, the borrow fails: GPUI logs `RefCell already borrowed` and the event
//! is lost. On Windows that happens when a call made during an update pumps
//! messages, as a cross-process COM call does while it waits, and a repaint
//! comes in there (as making a toast once did, `notify/toast.rs`). The log
//! line doesn't say where. [`Layer`] catches it and writes a backtrace to
//! [`FILE`] in the logs folder: the frames under GPUI's callback name the
//! update that held the app.

use std::backtrace::Backtrace;
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;

/// The file the backtraces go to, in the logs folder.
pub const FILE: &str = "borrow-trace.log";

/// What GPUI's error says when the borrow failed.
const BORROWED: &str = "already borrowed";

/// A tracing layer that writes a backtrace for each `already borrowed`
/// error. Symbols take a while to resolve, so a thread of its own writes
/// them, and the UI thread only captures the frames.
pub struct Layer {
    traces: Option<mpsc::Sender<Trace>>,
    writer: Option<JoinHandle<()>>,
}

struct Trace {
    at: SystemTime,
    message: String,
    backtrace: Backtrace,
}

impl Layer {
    /// Appends the backtraces to `path`.
    pub fn new(path: PathBuf) -> Self {
        let (traces, received) = mpsc::channel::<Trace>();
        let writer = std::thread::Builder::new()
            .name("tt-git-borrow-trace".into())
            .spawn(move || {
                for (n, trace) in received.into_iter().enumerate() {
                    append(&path, n + 1, &trace);
                }
            })
            .inspect_err(|e| tracing::warn!("no borrow traces: {e}"))
            .ok();
        Self {
            traces: writer.is_some().then_some(traces),
            writer,
        }
    }
}

impl Drop for Layer {
    /// Writes what's still queued.
    fn drop(&mut self) {
        self.traces = None;
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

impl<S: Subscriber> tracing_subscriber::Layer<S> for Layer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if *event.metadata().level() != Level::ERROR {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        if !message.0.contains(BORROWED) {
            return;
        }
        if let Some(traces) = &self.traces {
            let _ = traces.send(Trace {
                at: SystemTime::now(),
                message: message.0,
                backtrace: Backtrace::force_capture(),
            });
        }
    }
}

/// The `n`th trace, under a header with its time in Unix seconds.
fn append(path: &Path, n: usize, trace: &Trace) {
    let at = trace.at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let text = format!(
        "=== #{n} at {}.{:03}: {}\n{}\n",
        at.as_secs(),
        at.subsec_millis(),
        trace.message,
        trace.backtrace
    );
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()));
    if let Err(e) = written {
        tracing::warn!("couldn't write {}: {e}", path.display());
    }
}

/// An event's message: `log`'s records come through `tracing-log` with it
/// as a `fmt::Arguments`, tracing's own as a `&str` or the same.
#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_string();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::prelude::*;

    use super::*;

    #[test]
    fn a_borrow_error_is_written_with_its_backtrace_and_nothing_else_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let subscriber = tracing_subscriber::registry().with(Layer::new(path.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!("something else failed");
            tracing::warn!("RefCell already borrowed");
            tracing::error!("RefCell already borrowed");
        });
        // The subscriber is dropped, and with it the layer, which waited
        // for its writer.
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("=== #").count(), 1, "{text}");
        assert!(text.starts_with("=== #1 at "), "{text}");
        assert!(text.contains(": RefCell already borrowed\n"), "{text}");
        assert!(
            text.contains("a_borrow_error_is_written_with_its_backtrace"),
            "the backtrace has the frame that logged it:\n{text}"
        );
    }
}
