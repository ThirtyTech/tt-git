//! Running the git CLI.
//!
//! Every invocation goes through [`GitCli`], which pins output-affecting
//! config, disables interactive prompts on the terminal and records the
//! command in a [`CommandLog`] for the UI's command-log view.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// Config pinned on every call so output is parseable regardless of the
/// user's settings.
const PINNED_CONFIG: &[&str] = &[
    "-c",
    "color.ui=never",
    "-c",
    "core.quotepath=false",
    "-c",
    "log.showSignature=false",
    "-c",
    "core.pager=cat",
];

const LOG_CAPACITY: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("could not run git: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("{} is not inside a git repository", .0.display())]
    NotARepo(PathBuf),
    #[error("git {args} failed ({code}): {stderr}")]
    Failed { args: String, code: String, stderr: String },
    #[error("unexpected git output: {0}")]
    Parse(String),
}

impl GitError {
    /// git's stderr for a failed command, for showing to the user.
    pub fn stderr(&self) -> Option<&str> {
        match self {
            GitError::Failed { stderr, .. } => Some(stderr),
            _ => None,
        }
    }
}

pub type Result<T, E = GitError> = std::result::Result<T, E>;

/// One finished (or running) git invocation.
#[derive(Debug, Clone)]
pub struct LoggedCommand {
    pub args: String,
    pub started: SystemTime,
    pub duration: Option<Duration>,
    pub success: Option<bool>,
}

/// Shared ring buffer of recent git commands.
#[derive(Debug, Clone, Default)]
pub struct CommandLog(Arc<Mutex<VecDeque<LoggedCommand>>>);

impl CommandLog {
    fn push(&self, args: String) -> usize {
        let mut log = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if log.len() == LOG_CAPACITY {
            log.pop_front();
        }
        log.push_back(LoggedCommand {
            args,
            started: SystemTime::now(),
            duration: None,
            success: None,
        });
        log.len() - 1
    }

    fn finish(&self, args: &str, started: Instant, success: bool) {
        let mut log = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // Entries may have shifted while the command ran; find ours from the back.
        if let Some(entry) = log.iter_mut().rev().find(|e| e.args == args && e.success.is_none()) {
            entry.duration = Some(started.elapsed());
            entry.success = Some(success);
        }
    }

    /// Snapshot of the log, oldest first.
    pub fn entries(&self) -> Vec<LoggedCommand> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}

/// Captured output of a successful command.
#[derive(Debug, Clone)]
pub struct GitOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl GitOutput {
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// A git command runner bound to a working directory.
#[derive(Debug, Clone)]
pub struct GitCli {
    dir: PathBuf,
    program: OsString,
    env: Vec<(OsString, OsString)>,
    log: CommandLog,
}

impl GitCli {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            program: "git".into(),
            env: Vec::new(),
            log: CommandLog::default(),
        }
    }

    /// Adds environment variables to every command (tests use this to stay
    /// hermetic).
    pub fn with_env<K, V>(mut self, vars: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<OsString>,
        V: Into<OsString>,
    {
        self.env.extend(vars.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn log(&self) -> &CommandLog {
        &self.log
    }

    /// Same runner (sharing the log) in another directory.
    pub fn in_dir(&self, dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            ..self.clone()
        }
    }

    fn command<S: AsRef<OsStr>>(&self, args: &[S]) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(PINNED_CONFIG)
            .args(args)
            .current_dir(&self.dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_console_window(&mut cmd);
        cmd
    }

    /// Runs `git <args>` and returns its output, or an error carrying stderr.
    pub fn run<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<GitOutput> {
        self.run_with_stdin(args, None)
    }

    /// Runs `git <args>`, feeding `stdin` if given.
    pub fn run_with_stdin<S: AsRef<OsStr>>(&self, args: &[S], stdin: Option<&[u8]>) -> Result<GitOutput> {
        let shown = display_args(args);
        let started = Instant::now();
        self.log.push(shown.clone());
        tracing::debug!(args = %shown, dir = %self.dir.display(), "git");

        let mut cmd = self.command(args);
        if stdin.is_some() {
            cmd.stdin(Stdio::piped());
        }
        let result = (|| {
            let mut child = cmd.spawn().map_err(GitError::Spawn)?;
            if let Some(input) = stdin {
                let mut pipe = child.stdin.take().expect("stdin piped");
                pipe.write_all(input).map_err(GitError::Spawn)?;
            }
            child.wait_with_output().map_err(GitError::Spawn)
        })();

        let output = match result {
            Ok(o) => o,
            Err(e) => {
                self.log.finish(&shown, started, false);
                return Err(e);
            }
        };
        let success = output.status.success();
        self.log.finish(&shown, started, success);
        if success {
            Ok(GitOutput {
                stdout: output.stdout,
                stderr: output.stderr,
            })
        } else {
            Err(GitError::Failed {
                args: shown,
                code: output
                    .status
                    .code()
                    .map_or_else(|| "signal".to_string(), |c| c.to_string()),
                stderr: error_text(&output.stderr, &output.stdout),
            })
        }
    }

    /// Runs `git <args>` and calls `on_record` for each `sep`-terminated
    /// record of stdout as it arrives. Returning `false` stops early (the
    /// child is killed). Used for streaming large logs.
    pub fn stream<S: AsRef<OsStr>>(&self, args: &[S], sep: u8, mut on_record: impl FnMut(&[u8]) -> bool) -> Result<()> {
        let shown = display_args(args);
        let started = Instant::now();
        self.log.push(shown.clone());
        tracing::debug!(args = %shown, dir = %self.dir.display(), "git (stream)");

        let mut child = self.command(args).spawn().map_err(GitError::Spawn)?;
        let stdout = child.stdout.take().expect("stdout piped");
        let mut stderr = child.stderr.take().expect("stderr piped");
        // Drain stderr on a thread so a chatty child can't block on a full pipe.
        let stderr_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf);
            buf
        });

        let mut reader = BufReader::with_capacity(64 * 1024, stdout);
        let mut record = Vec::new();
        let mut stopped = false;
        loop {
            record.clear();
            let n = reader.read_until(sep, &mut record).map_err(GitError::Spawn)?;
            if n == 0 {
                break;
            }
            if record.last() == Some(&sep) {
                record.pop();
            }
            if !on_record(&record) {
                stopped = true;
                let _ = child.kill();
                break;
            }
        }
        drop(reader);
        let status = child.wait().map_err(GitError::Spawn)?;
        let stderr = stderr_thread.join().unwrap_or_default();
        let success = stopped || status.success();
        self.log.finish(&shown, started, success);
        if success {
            Ok(())
        } else {
            Err(GitError::Failed {
                args: shown,
                code: status.code().map_or_else(|| "signal".to_string(), |c| c.to_string()),
                stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
            })
        }
    }
}

/// The message to show for a failed command: stderr, or stdout when git
/// wrote its explanation there (e.g. `commit` with nothing staged).
fn error_text(stderr: &[u8], stdout: &[u8]) -> String {
    let err = String::from_utf8_lossy(stderr).trim().to_string();
    if err.is_empty() {
        String::from_utf8_lossy(stdout).trim().to_string()
    } else {
        err
    }
}

fn display_args<S: AsRef<OsStr>>(args: &[S]) -> String {
    args.iter()
        .map(|a| {
            let s = a.as_ref().to_string_lossy();
            if s.is_empty() || s.contains([' ', '\t', '"']) {
                format!("{s:?}")
            } else {
                s.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
fn hide_console_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console_window(_cmd: &mut Command) {}
