//! Fetching every remote on a timer while a repository is open
//! (`[fetch] interval`).
//!
//! [`App`](crate::App) asks every [`CHECK_EVERY`] whether a fetch is due and,
//! if no fetch, pull or push is running, starts one [`run`] off the UI
//! thread. `run` decides: a fetch is due once the newest fetch of the
//! repository (any worktree's `FETCH_HEAD`, so a fetch in a terminal counts
//! too, or the last background attempt) is older than the interval the
//! [`Policy`] gives. A lock in tt-git's state dir, keyed by the repository's
//! common git dir, makes several windows and worktrees of one repository
//! share one fetch instead of each fetching.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tt_git_core::Repo;

/// How often the app asks whether a fetch is due.
pub const CHECK_EVERY: Duration = Duration::from_secs(30);

/// How long after the repository opens the first check waits, so opening
/// it isn't slowed by a fetch.
pub const FIRST_CHECK: Duration = Duration::from_secs(10);

/// A lock older than this was left by a process that died mid-fetch.
const STALE_LOCK: Duration = Duration::from_secs(10 * 60);

/// Stretches or pauses the configured interval, given the conditions now.
/// [`run`] asks it on the job's thread, so it may take a moment to answer.
pub trait Policy: Send + Sync {
    /// The interval to fetch at, given the configured one; `None` skips
    /// this check.
    fn interval(&self, base: Duration) -> Option<Duration>;
}

/// The interval as configured, whatever the conditions.
pub struct AsConfigured;

impl Policy for AsConfigured {
    fn interval(&self, base: Duration) -> Option<Duration> {
        Some(base)
    }
}

/// How much slower background fetching goes on battery.
pub const ON_BATTERY: u32 = 3;

/// What [`Conserve`] goes by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Conditions {
    pub on_battery: bool,
    /// Battery saver, Low Power Mode or the power-saver profile.
    pub saving_power: bool,
    pub metered: bool,
}

/// The interval to fetch at given `conditions` (`[fetch] conserve`): none
/// while saving power or on a metered connection, [`ON_BATTERY`] times as
/// long on battery, else `base`.
pub fn conserve(base: Duration, conditions: Conditions) -> Option<Duration> {
    if conditions.metered || conditions.saving_power {
        None
    } else if conditions.on_battery {
        Some(base * ON_BATTERY)
    } else {
        Some(base)
    }
}

/// [`conserve`] with the conditions `probe` reads.
pub struct Conserve(pub fn() -> Conditions);

impl Conserve {
    /// Reading the OS's own power and network state.
    pub fn system() -> Self {
        Self(system_conditions)
    }
}

impl Policy for Conserve {
    fn interval(&self, base: Duration) -> Option<Duration> {
        let conditions = (self.0)();
        let interval = conserve(base, conditions);
        if interval != Some(base) {
            tracing::debug!(?conditions, ?interval, "background fetch: conserving");
        }
        interval
    }
}

/// The power and network state, as far as this OS says.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn system_conditions() -> Conditions {
    let state = tt_git_win::power::state();
    Conditions {
        on_battery: state.on_battery,
        saving_power: state.saving_power,
        metered: state.metered,
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn system_conditions() -> Conditions {
    Conditions::default()
}

/// What a check came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The repository has no remotes, was fetched recently enough, or
    /// another window or worktree is fetching it.
    NotDue,
    /// The policy paused fetching for now.
    Paused,
    /// Fetched; git's output or error text.
    Fetched(Result<String, String>),
}

/// The lock and the time of the last attempt for one repository, in
/// `<state_dir>/fetch/`.
#[derive(Debug, Clone)]
pub struct Store {
    lock: PathBuf,
    stamp: PathBuf,
}

impl Store {
    /// The store in `dir` for the repository whose common git dir is
    /// `common_dir`. Every worktree of a repository shares one.
    pub fn new(dir: &Path, common_dir: &Path) -> Self {
        let common = fs::canonicalize(common_dir).unwrap_or_else(|_| common_dir.to_path_buf());
        let key = format!("{:016x}", fnv1a(common.to_string_lossy().as_bytes()));
        Self {
            lock: dir.join(format!("{key}.lock")),
            stamp: dir.join(format!("{key}.stamp")),
        }
    }

    /// When a background fetch was last tried.
    fn last_attempt(&self) -> Option<SystemTime> {
        modified(&self.stamp)
    }

    fn record_attempt(&self, now: SystemTime) {
        let written = fs::File::create(&self.stamp).and_then(|f| f.set_modified(now));
        if let Err(e) = written {
            tracing::warn!(path = %self.stamp.display(), "background fetch: can't record the attempt: {e}");
        }
    }

