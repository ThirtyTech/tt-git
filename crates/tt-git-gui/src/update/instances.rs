//! The running copies of the app, so an update can restart every one of
//! them on the repositories it had open.
//!
//! Each instance registers in `<state_dir>/gui-instances/` ([`Instance`]):
//! `<pid>.lock`, which it holds an exclusive lock on while it runs, and
//! `<pid>.json` ([`Record`]), saying how to start it again. The lock is
//! released when the process ends however it ends, so a crash never leaves
//! an instance that looks alive.
//!
//! To restart them all, the updating instance writes [`RESTART`] and starts
//! the waiter ([`finish`], `tt-git-gui --finish-update`). Every instance
//! polls for that file ([`Instance::restart_requested`]); when it appears,
//! the instance writes its open repositories into its record and quits.
//! The waiter waits until each of them has let go of its lock, runs the
//! installer if the update has one, and starts each instance again.

use std::fs::File;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The folder in the state folder.
pub const DIR: &str = "gui-instances";

/// The file that asks every instance to restart.
pub const RESTART: &str = "restart.json";

/// How long the waiter waits for every instance to quit. One that's still
/// open after that (its user said no to closing a window) keeps running
/// and isn't started again.
pub const WAIT: Duration = Duration::from_secs(5 * 60);

/// How to start an instance again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// What to run: the executable, or the macOS bundle or AppImage.
    pub launch: PathBuf,
    /// When it started, in milliseconds since the epoch.
    pub started: u64,
    /// The repositories it had open when it was asked to restart.
    #[serde(default)]
    pub repos: Vec<PathBuf>,
    /// It quit to restart: start it again.
    #[serde(default)]
    pub restart: bool,
}

/// What [`RESTART`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// When it was made, in milliseconds since the epoch. Instances started
    /// after it are already up to date.
    pub at: u64,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// This process's place in the registry. Holding it holds the lock.
pub struct Instance {
    dir: PathBuf,
    pid: u32,
    record: Record,
    _lock: File,
}

impl Instance {
    /// Registers this process in `dir`, to be started again with `launch`.
    /// Clears out what instances that are gone left behind.
    pub fn register(dir: &Path, launch: PathBuf) -> std::io::Result<Self> {
        Self::register_as(dir, std::process::id(), launch)
    }

    fn register_as(dir: &Path, pid: u32, launch: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        sweep(dir);
        let lock = File::create(dir.join(format!("{pid}.lock")))?;
        lock.try_lock().map_err(std::io::Error::other)?;
        let instance = Self {
            dir: dir.to_path_buf(),
            pid,
            record: Record {
                launch,
                started: now_ms(),
                repos: Vec::new(),
                restart: false,
            },
            _lock: lock,
        };
        instance.write()?;
        Ok(instance)
    }

    fn json(&self) -> PathBuf {
        self.dir.join(format!("{}.json", self.pid))
    }

    fn write(&self) -> std::io::Result<()> {
        write_json(&self.json(), &self.record)
    }

    /// Whether an update asked every instance started before it to
    /// restart, and this one hasn't yet.
    pub fn restart_requested(&self) -> bool {
        !self.record.restart
            && read_json::<Request>(&self.dir.join(RESTART)).is_some_and(|r| r.at >= self.record.started)
    }

    /// Notes that this instance is quitting to restart, with `repos` open.
    pub fn restarting(&mut self, repos: Vec<PathBuf>) -> std::io::Result<()> {
        self.record.repos = repos;
        self.record.restart = true;
        self.write()
    }

    /// Asks every instance to restart, this one included.
    pub fn request_restart(&self) -> std::io::Result<()> {
        write_json(&self.dir.join(RESTART), &Request { at: now_ms() })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Leaves the registry, unless it's quitting to restart.
    pub fn leave(&self) {
        if !self.record.restart {
            let _ = std::fs::remove_file(self.json());
        }
    }
}

/// Removes the files of instances that are gone and weren't asked to
/// restart.
fn sweep(dir: &Path) {
    for (pid, lock) in locks(dir) {
        let json = dir.join(format!("{pid}.json"));
        let restarting = read_json::<Record>(&json).is_some_and(|r| r.restart);
        if !restarting && is_free(&lock) {
            let _ = std::fs::remove_file(&json);
            let _ = std::fs::remove_file(&lock);
        }
    }
}

/// The lock files in `dir`, by pid.
fn locks(dir: &Path) -> Vec<(u32, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let pid = path
                .extension()
                .is_some_and(|x| x == "lock")
                .then(|| path.file_stem()?.to_str()?.parse::<u32>().ok())??;
            Some((pid, path))
        })
        .collect();
    found.sort();
    found
}

