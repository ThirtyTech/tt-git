//! Running plugin commands.
//!
//! Every action, event hook and startup command is a fresh process (argv,
//! no shell) started in the plugin's root with `TTGIT_*` variables describing
//! the host and the invocation. Output is captured (capped) into a log record.
//! There is no sandbox: plugins run as the user, exactly like herdr.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const MAX_COMMANDS_IN_FLIGHT: usize = 32;
pub const OUTPUT_MAX_BYTES: usize = 64 * 1024;
pub const LOG_LIMIT: usize = 200;

/// What the user was looking at when a plugin command was invoked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationContext {
    pub repo_path: Option<String>,
    pub repo_name: Option<String>,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub selected_commit: Option<String>,
    pub selected_branch: Option<String>,
    pub selected_file: Option<String>,
    pub selected_text: Option<String>,
    pub focused_pane: Option<String>,
    /// `keybinding`, `palette`, `socket`, `event`, `startup`, `link_click`.
    pub invocation_source: Option<String>,
    pub correlation_id: Option<String>,
    pub clicked_url: Option<String>,
    pub link_handler_id: Option<String>,
}

/// An event as delivered to hooks (`TTGIT_PLUGIN_EVENT_JSON`) and subscribers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event: String,
    pub data: serde_json::Value,
}

/// Host-wide values every plugin process receives.
#[derive(Debug, Clone, Default)]
pub struct HostEnv {
    pub socket_path: Option<String>,
    pub bin_path: Option<PathBuf>,
    /// Extra variables for every command (tests use this for hermetic git).
    pub extra: Vec<(OsString, OsString)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Running,
    Succeeded,
    Failed,
}

/// One plugin command execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandLog {
    pub log_id: u64,
    pub plugin_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    pub command: Vec<String>,
    pub status: CommandStatus,
    pub started_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_unix_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunError {
    #[error("plugin_command_limit_reached: {MAX_COMMANDS_IN_FLIGHT} plugin commands are already running")]
    LimitReached,
    #[error("plugin_command_empty: no command to run")]
    Empty,
}

/// What kind of entry point is being run, for env vars and logs.
#[derive(Debug, Clone, PartialEq)]
pub enum Invocation {
    Action { action_id: String },
    Event { envelope: EventEnvelope },
    Startup,
    Pane { entrypoint: String },
}

/// Everything needed to start one plugin command.
#[derive(Debug, Clone)]
pub struct CommandRequest {
    pub plugin_id: String,
    pub plugin_root: PathBuf,
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub argv: Vec<String>,
    pub invocation: Invocation,
    pub context: InvocationContext,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Builds the environment for a plugin process. Host-managed variables are
/// applied last so callers can't override them.
pub fn plugin_env(host: &HostEnv, req: &CommandRequest) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = host.extra.clone();
    let mut set = |k: &str, v: OsString| env.push((k.into(), v));
    set("TTGIT_ENV", "1".into());
    if let Some(s) = &host.socket_path {
        set("TTGIT_SOCKET_PATH", s.into());
    }
    if let Some(b) = &host.bin_path {
        set("TTGIT_BIN_PATH", b.into());
    }
    set("TTGIT_PLUGIN_ID", req.plugin_id.clone().into());
    set("TTGIT_PLUGIN_ROOT", req.plugin_root.clone().into());
    set("TTGIT_PLUGIN_CONFIG_DIR", req.config_dir.clone().into());
    set("TTGIT_PLUGIN_STATE_DIR", req.state_dir.clone().into());
    set(
        "TTGIT_PLUGIN_CONTEXT_JSON",
        serde_json::to_string(&req.context).unwrap_or_default().into(),
    );
    if let Some(repo) = &req.context.repo_path {
        set("TTGIT_REPO_PATH", repo.into());
    }
    match &req.invocation {
        Invocation::Action { action_id } => set("TTGIT_PLUGIN_ACTION_ID", action_id.clone().into()),
        Invocation::Event { envelope } => {
            set("TTGIT_PLUGIN_EVENT", envelope.event.clone().into());
            set(
                "TTGIT_PLUGIN_EVENT_JSON",
                serde_json::to_string(envelope).unwrap_or_default().into(),
            );
        }
        Invocation::Startup => set("TTGIT_PLUGIN_EVENT", "startup".into()),
        Invocation::Pane { entrypoint } => set("TTGIT_PLUGIN_ENTRYPOINT_ID", entrypoint.clone().into()),
    }
    if let Some(url) = &req.context.clicked_url {
        set("TTGIT_PLUGIN_CLICKED_URL", url.into());
    }
    if let Some(id) = &req.context.link_handler_id {
        set("TTGIT_PLUGIN_LINK_HANDLER_ID", id.into());
    }
    env
}

/// Resolves `argv[0]`: a relative path containing a separator is relative to
/// the plugin root; on Windows a bare name is searched on PATH with PATHEXT
/// so `npm` finds `npm.cmd`.
pub fn resolve_program(program: &str, root: &Path) -> PathBuf {
    let has_sep = program.contains('/') || (cfg!(windows) && program.contains('\\'));
    let p = Path::new(program);
    if has_sep && p.is_relative() {
        return root.join(p);
    }
    #[cfg(windows)]
    if !has_sep
        && p.extension().is_none()
        && let Some(found) = search_path_windows(program)
    {
        return found;
    }
    PathBuf::from(program)
}

