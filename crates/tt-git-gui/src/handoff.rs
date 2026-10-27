//! One running copy: `tt-git-gui <path>` while the app is open hands the
//! path to the copy that's running, which opens (or brings forward) that
//! repository's window, and exits.
//!
//! The first copy ([`start`]) holds an exclusive lock on
//! `<state_dir>/gui-handoff.lock` and listens on a local socket
//! ([`address`]: `<state_dir>/gui-handoff.sock`, or a named pipe on
//! Windows), both per state folder, so a copy with its own
//! `TTGIT_STATE_DIR` never hands off to another. A later copy can't take
//! the lock, so it connects instead: the running copy says its pid (on
//! Windows the new one lets it take the foreground), the new one sends its
//! [`Request`], one JSON line, and the running copy acks it at once and
//! opens it on GPUI's thread ([`serve`], [`open`]). Requests that arrive
//! before GPUI is up wait in the channel.
//!
//! If the hand-off fails (the copy holding the lock doesn't answer within
//! [`WAIT`]), the new copy starts on its own. `main` skips all of this for
//! `--new-instance` and for the options the running copy couldn't honor
//! (`--config`, `--no-plugins`, `--filter-to-cwd`): such a copy neither
//! hands off nor is handed to.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::component::Root;
use gpui::{App, Global, WindowHandle};
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, Stream};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tt_git_core::{GitCli, GitError, Repo};

use crate::windows;

/// The lock the running copy holds, in the state folder.
pub const LOCK: &str = "gui-handoff.lock";

/// The socket the running copy listens on, in the state folder (Unix).
pub const SOCKET: &str = "gui-handoff.sock";

/// How long a new copy keeps trying to hand off before it starts on its
/// own: long enough for a copy that's starting to begin listening.
pub const WAIT: Duration = Duration::from_secs(2);

/// How long a new copy waits between tries.
const RETRY: Duration = Duration::from_millis(100);

/// What a new copy asks the running one to open: its command line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// The folders named on the command line, made absolute.
    #[serde(default)]
    pub paths: Vec<PathBuf>,
    /// The folder it was started in, opened when none were named.
    #[serde(default)]
    pub cwd: PathBuf,
    /// `--open-repository`: show the folder picker.
    #[serde(default)]
    pub open_repository: bool,
    /// `--start-window`: show the start window.
    #[serde(default)]
    pub start_window: bool,
}

impl Request {
    /// The request for a command line started in `cwd`.
    pub fn new(paths: &[PathBuf], cwd: PathBuf, open_repository: bool, start_window: bool) -> Self {
        Self {
            paths: paths.iter().map(|p| cwd.join(p)).collect(),
            cwd,
            open_repository,
            start_window,
        }
    }
}

/// The running copy's first line to a new one.
#[derive(Debug, Serialize, Deserialize)]
struct Hello {
    pid: u32,
}

/// The running copy's answer to a request: it has it.
#[derive(Debug, Serialize, Deserialize)]
struct Ack {
    ok: bool,
}