/// Whether no process holds the lock on `lock` (or it's gone).
fn is_free(lock: &Path) -> bool {
    match File::options().write(true).open(lock) {
        Ok(f) => f.try_lock().is_ok(),
        Err(e) => e.kind() == ErrorKind::NotFound,
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Writes through a temporary file, so a reader never sees half of it.
fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// What the waiter does to the world: tests record it instead.
pub trait Launch {
    /// Runs the downloaded installer and waits for it.
    fn install(&mut self, setup: &Path) -> Result<(), String>;
    /// Starts an instance again from its record.
    fn start(&mut self, record: &Record) -> Result<(), String>;
}

/// What [`finish`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Finished {
    /// Instances started again.
    pub started: usize,
    /// Instances still running when the wait ran out.
    pub still_running: usize,
}

/// The waiter: once every instance that was running when the restart was
/// asked for has quit (or `wait` runs out), runs `installer` if there is
/// one, then starts each instance that quit to restart. Cleans the
/// registry up after itself.
pub fn finish(
    dir: &Path,
    installer: Option<&Path>,
    launch: &mut dyn Launch,
    wait: Duration,
) -> Result<Finished, String> {
    let asked = read_json::<Request>(&dir.join(RESTART)).map_or_else(now_ms, |r| r.at);
    // Instances started after the request run the new version already.
    let waiting: Vec<(u32, PathBuf)> = locks(dir)
        .into_iter()
        .filter(|(pid, _)| read_json::<Record>(&dir.join(format!("{pid}.json"))).is_none_or(|r| r.started <= asked))
        .collect();
    let deadline = Instant::now() + wait;
    let mut running: Vec<_> = waiting.clone();
    while !running.is_empty() {
        running.retain(|(_, lock)| !is_free(lock));
        if running.is_empty() || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = std::fs::remove_file(dir.join(RESTART));
    if !running.is_empty() {
        tracing::warn!("{} instances didn't quit; leaving them running", running.len());
    }

    let installed = match installer {
        // Not while any instance still has the files open.
        Some(_) if !running.is_empty() => Err("tt-git is still running, so the installer wasn't run".to_string()),
        Some(setup) => launch.install(setup),
        None => Ok(()),
    };
    if let Err(e) = &installed {
        tracing::error!("{e}");
    }

    let mut done = Finished {
        started: 0,
        still_running: running.len(),
    };
    for (pid, lock) in waiting {
        if running.iter().any(|(p, _)| *p == pid) {
            continue;
        }
        let json = dir.join(format!("{pid}.json"));
        if let Some(record) = read_json::<Record>(&json).filter(|r| r.restart) {
            match launch.start(&record) {
                Ok(()) => done.started += 1,
                Err(e) => tracing::error!("couldn't start {}: {e}", record.launch.display()),
            }
        }
        let _ = std::fs::remove_file(&json);
        let _ = std::fs::remove_file(&lock);
    }
    installed.map(|()| done)
}

/// Starts the processes for real.
pub struct System;

/// The hidden argument that starts on the start window, for an instance
/// that had no repository open.
pub const START_WINDOW: &str = "--start-window";

impl System {
    /// The command that starts `record` again.
    pub fn command(record: &Record) -> std::process::Command {
        let args: Vec<std::ffi::OsString> = if record.repos.is_empty() {
            vec![START_WINDOW.into()]
        } else {
            record.repos.iter().map(|r| r.clone().into_os_string()).collect()
        };
        let mut cmd = if record.launch.extension().is_some_and(|e| e == "app") {
            let mut open = std::process::Command::new("open");
            open.arg("-n").arg(&record.launch).arg("--args");
            open
        } else {
            std::process::Command::new(&record.launch)
        };
        cmd.args(args);
        if let Some(first) = record.repos.first() {
            cmd.current_dir(first);
        }
        cmd
    }
}

impl Launch for System {
    fn install(&mut self, setup: &Path) -> Result<(), String> {
        tracing::info!("running {}", setup.display());
        // Silent, no reboot; it keeps the install's folder and per-user or
        // all-users choice.
        let status = std::process::Command::new(setup)
            .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"])
            .status()
            .map_err(|e| format!("couldn't run {}: {e}", setup.display()))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("the installer failed ({status})"))
        }
    }

    fn start(&mut self, record: &Record) -> Result<(), String> {
        tracing::info!("starting {} on {:?}", record.launch.display(), record.repos);
        let mut cmd = Self::command(record);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        cmd.spawn().map(drop).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorded {
        installed: Vec<PathBuf>,
        started: Vec<Record>,
    }

    impl Launch for Recorded {
        fn install(&mut self, setup: &Path) -> Result<(), String> {
            self.installed.push(setup.to_path_buf());
            Ok(())
        }
        fn start(&mut self, record: &Record) -> Result<(), String> {
            self.started.push(record.clone());
            Ok(())
        }
    }

    #[test]
    fn a_restart_reopens_every_instance_on_its_repos() {
        let dir = tempfile::tempdir().unwrap();
        let reg = dir.path().join(DIR);
        let mut one = Instance::register_as(&reg, 1, "gui".into()).unwrap();
        let mut two = Instance::register_as(&reg, 2, "gui".into()).unwrap();
        assert!(!one.restart_requested());

        std::thread::sleep(Duration::from_millis(2));
        one.request_restart().unwrap();
        assert!(one.restart_requested() && two.restart_requested());
        one.restarting(vec!["a".into(), "b".into()]).unwrap();
        two.restarting(vec![]).unwrap();
        assert!(!one.restart_requested(), "asked once");

        // Both quit.
        drop((one, two));
        let mut launch = Recorded::default();
        let done = finish(&reg, None, &mut launch, Duration::from_secs(5)).unwrap();
        assert_eq!(
            done,
            Finished {
                started: 2,
                still_running: 0
            }
        );
        let repos: Vec<_> = launch.started.iter().map(|r| r.repos.clone()).collect();
        assert_eq!(repos, [vec![PathBuf::from("a"), PathBuf::from("b")], vec![]]);
        assert!(launch.installed.is_empty());
        let left: Vec<_> = std::fs::read_dir(&reg).unwrap().flatten().collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn the_installer_waits_for_every_instance_to_quit() {
        let dir = tempfile::tempdir().unwrap();
        let reg = dir.path().join(DIR);
        let mut one = Instance::register_as(&reg, 1, "gui".into()).unwrap();
        one.request_restart().unwrap();
        one.restarting(vec!["a".into()]).unwrap();
        let quits = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(one);
        });
        let mut launch = Recorded::default();
        let started = Instant::now();
        let done = finish(&reg, Some(Path::new("setup.exe")), &mut launch, Duration::from_secs(10)).unwrap();
        quits.join().unwrap();
        assert!(started.elapsed() >= Duration::from_millis(250), "waited for it");
        assert_eq!(done.started, 1);
        assert_eq!(launch.installed, [PathBuf::from("setup.exe")]);
    }

    #[test]
    fn one_that_wont_quit_is_left_running_and_not_started_again() {
        let dir = tempfile::tempdir().unwrap();
        let reg = dir.path().join(DIR);
        let mut stays = Instance::register_as(&reg, 1, "gui".into()).unwrap();
        stays.request_restart().unwrap();
        stays.restarting(vec!["a".into()]).unwrap();
        let mut launch = Recorded::default();
        let err = finish(
            &reg,
            Some(Path::new("setup.exe")),
            &mut launch,
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(err.contains("still running"), "{err}");
        assert!(launch.installed.is_empty() && launch.started.is_empty());
        drop(stays);
    }

    #[test]
    fn instances_that_are_gone_are_swept_and_newer_ones_ignore_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let reg = dir.path().join(DIR);
        drop(Instance::register_as(&reg, 7, "gui".into()).unwrap());
        let old = Instance::register_as(&reg, 8, "gui".into()).unwrap();
        assert!(!reg.join("7.json").exists() && !reg.join("7.lock").exists(), "swept");
        old.request_restart().unwrap();
        std::thread::sleep(Duration::from_millis(2));
        let new = Instance::register_as(&reg, 9, "gui".into()).unwrap();
        assert!(old.restart_requested());
        assert!(!new.restart_requested(), "started after the request");
        new.leave();
        assert!(!reg.join("9.json").exists());
    }

    #[test]
    fn instances_restart_with_their_repos_or_on_the_start_window() {
        let record = Record {
            launch: "/opt/tt-git/bin/tt-git-gui".into(),
            repos: vec!["/src/a".into(), "/src/b".into()],
            ..Record::default()
        };
        let cmd = System::command(&record);
        assert_eq!(cmd.get_program(), "/opt/tt-git/bin/tt-git-gui");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["/src/a", "/src/b"]);
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/src/a")));

        let bundle = Record {
            launch: "/Applications/tt-git.app".into(),
            ..Record::default()
        };
        let cmd = System::command(&bundle);
        assert_eq!(cmd.get_program(), "open");
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            ["-n", "/Applications/tt-git.app", "--args", START_WINDOW]
        );
    }
}