#[cfg(windows)]
fn search_path_windows(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    for dir in std::env::split_paths(&path) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// A `std::process::Command` for a plugin process, without stdio set up.
/// It gets no console window, so an event command (say on each background
/// fetch's `fetch.completed`) doesn't flash one up from the desktop app.
pub fn build_command(host: &HostEnv, req: &CommandRequest) -> Result<Command, RunError> {
    let (program, args) = req.argv.split_first().ok_or(RunError::Empty)?;
    let mut cmd = Command::new(resolve_program(program, &req.plugin_root));
    cmd.args(args).current_dir(&req.plugin_root).envs(plugin_env(host, req));
    tt_git_core::git::hide_console_window(&mut cmd);
    Ok(cmd)
}

/// Reads a stream fully, keeping at most `OUTPUT_MAX_BYTES`.
fn capture(mut r: impl Read) -> String {
    let mut kept = Vec::new();
    let mut total = 0usize;
    let mut buf = [0u8; 8192];
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total += n;
                let room = OUTPUT_MAX_BYTES.saturating_sub(kept.len());
                kept.extend_from_slice(&buf[..n.min(room)]);
            }
        }
    }
    let mut s = String::from_utf8_lossy(&kept).into_owned();
    if total > OUTPUT_MAX_BYTES {
        s.push_str(&format!(
            "\n[tt-git truncated plugin output after {OUTPUT_MAX_BYTES} bytes]"
        ));
    }
    s
}

/// Runs plugin commands in the background and keeps their logs.
/// Called with each command's final log.
type OnFinish = dyn Fn(&Arc<CommandLog>) + Send + Sync;

#[derive(Clone)]
pub struct Runner {
    host: Arc<Mutex<HostEnv>>,
    /// Shared with `on_finish`, so finished output isn't copied for it.
    logs: Arc<Mutex<VecDeque<Arc<CommandLog>>>>,
    in_flight: Arc<AtomicUsize>,
    next_id: Arc<AtomicU64>,
    on_finish: Arc<OnFinish>,
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runner")
            .field("in_flight", &self.in_flight.load(Ordering::SeqCst))
            .finish()
    }
}

impl Runner {
    /// `on_finish` is called (on the command's thread) when each command ends.
    pub fn new(host: HostEnv, on_finish: impl Fn(&Arc<CommandLog>) + Send + Sync + 'static) -> Self {
        Self {
            host: Arc::new(Mutex::new(host)),
            logs: Arc::new(Mutex::new(VecDeque::new())),
            in_flight: Arc::new(AtomicUsize::new(0)),
            next_id: Arc::new(AtomicU64::new(1)),
            on_finish: Arc::new(on_finish),
        }
    }

    pub fn host(&self) -> HostEnv {
        self.host.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_host(&self, host: HostEnv) {
        *self.host.lock().unwrap_or_else(|e| e.into_inner()) = host;
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::SeqCst)
    }

    /// Most recent logs first, optionally for one plugin.
    pub fn logs(&self, plugin_id: Option<&str>, limit: usize) -> Vec<CommandLog> {
        let logs = self.logs.lock().unwrap_or_else(|e| e.into_inner());
        logs.iter()
            .rev()
            .filter(|l| plugin_id.is_none_or(|p| l.plugin_id == p))
            .take(limit)
            .map(|l| CommandLog::clone(l))
            .collect()
    }

    fn store(&self, log: Arc<CommandLog>) {
        let mut logs = self.logs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = logs.iter_mut().find(|l| l.log_id == log.log_id) {
            *existing = log;
            return;
        }
        if logs.len() == LOG_LIMIT {
            logs.pop_front();
        }
        logs.push_back(log);
    }

    /// Starts a command and returns its log id immediately.
    pub fn spawn(&self, req: CommandRequest) -> Result<u64, RunError> {
        if req.argv.is_empty() {
            return Err(RunError::Empty);
        }
        if self.in_flight.fetch_add(1, Ordering::SeqCst) >= MAX_COMMANDS_IN_FLIGHT {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            return Err(RunError::LimitReached);
        }
        let log_id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (action_id, event) = match &req.invocation {
            Invocation::Action { action_id } => (Some(action_id.clone()), None),
            Invocation::Event { envelope } => (None, Some(envelope.event.clone())),
            Invocation::Startup => (None, Some("startup".into())),
            Invocation::Pane { entrypoint } => (Some(entrypoint.clone()), None),
        };
        let mut log = CommandLog {
            log_id,
            plugin_id: req.plugin_id.clone(),
            action_id,
            event,
            command: req.argv.clone(),
            status: CommandStatus::Running,
            started_unix_ms: now_ms(),
            finished_unix_ms: None,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            error: None,
        };
        self.store(Arc::new(log.clone()));

        let runner = self.clone();
        let host = self.host();
        let spawned = std::thread::Builder::new()
            .name("tt-git-plugin-command".into())
            .stack_size(crate::IO_THREAD_STACK)
            .spawn(move || {
                match run_to_completion(&host, &req) {
                    Ok((code, stdout, stderr)) => {
                        log.exit_code = code;
                        log.stdout = stdout;
                        log.stderr = stderr;
                        log.status = if code == Some(0) {
                            CommandStatus::Succeeded
                        } else {
                            CommandStatus::Failed
                        };
                    }
                    Err(e) => {
                        log.status = CommandStatus::Failed;
                        log.error = Some(e);
                    }
                }
                log.finished_unix_ms = Some(now_ms());
                let log = Arc::new(log);
                runner.store(log.clone());
                // Notify before decrementing so an idle check never sees
                // "nothing running" while the result is still unreported.
                (runner.on_finish)(&log);
                runner.in_flight.fetch_sub(1, Ordering::SeqCst);
            });
        if let Err(e) = spawned {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            tracing::error!("could not start plugin thread: {e}");
        }
        Ok(log_id)
    }
}