/// Where the running copy for `state_dir` listens.
pub fn address(state_dir: &Path) -> String {
    let state_dir = std::path::absolute(state_dir).unwrap_or_else(|_| state_dir.to_path_buf());
    let hash = {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(state_dir.as_os_str().as_encoded_bytes());
        digest[..8].iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    if cfg!(windows) {
        format!(r"\\.\pipe\tt-git-gui-{hash}")
    } else {
        let path = state_dir.join(SOCKET);
        // A Unix socket's path has to fit in about 100 bytes.
        if path.as_os_str().len() < 100 {
            return path.to_string_lossy().into_owned();
        }
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        base.join(format!("tt-git-gui-{hash}.sock"))
            .to_string_lossy()
            .into_owned()
    }
}

/// The running copy's end: the lock and the socket. Dropping it stops
/// taking hand-offs.
pub struct Listener {
    address: String,
    requests: Option<UnboundedReceiver<Request>>,
    _lock: File,
}

impl Listener {
    /// The requests handed over, once; [`serve`] takes them.
    pub fn take_requests(&mut self) -> Option<UnboundedReceiver<Request>> {
        self.requests.take()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if !cfg!(windows) {
            let _ = std::fs::remove_file(&self.address);
        }
    }
}

/// How this copy starts.
pub enum Start {
    /// It's the running copy: it takes the hand-offs.
    Primary(Listener),
    /// It handed its request to the running copy, whose pid this is, and
    /// should exit.
    HandedOff(u32),
    /// It runs on its own: the hand-off failed.
    Alone,
}

/// Becomes the running copy for `state_dir`, or hands `request` to the one
/// there is. Keeps trying both for `wait`, so a copy that's starting or
/// quitting is waited for.
pub fn start(state_dir: &Path, request: &Request, wait: Duration) -> Start {
    let deadline = Instant::now() + wait;
    loop {
        match claim(state_dir) {
            Ok(Some(listener)) => return Start::Primary(listener),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("couldn't listen for other copies: {e}");
                return Start::Alone;
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        match send(state_dir, request, left.max(RETRY)) {
            Ok(pid) => return Start::HandedOff(pid),
            Err(e) if Instant::now() >= deadline => {
                tracing::warn!("couldn't hand off to the running copy: {e}");
                return Start::Alone;
            }
            Err(e) => tracing::debug!("handing off: {e}"),
        }
        std::thread::sleep(RETRY);
    }
}

/// Takes the lock and listens, or `None` if another copy holds the lock.
fn claim(state_dir: &Path) -> std::io::Result<Option<Listener>> {
    std::fs::create_dir_all(state_dir)?;
    // Not truncated: another copy may hold it.
    let lock = File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(state_dir.join(LOCK))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
        Err(std::fs::TryLockError::Error(e)) => return Err(e),
    }
    let address = address(state_dir);
    // With the lock held, a socket file left behind is a dead copy's.
    let name = address.as_str().to_fs_name::<GenericFilePath>()?;
    let listener = ListenerOptions::new().name(name).try_overwrite(true).create_sync()?;
    let (tx, rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("tt-git-gui-handoff".into())
        .spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { continue };
                let tx = tx.clone();
                let _ = std::thread::Builder::new()
                    .name("tt-git-gui-handoff-conn".into())
                    .spawn(move || {
                        if let Err(e) = answer(conn, &tx) {
                            tracing::warn!("a hand-off failed: {e}");
                        }
                    });
            }
        })?;
    tracing::info!("taking hand-offs at {address}");
    Ok(Some(Listener {
        address,
        requests: Some(rx),
        _lock: lock,
    }))
}

/// The running copy's side of one hand-off.
fn answer(conn: Stream, tx: &UnboundedSender<Request>) -> std::io::Result<()> {
    let mut conn = BufReader::new(conn);
    write_line(
        conn.get_mut(),
        &Hello {
            pid: std::process::id(),
        },
    )?;
    let request: Request = read_line(&mut conn)?;
    tracing::info!("handed {request:?}");
    let ok = tx.send(request).is_ok();
    write_line(conn.get_mut(), &Ack { ok })
}

/// The new copy's side: hands `request` to the copy listening for
/// `state_dir`, giving up after `wait`. Returns its pid.
pub fn send(state_dir: &Path, request: &Request, wait: Duration) -> Result<u32, String> {
    let address = address(state_dir);
    let request = request.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    // On a thread, so a copy that never answers can't hold this one up.
    std::thread::Builder::new()
        .name("tt-git-gui-handoff-send".into())
        .spawn(move || {
            let _ = tx.send(exchange(&address, &request));
        })
        .map_err(|e| e.to_string())?;
    match rx.recv_timeout(wait) {
        Ok(result) => result.map_err(|e| e.to_string()),
        Err(_) => Err("the running copy didn't answer".into()),
    }
}

fn exchange(address: &str, request: &Request) -> std::io::Result<u32> {
    let name = address.to_fs_name::<GenericFilePath>()?;
    let mut conn = BufReader::new(Stream::connect(name)?);
    let Hello { pid } = read_line(&mut conn)?;
    allow_foreground(pid);
    write_line(conn.get_mut(), request)?;
    let ack: Ack = read_line(&mut conn)?;
    if ack.ok {
        Ok(pid)
    } else {
        Err(std::io::Error::other("the running copy is quitting"))
    }
}