    /// Takes the lock, or `None` while another process holds it. A lock
    /// left by a process that died is taken over.
    fn lock(&self, now: SystemTime) -> Option<Lock> {
        if let Some(parent) = self.lock.parent() {
            let _ = fs::create_dir_all(parent);
        }
        for _ in 0..2 {
            match fs::OpenOptions::new().write(true).create_new(true).open(&self.lock) {
                Ok(file) => {
                    let _ = file.set_modified(now);
                    return Some(Lock(self.lock.clone()));
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    let stale = modified(&self.lock).is_none_or(|at| age(now, at) >= STALE_LOCK);
                    if !stale {
                        return None;
                    }
                    let _ = fs::remove_file(&self.lock);
                }
                Err(e) => {
                    tracing::warn!(path = %self.lock.display(), "background fetch: can't take the lock: {e}");
                    return None;
                }
            }
        }
        None
    }
}

/// Held while fetching; removes the lock file when dropped.
struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Fetches `repo` if a fetch is due at `now` (see the module docs).
pub fn run(repo: &Repo, base: Duration, policy: &dyn Policy, store: &Store, now: SystemTime) -> Outcome {
    if repo.remotes().is_ok_and(|r| r.is_empty()) {
        return Outcome::NotDue;
    }
    let Some(interval) = policy.interval(base) else {
        return Outcome::Paused;
    };
    if !due(last_fetch(repo, store), interval, now) {
        return Outcome::NotDue;
    }
    let Some(_lock) = store.lock(now) else {
        return Outcome::NotDue;
    };
    // Another process may have fetched between the check and the lock.
    if !due(last_fetch(repo, store), interval, now) {
        return Outcome::NotDue;
    }
    store.record_attempt(now);
    Outcome::Fetched(repo.fetch_all_quietly().map_err(|e| e.to_string()))
}

/// Whether a fetch last made at `last` is `interval` old at `now`.
fn due(last: Option<SystemTime>, interval: Duration, now: SystemTime) -> bool {
    last.is_none_or(|at| age(now, at) >= interval)
}