fn run_to_completion(host: &HostEnv, req: &CommandRequest) -> Result<(Option<i32>, String, String), String> {
    let mut cmd = build_command(host, req).map_err(|e| e.to_string())?;
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start {:?}: {e}", req.argv[0]))?;
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    // As `std::thread::spawn`, with a smaller stack.
    let err_thread = std::thread::Builder::new()
        .stack_size(crate::IO_THREAD_STACK)
        .spawn(move || capture(stderr))
        .expect("failed to spawn thread");
    let out = capture(stdout);
    let err = err_thread.join().unwrap_or_default();
    let status = child.wait().map_err(|e| e.to_string())?;
    Ok((status.code(), out, err))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(argv: &[&str]) -> CommandRequest {
        CommandRequest {
            plugin_id: "acme.t".into(),
            plugin_root: PathBuf::from("/plugins/acme"),
            config_dir: PathBuf::from("/cfg"),
            state_dir: PathBuf::from("/state"),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            invocation: Invocation::Action {
                action_id: "acme.t.go".into(),
            },
            context: InvocationContext {
                repo_path: Some("/repo".into()),
                selected_commit: Some("abc".into()),
                ..Default::default()
            },
        }
    }

    fn env_map(env: &[(OsString, OsString)]) -> std::collections::HashMap<String, String> {
        env.iter()
            .map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()))
            .collect()
    }

    #[test]
    fn env_has_host_and_invocation_vars() {
        let host = HostEnv {
            socket_path: Some("sock".into()),
            bin_path: Some(PathBuf::from("/bin/tt-git")),
            extra: vec![("TTGIT_PLUGIN_ID".into(), "spoofed".into())],
        };
        let env = plugin_env(&host, &req(&["x"]));
        let last_wins = env_map(&env);
        assert_eq!(last_wins["TTGIT_PLUGIN_ID"], "acme.t", "host vars override extras");
        assert_eq!(last_wins["TTGIT_SOCKET_PATH"], "sock");
        assert_eq!(last_wins["TTGIT_ENV"], "1");
        assert_eq!(last_wins["TTGIT_PLUGIN_ACTION_ID"], "acme.t.go");
        assert_eq!(last_wins["TTGIT_REPO_PATH"], "/repo");
        let ctx: InvocationContext = serde_json::from_str(&last_wins["TTGIT_PLUGIN_CONTEXT_JSON"]).unwrap();
        assert_eq!(ctx.selected_commit.as_deref(), Some("abc"));

        let mut r = req(&["x"]);
        r.invocation = Invocation::Event {
            envelope: EventEnvelope {
                event: "push.completed".into(),
                data: serde_json::json!({"branch": "main"}),
            },
        };
        let m = env_map(&plugin_env(&HostEnv::default(), &r));
        assert_eq!(m["TTGIT_PLUGIN_EVENT"], "push.completed");
        let ev: EventEnvelope = serde_json::from_str(&m["TTGIT_PLUGIN_EVENT_JSON"]).unwrap();
        assert_eq!(ev.data["branch"], "main");
        assert!(!m.contains_key("TTGIT_SOCKET_PATH"));
    }

    #[test]
    fn resolves_relative_programs_against_root() {
        let root = Path::new("/plugins/acme");
        assert_eq!(resolve_program("./bin/run", root), root.join("./bin/run"));
        assert_eq!(resolve_program("dist/x.js", root), root.join("dist/x.js"));
        if !cfg!(windows) {
            assert_eq!(resolve_program("node", root), PathBuf::from("node"));
        }
    }

    #[test]
    fn capture_truncates() {
        let big = vec![b'x'; OUTPUT_MAX_BYTES + 10];
        let s = capture(&big[..]);
        assert!(s.ends_with(&format!(
            "[tt-git truncated plugin output after {OUTPUT_MAX_BYTES} bytes]"
        )));
        assert_eq!(capture(&b"hi"[..]), "hi");
    }

    #[test]
    fn empty_command_is_rejected() {
        let r = Runner::new(HostEnv::default(), |_| {});
        assert_eq!(r.spawn(req(&[])), Err(RunError::Empty));
    }
}