/// Lets the running copy bring its window forward (Windows only lets the
/// process the user is using do that).
fn allow_foreground(pid: u32) {
    #[cfg(windows)]
    if !tt_git_win::foreground::allow(pid) {
        tracing::debug!("couldn't let {pid} take the foreground");
    }
    #[cfg(not(windows))]
    let _ = pid;
}

fn write_line(w: &mut impl Write, value: &impl Serialize) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    line.push(b'\n');
    w.write_all(&line)?;
    w.flush()
}

fn read_line<T: for<'de> Deserialize<'de>>(r: &mut impl BufRead) -> std::io::Result<T> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Err(std::io::ErrorKind::UnexpectedEof.into());
    }
    serde_json::from_str(&line).map_err(std::io::Error::other)
}

/// Keeps the listener (its lock and socket) for the app's life.
struct Running(#[allow(dead_code)] Listener);

impl Global for Running {}

/// Opens each request handed to `listener` as it comes, for as long as the
/// app runs.
pub fn serve(mut listener: Listener, cx: &mut App) {
    let Some(mut requests) = listener.take_requests() else {
        return;
    };
    cx.set_global(Running(listener));
    cx.spawn(async move |cx| {
        while let Some(request) = requests.recv().await {
            cx.update(|cx| open(&request, cx));
        }
    })
    .detach();
}

/// Opens what another copy was started to open, as it would have:
/// the folders named, each in its window ([`windows::open_requested`]);
/// with none, the repository it was started in, or the start window outside
/// one; the start window, or the folder picker over it, when it asked.
pub fn open(request: &Request, cx: &mut App) {
    if request.open_repository {
        if let Some(start) = start_window(cx) {
            crate::start::pick(start, cx);
        }
    } else if request.start_window {
        start_window(cx);
    } else if !request.paths.is_empty() {
        windows::open_requested(&request.paths, cx);
        return;
    } else {
        match Repo::discover(&request.cwd, &GitCli::new(&request.cwd)) {
            // Started outside a repository: no error, as at startup.
            Err(GitError::NotARepo(_)) => {
                start_window(cx);
            }
            _ => {
                windows::open_requested(std::slice::from_ref(&request.cwd), cx);
                return;
            }
        }
    }
    cx.activate(true);
}

/// The start window brought forward, opened if there isn't one.
fn start_window(cx: &mut App) -> Option<WindowHandle<Root>> {
    if let Some(&start) = crate::start::windows(cx).first() {
        let _ = start.update(cx, |_, window, _| window.activate_window());
        return Some(start);
    }
    crate::start::open(None, cx)
        .inspect_err(|e| tracing::warn!("couldn't open the start window: {e}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::sync::Arc;

    use gpui::TestAppContext;
    use tt_git_app::{App as TtApp, Config};
    use tt_git_core::time::FixedClock;
    use tt_git_testkit::{NOW, Sandbox};

    use super::*;

    fn primary(state: &Path) -> Listener {
        match start(state, &Request::default(), WAIT) {
            Start::Primary(listener) => listener,
            _ => panic!("the first copy should be the running one"),
        }
    }

    #[test]
    fn the_second_copy_hands_its_request_to_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = primary(dir.path());
        let mut requests = first.take_requests().unwrap();

        let request = Request::new(&["a".into(), "/b".into()], "/src".into(), false, false);
        match start(dir.path(), &request, WAIT) {
            Start::HandedOff(pid) => assert_eq!(pid, std::process::id()),
            _ => panic!("should hand off"),
        }
        let got = requests.try_recv().expect("the request arrived");
        assert_eq!(got.paths, [PathBuf::from("/src/a"), PathBuf::from("/b")]);
        assert_eq!(got.cwd, PathBuf::from("/src"));
    }

    #[test]
    fn once_the_running_copy_quits_the_next_one_takes_over() {
        let dir = tempfile::tempdir().unwrap();
        drop(primary(dir.path()));
        // A dead copy's socket file doesn't get in the way either.
        let _ = std::fs::write(dir.path().join(SOCKET), "");
        let mut next = primary(dir.path());
        assert!(next.take_requests().is_some());
    }

    #[test]
    fn a_copy_that_holds_the_lock_but_never_answers_is_given_up_on() {
        let dir = tempfile::tempdir().unwrap();
        let lock = File::create(dir.path().join(LOCK)).unwrap();
        lock.try_lock().unwrap();
        let started = Instant::now();
        let outcome = start(dir.path(), &Request::default(), Duration::from_millis(300));
        assert!(matches!(outcome, Start::Alone));
        assert!(started.elapsed() >= Duration::from_millis(300), "tried for the wait");
    }

    #[test]
    fn each_state_folder_has_its_own_address() {
        let (a, b) = (Path::new("/state/a"), Path::new("/state/b"));
        assert_ne!(address(a), address(b));
        assert_eq!(address(a), address(a));
        if !cfg!(windows) {
            assert_eq!(address(a), format!("/state/a/{SOCKET}"));
            let long = PathBuf::from("/").join("x".repeat(120));
            assert!(address(&long).len() < 100, "{}", address(&long));
        }
    }

    fn setup(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        cx.update(|cx| {
            crate::init(cx);
            crate::set_launcher(
                Rc::new(|repo| {
                    let config = Config {
                        watch: false,
                        ..Config::default()
                    };
                    TtApp::new(config, repo, Arc::new(FixedClock(NOW))).map_err(|e| e.to_string())
                }),
                cx,
            );
        });
    }

    fn roots(cx: &mut TestAppContext) -> Vec<PathBuf> {
        cx.update(|cx| windows::repo_windows(cx).into_iter().map(|(_, r)| r).collect())
    }

    fn canonical(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap()
    }

    #[gpui::test]
    fn a_handed_path_opens_its_window_once(cx: &mut TestAppContext) {
        setup(cx);
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let request = Request::new(&[sb.path().join("sub")], "/".into(), false, false);
        std::fs::create_dir_all(sb.path().join("sub")).unwrap();
        cx.update(|cx| open(&request, cx));
        cx.update(|cx| open(&request, cx));
        assert_eq!(roots(cx), [canonical(sb.path())]);
        assert_eq!(cx.windows().len(), 1, "brought forward, not opened again");
    }

    #[gpui::test]
    fn no_path_opens_the_folder_it_was_started_in(cx: &mut TestAppContext) {
        setup(cx);
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        cx.update(|cx| crate::start::open(None, cx)).unwrap();
        cx.update(|cx| open(&Request::new(&[], sb.path().to_path_buf(), false, false), cx));
        assert_eq!(roots(cx), [canonical(sb.path())]);
        assert_eq!(cx.windows().len(), 1, "the start window closed");
    }

    #[gpui::test]
    fn started_outside_a_repo_it_shows_the_start_window_without_an_error(cx: &mut TestAppContext) {
        setup(cx);
        let elsewhere = tempfile::tempdir().unwrap();
        let request = Request::new(&[], elsewhere.path().to_path_buf(), false, false);
        cx.update(|cx| open(&request, cx));
        let starts = cx.update(crate::start::windows);
        assert_eq!(starts.len(), 1);
        let error = starts[0]
            .update(cx, |root, _, cx| {
                let view = root.view().clone().downcast::<crate::start::StartView>().unwrap();
                view.read(cx).error().map(str::to_string)
            })
            .unwrap();
        assert_eq!(error, None);

        // Again, or with --start-window: the same one comes forward.
        cx.update(|cx| open(&request, cx));
        cx.update(|cx| open(&Request::new(&[], "/".into(), false, true), cx));
        assert_eq!(cx.update(crate::start::windows), starts);
        assert_eq!(cx.windows().len(), 1);
    }

    #[gpui::test]
    fn open_repository_shows_the_picker(cx: &mut TestAppContext) {
        setup(cx);
        cx.update(|cx| open(&Request::new(&[], "/".into(), true, false), cx));
        cx.run_until_parked();
        assert!(cx.did_prompt_for_paths());
        assert_eq!(cx.update(crate::start::windows).len(), 1);
    }
}
