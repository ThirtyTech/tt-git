//! Hermetic throwaway git repositories.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, Ordering};

use tempfile::TempDir;

/// 2024-01-01T00:00:00Z. Each commit made through the sandbox advances the
/// fake clock by one minute so hashes and dates are reproducible.
pub const BASE_TIME: i64 = 1_704_067_200;

/// Env vars that would leak the developer's repo or config into a test.
const SCRUBBED_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_CEILING_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "TTGIT_SOCKET_PATH",
];

const GITCONFIG: &str = "\
[user]
\tname = Test User
\temail = test@example.com
[init]
\tdefaultBranch = main
[core]
\tautocrlf = false
\tfsmonitor = false
[commit]
\tgpgsign = false
[tag]
\tgpgsign = false
[advice]
\tdetachedHead = false
[protocol \"file\"]
\tallow = always
";

/// A temp directory holding a fake `$HOME`, a repo at `<tmp>/repo`, and
/// optional bare remotes under `<tmp>/remotes`.
pub struct Sandbox {
    dir: TempDir,
    repo: PathBuf,
    clock: AtomicI64,
}

impl Default for Sandbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Sandbox {
    /// A sandbox with an initialized, empty repo on branch `main`.
    pub fn new() -> Self {
        Self::named("repo")
    }

    /// Like [`new`](Self::new), with the repo at `<tmp>/<name>`, so the app
    /// shows `name` as the repository's name.
    pub fn named(name: &str) -> Self {
        let sb = Self::bare_dirs(name);
        sb.git(&["init", "-q", "-b", "main"]);
        sb
    }

    /// A sandbox whose repo directory exists but is not a git repo.
    pub fn without_repo() -> Self {
        Self::bare_dirs("repo")
    }

    fn bare_dirs(name: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("tt-git-test-")
            .tempdir()
            .expect("create temp dir");
        let root = dir.path();
        for sub in ["home", "home/.config", name, "remotes", "tt-config", "tt-state"] {
            std::fs::create_dir_all(root.join(sub)).expect("create sandbox dir");
        }
        std::fs::write(root.join("home/.gitconfig"), GITCONFIG).expect("write gitconfig");
        // Canonicalize so paths compare equal to what git reports
        // (macOS /var -> /private/var, Windows short names).
        let repo = dunce_canonicalize(&root.join(name));
        Self {
            dir,
            repo,
            clock: AtomicI64::new(BASE_TIME),
        }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// The working tree of the main repo.
    pub fn path(&self) -> &Path {
        &self.repo
    }

    pub fn home(&self) -> PathBuf {
        self.root().join("home")
    }

    /// Environment that makes git (and tt-git) ignore the developer's own
    /// configuration. Apply it to every child process the test spawns.
    ///
    /// It does not pin dates: commits and reflog entries the app under test
    /// makes get the real time, as they would for a user. Only the sandbox's
    /// own setup commands use the fake clock (see [`git`](Self::git)).
    pub fn env(&self) -> Vec<(OsString, OsString)> {
        let home = self.home();
        vec![
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_CONFIG_GLOBAL".into(), home.join(".gitconfig").into()),
            ("HOME".into(), home.clone().into()),
            ("USERPROFILE".into(), home.clone().into()),
            ("XDG_CONFIG_HOME".into(), home.join(".config").into()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            ("LC_ALL".into(), "C".into()),
            ("TTGIT_CONFIG_DIR".into(), self.root().join("tt-config").into()),
            ("TTGIT_STATE_DIR".into(), self.root().join("tt-state").into()),
        ]
    }

    /// Env var names a child process must not inherit.
    pub fn scrubbed_env() -> &'static [&'static str] {
        SCRUBBED_ENV
    }

    /// Applies the hermetic environment to `cmd`.
    pub fn apply_env(&self, cmd: &mut Command) {
        for var in SCRUBBED_ENV {
            cmd.env_remove(var);
        }
        cmd.envs(self.env());
    }

    /// Runs git in the main repo and returns trimmed stdout. Panics with
    /// stderr on failure.
    pub fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.repo, args)
    }

    /// Runs git in `dir` with the hermetic environment.
    pub fn git_in(&self, dir: &Path, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(dir);
        self.apply_env(&mut cmd);
        let now = format!("@{} +0000", self.clock.load(Ordering::SeqCst));
        cmd.env("GIT_AUTHOR_DATE", &now).env("GIT_COMMITTER_DATE", &now);
        let out = cmd.output().unwrap_or_else(|e| panic!("spawning git {args:?}: {e}"));
        assert!(
            out.status.success(),
            "git {args:?} failed in {}:\n{}{}",
            dir.display(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim_end().to_string()
    }

    /// Writes a file relative to the repo root, creating parent dirs.
    pub fn write(&self, rel: &str, contents: &str) -> &Self {
        let path = self.repo.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&path, contents).expect("write file");
        self
    }

    /// Advances the fake clock by one minute.
    pub fn tick(&self) -> &Self {
        self.clock.fetch_add(60, Ordering::SeqCst);
        self
    }

    /// Writes `file`, stages it and commits with `msg`. Returns the full hash.
    pub fn commit_file(&self, file: &str, contents: &str, msg: &str) -> String {
        self.write(file, contents);
        self.git(&["add", "--", file]);
        self.commit_staged(msg)
    }

    /// Commits a change to a file named after the message. Returns the hash.
    pub fn commit(&self, msg: &str) -> String {
        let file = format!("{}.txt", slug(msg));
        let contents = format!("{msg}\n{}\n", self.clock.load(Ordering::SeqCst));
        self.commit_file(&file, &contents, msg)
    }

    /// Commits whatever is staged (allowing empty commits).
    pub fn commit_staged(&self, msg: &str) -> String {
        self.tick();
        self.git(&["commit", "-q", "--allow-empty", "-m", msg]);
        self.rev_parse("HEAD")
    }

    pub fn branch(&self, name: &str) -> &Self {
        self.git(&["branch", name]);
        self
    }

    pub fn checkout(&self, name: &str) -> &Self {
        self.git(&["switch", "-q", name]);
        self
    }

    pub fn checkout_new(&self, name: &str) -> &Self {
        self.git(&["switch", "-q", "-c", name]);
        self
    }

    /// Merges `name` into the current branch with a merge commit.
    pub fn merge(&self, name: &str) -> String {
        self.tick();
        self.git(&["merge", "-q", "--no-ff", "--no-edit", name]);
        self.rev_parse("HEAD")
    }

    pub fn rev_parse(&self, rev: &str) -> String {
        self.git(&["rev-parse", rev])
    }

    pub fn current_branch(&self) -> String {
        self.git(&["branch", "--show-current"])
    }

    /// Creates a bare repo at `<tmp>/remotes/<name>.git`, adds it as remote
    /// `name`, and pushes all branches to it.
    pub fn add_remote(&self, name: &str) -> PathBuf {
        let bare = self.root().join("remotes").join(format!("{name}.git"));
        self.git_in(
            self.root(),
            &["init", "-q", "--bare", "-b", "main", &bare.to_string_lossy()],
        );
        self.git(&["remote", "add", name, &bare.to_string_lossy()]);
        if !self.git(&["branch", "--list"]).is_empty() {
            self.git(&["push", "-q", "--all", name]);
        }
        self.git(&["fetch", "-q", name]);
        bare
    }

    /// Clones `remote_url` into `<tmp>/<dir>` (another "user" of the remote)
    /// and returns its path, for making commits other people pushed.
    pub fn clone_to(&self, remote_url: &Path, dir: &str) -> PathBuf {
        let dest = self.root().join(dir);
        self.git_in(
            self.root(),
            &["clone", "-q", &remote_url.to_string_lossy(), &dest.to_string_lossy()],
        );
        dest
    }
}

fn slug(msg: &str) -> String {
    let s: String = msg
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "commit".into() } else { s }
}

/// `canonicalize` without the `\\?\` prefix Windows adds, which git rejects
/// in some places.
fn dunce_canonicalize(path: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let s = canon.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => canon,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_are_reproducible() {
        let a = Sandbox::new();
        let b = Sandbox::new();
        assert_eq!(a.commit("first"), b.commit("first"));
        assert_eq!(
            a.git(&["log", "-1", "--format=%an <%ae> %ct"]),
            format!("Test User <test@example.com> {}", BASE_TIME + 60)
        );
    }

    #[test]
    fn branches_merges_and_remotes() {
        let sb = Sandbox::new();
        sb.commit("init");
        sb.checkout_new("feature");
        sb.commit("feature work");
        sb.checkout("main");
        sb.merge("feature");
        assert_eq!(sb.git(&["rev-list", "--count", "HEAD"]), "3");
        sb.add_remote("origin");
        assert!(sb.git(&["branch", "-r"]).contains("origin/feature"));
    }
}