/// The newest of every worktree's `FETCH_HEAD` and the last background
/// attempt.
fn last_fetch(repo: &Repo, store: &Store) -> Option<SystemTime> {
    let common = repo.common_dir();
    let worktrees = fs::read_dir(common.join("worktrees"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path().join("FETCH_HEAD"));
    std::iter::once(common.join("FETCH_HEAD"))
        .chain(worktrees)
        .filter_map(|path| modified(&path))
        .chain(store.last_attempt())
        .max()
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// How long before `now` `at` was; nothing if it's in the future.
fn age(now: SystemTime, at: SystemTime) -> Duration {
    now.duration_since(at).unwrap_or_default()
}

/// FNV-1a, for a file name that's the same in every build (std's hasher
/// isn't promised to be).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_git_core::GitCli;
    use tt_git_testkit::Sandbox;

    const MINUTE: Duration = Duration::from_secs(60);

    struct Paused;

    impl Policy for Paused {
        fn interval(&self, _: Duration) -> Option<Duration> {
            None
        }
    }

    struct Doubled;

    impl Policy for Doubled {
        fn interval(&self, base: Duration) -> Option<Duration> {
            Some(base * 2)
        }
    }

    /// A sandbox with a remote it has never fetched from, and a clone that
    /// has pushed a branch to it.
    fn with_remote() -> (Sandbox, Repo) {
        let sb = Sandbox::new();
        sb.commit("base");
        let remote = sb.add_remote("origin");
        std::fs::remove_file(sb.path().join(".git/FETCH_HEAD")).unwrap();
        let other = sb.clone_to(&remote, "other");
        sb.git_in(&other, &["push", "-q", "origin", "main:elsewhere"]);
        let repo = Repo::discover(sb.path(), &GitCli::new(sb.path()).with_env(sb.env())).unwrap();
        (sb, repo)
    }

    fn store(sb: &Sandbox, repo: &Repo) -> Store {
        Store::new(&sb.root().join("tt-state/fetch"), repo.common_dir())
    }

    fn fetched(sb: &Sandbox) -> bool {
        sb.git(&["branch", "-r"]).contains("origin/elsewhere")
    }

    #[test]
    fn fetches_a_repo_never_fetched() {
        let (sb, repo) = with_remote();
        let out = run(&repo, MINUTE, &AsConfigured, &store(&sb, &repo), SystemTime::now());
        assert!(matches!(out, Outcome::Fetched(Ok(_))), "{out:?}");
        assert!(fetched(&sb));
    }

    #[test]
    fn waits_for_the_interval_since_the_last_fetch() {
        let (sb, repo) = with_remote();
        let store = store(&sb, &repo);
        sb.git(&["fetch", "-q", "origin", "main"]);
        let now = SystemTime::now();
        assert_eq!(run(&repo, MINUTE, &AsConfigured, &store, now), Outcome::NotDue);
        assert!(!fetched(&sb));
        let later = now + MINUTE;
        assert!(matches!(
            run(&repo, MINUTE, &AsConfigured, &store, later),
            Outcome::Fetched(Ok(_))
        ));
        assert!(fetched(&sb));
    }

    #[test]
    fn a_fetch_in_another_worktree_counts() {
        let (sb, repo) = with_remote();
        let wt = sb.root().join("wt");
        sb.git(&["worktree", "add", "-q", "-b", "wt", wt.to_str().unwrap()]);
        sb.git_in(&wt, &["fetch", "-q", "origin", "main"]);
        let out = run(&repo, MINUTE, &AsConfigured, &store(&sb, &repo), SystemTime::now());
        assert_eq!(out, Outcome::NotDue);
    }

    #[test]
    fn a_failed_attempt_waits_too() {
        let (sb, repo) = with_remote();
        sb.git(&["remote", "set-url", "origin", "/nowhere/at/all"]);
        let store = store(&sb, &repo);
        let now = SystemTime::now();
        assert!(matches!(
            run(&repo, MINUTE, &AsConfigured, &store, now),
            Outcome::Fetched(Err(_))
        ));
        assert_eq!(
            run(&repo, MINUTE, &AsConfigured, &store, now + MINUTE / 2),
            Outcome::NotDue
        );
    }

    #[test]
    fn worktrees_of_one_repo_share_the_lock() {
        let (sb, repo) = with_remote();
        let wt = sb.root().join("wt");
        sb.git(&["worktree", "add", "-q", "-b", "wt", wt.to_str().unwrap()]);
        let in_wt = Repo::discover(&wt, &GitCli::new(&wt).with_env(sb.env())).unwrap();
        let now = SystemTime::now();
        let held = store(&sb, &repo).lock(now).expect("lock");
        assert!(store(&sb, &in_wt).lock(now).is_none());
        assert_eq!(
            run(&in_wt, MINUTE, &AsConfigured, &store(&sb, &in_wt), now),
            Outcome::NotDue
        );
        drop(held);
        assert!(store(&sb, &in_wt).lock(now).is_some());
    }

    #[test]
    fn a_stale_lock_is_taken_over() {
        let (sb, repo) = with_remote();
        let store = store(&sb, &repo);
        let now = SystemTime::now();
        std::mem::forget(store.lock(now - STALE_LOCK).expect("lock"));
        assert!(store.lock(now).is_some());
    }

    #[test]
    fn the_policy_stretches_or_pauses() {
        let (sb, repo) = with_remote();
        let store = store(&sb, &repo);
        assert_eq!(run(&repo, MINUTE, &Paused, &store, SystemTime::now()), Outcome::Paused);
        sb.git(&["fetch", "-q", "origin", "main"]);
        let now = SystemTime::now();
        assert_eq!(run(&repo, MINUTE, &Doubled, &store, now + MINUTE), Outcome::NotDue);
        assert!(matches!(
            run(&repo, MINUTE, &Doubled, &store, now + MINUTE * 2),
            Outcome::Fetched(Ok(_))
        ));
    }

    #[test]
    fn conserving_stretches_on_battery_and_pauses_when_saving_or_metered() {
        let at = |on_battery, saving_power, metered| {
            conserve(
                MINUTE,
                Conditions {
                    on_battery,
                    saving_power,
                    metered,
                },
            )
        };
        assert_eq!(at(false, false, false), Some(MINUTE));
        assert_eq!(at(true, false, false), Some(MINUTE * 3));
        assert_eq!(at(true, true, false), None);
        assert_eq!(at(false, true, false), None);
        assert_eq!(at(false, false, true), None);
        assert_eq!(at(true, false, true), None);
    }

    #[test]
    fn conserve_reads_the_conditions_at_each_check() {
        let (sb, repo) = with_remote();
        let store = store(&sb, &repo);
        let metered = Conserve(|| Conditions {
            metered: true,
            ..Conditions::default()
        });
        assert_eq!(run(&repo, MINUTE, &metered, &store, SystemTime::now()), Outcome::Paused);
        assert!(!fetched(&sb));
        let on_battery = Conserve(|| Conditions {
            on_battery: true,
            ..Conditions::default()
        });
        assert!(matches!(
            run(&repo, MINUTE, &on_battery, &store, SystemTime::now()),
            Outcome::Fetched(Ok(_))
        ));
        let now = SystemTime::now();
        assert_eq!(
            run(&repo, MINUTE, &on_battery, &store, now + MINUTE * 2),
            Outcome::NotDue
        );
        assert!(matches!(
            run(&repo, MINUTE, &on_battery, &store, now + MINUTE * 3),
            Outcome::Fetched(Ok(_))
        ));
    }

    #[test]
    fn no_remotes_no_fetch() {
        let sb = Sandbox::new();
        sb.commit("base");
        let repo = Repo::discover(sb.path(), &GitCli::new(sb.path()).with_env(sb.env())).unwrap();
        let out = run(&repo, MINUTE, &AsConfigured, &store(&sb, &repo), SystemTime::now());
        assert_eq!(out, Outcome::NotDue);
    }
}
