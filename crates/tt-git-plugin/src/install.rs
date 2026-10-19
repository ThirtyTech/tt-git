//! Installing plugins from GitHub (`owner/repo[/subdir…]`), updating them,
//! and garbage-collecting old installation generations.
//!
//! Each install or update fetches into a fresh directory
//! `<config>/plugins/github-installations/<id>/<pid>-<generation>/checkout`
//! so running plugin processes keep their files while new invocations use
//! the new ones. Old generations are deleted once no running tt-git holds a
//! lease on them.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::manifest::{self, LoadedManifest, platform_matches};
use crate::registry::{self, InstalledPlugin, PluginDirs, Registry, RegistryError, Source, escape_id};

/// `owner/repo[/subdir…]`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubSpec {
    pub owner: String,
    pub repo: String,
    pub subdir: Option<String>,
}

impl GithubSpec {
    pub fn parse(s: &str) -> Result<Self, InstallError> {
        let s = s.trim().trim_end_matches('/');
        let mut parts = s.split('/');
        let (Some(owner), Some(repo)) = (parts.next(), parts.next()) else {
            return Err(InstallError::BadSpec(s.to_string()));
        };
        let valid = |p: &str| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        };
        let rest: Vec<&str> = parts.collect();
        if !valid(owner) || !valid(repo) || !rest.iter().all(|p| valid(p)) {
            return Err(InstallError::BadSpec(s.to_string()));
        }
        Ok(Self {
            owner: owner.to_string(),
            repo: repo.trim_end_matches(".git").to_string(),
            subdir: (!rest.is_empty()).then(|| rest.join("/")),
        })
    }

    /// Clone URL. `TTGIT_PLUGIN_GITHUB_BASE` replaces `https://github.com`
    /// (tests point it at a directory of bare repos).
    pub fn url(&self) -> String {
        let base = std::env::var("TTGIT_PLUGIN_GITHUB_BASE").unwrap_or_else(|_| "https://github.com".into());
        format!("{}/{}/{}.git", base.trim_end_matches('/'), self.owner, self.repo)
    }

    pub fn display(&self) -> String {
        match &self.subdir {
            Some(sub) => format!("{}/{}/{sub}", self.owner, self.repo),
            None => format!("{}/{}", self.owner, self.repo),
        }
    }

    fn matches(&self, source: &Source) -> bool {
        matches!(source, Source::Github { owner, repo, subdir, .. }
            if *owner == self.owner && *repo == self.repo && *subdir == self.subdir)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("invalid plugin source {0:?}: expected owner/repo[/subdir]")]
    BadSpec(String),
    #[error("git {args} failed: {stderr}")]
    Git { args: String, stderr: String },
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Manifest(#[from] manifest::ManifestError),
    #[error("plugin_linked_locally: {0} is linked from a local path; unlink it before installing")]
    LinkedLocally(String),
    #[error("platform_unsupported: {id} does not support {platform}")]
    Platform { id: String, platform: &'static str },
    #[error("plugin_build_failed: {command:?} exited with {status}")]
    BuildFailed { command: Vec<String>, status: String },
    #[error("plugin_manifest_changed_during_build: {0} changed while building; refusing to install")]
    ManifestChanged(String),
    #[error("plugin_not_found: no GitHub-installed plugin matches {0:?}")]
    NotInstalled(String),
}

fn git(dir: &Path, args: &[&str]) -> Result<String, InstallError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if !out.status.success() {
        return Err(InstallError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// A fetched (not yet registered) plugin checkout.
#[derive(Debug)]
pub struct Fetched {
    pub spec: GithubSpec,
    pub requested_ref: Option<String>,
    pub resolved_commit: String,
    /// The checkout in the staging area.
    pub checkout: PathBuf,
    pub manifest: LoadedManifest,
}

impl Fetched {
    /// What will run, shown before asking for confirmation.
    pub fn preview(&self) -> String {
        let m = &self.manifest.manifest;
        let platform = manifest::current_platform();
        let mut s = format!(
            "Plugin {} {} ({})\nSource: github {}{}\nCommit: {}\n",
            m.id,
            m.version,
            m.name,
            self.spec.display(),
            self.requested_ref
                .as_deref()
                .map(|r| format!(" @ {r}"))
                .unwrap_or_default(),
            self.resolved_commit
        );
        let list = |title: &str, cmds: Vec<&Vec<String>>, s: &mut String| {
            if !cmds.is_empty() {
                s.push_str(&format!("{title}:\n"));
                for c in cmds {
                    s.push_str(&format!("  $ {}\n", c.join(" ")));
                }
            }
        };
        list(
            "Build commands (run now, as you)",
            m.build
                .iter()
                .filter(|b| platform_matches(&b.platforms, platform))
                .map(|b| &b.command)
                .collect(),
            &mut s,
        );
        list(
            "Startup commands (run when tt-git starts)",
            m.startup
                .iter()
                .filter(|b| platform_matches(&b.platforms, platform))
                .map(|b| &b.command)
                .collect(),
            &mut s,
        );
        s.push_str("Plugins run with your user's permissions and are not sandboxed.\n");
        s
    }

    fn root(&self) -> &Path {
        &self.manifest.root
    }
}

/// Shallow-fetches `spec` at `git_ref` (default: the remote HEAD) into a
/// staging directory and validates its manifest.
pub fn fetch(dirs: &PluginDirs, spec: &GithubSpec, git_ref: Option<&str>) -> Result<Fetched, InstallError> {
    let staging = dirs
        .installations_dir()
        .join(".staging")
        .join(format!("{}-{}", std::process::id(), nanos()));
    let checkout = staging.join("checkout");
    std::fs::create_dir_all(&checkout)?;
    let result = (|| {
        git(&checkout, &["init", "-q"])?;
        git(&checkout, &["remote", "add", "origin", &spec.url()])?;
        git(
            &checkout,
            &["fetch", "-q", "--depth", "1", "origin", git_ref.unwrap_or("HEAD")],
        )?;
        git(
            &checkout,
            &[
                "-c",
                "advice.detachedHead=false",
                "checkout",
                "-q",
                "--detach",
                "FETCH_HEAD",
            ],
        )?;
        let resolved_commit = git(&checkout, &["rev-parse", "HEAD"])?;
        let root = match &spec.subdir {
            Some(sub) => checkout.join(sub),
            None => checkout.clone(),
        };
        let loaded = manifest::load(&root, &crate::host_version())?;
        if !platform_matches(&loaded.manifest.platforms, manifest::current_platform()) {
            return Err(InstallError::Platform {
                id: loaded.manifest.id.clone(),
                platform: manifest::current_platform(),
            });
        }
        Ok((resolved_commit, loaded))
    })();
    match result {
        Ok((resolved_commit, manifest)) => Ok(Fetched {
            spec: spec.clone(),
            requested_ref: git_ref.map(str::to_string),
            resolved_commit,
            checkout,
            manifest,
        }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

/// Runs the `[[build]]` commands (inheriting stdio, without any runtime
/// `TTGIT_*` variables) and checks the manifest wasn't changed by them.
pub fn build(fetched: &Fetched) -> Result<(), InstallError> {
    let manifest_file = manifest::manifest_path(fetched.root());
    let before = std::fs::read(&manifest_file)?;
    let platform = manifest::current_platform();
    for b in fetched
        .manifest
        .manifest
        .build
        .iter()
        .filter(|b| platform_matches(&b.platforms, platform))
    {
        let (program, args) = b.command.split_first().expect("validated non-empty");
        let status = Command::new(crate::runtime::resolve_program(program, fetched.root()))
            .args(args)
            .current_dir(fetched.root())
            .status()?;
        if !status.success() {
            return Err(InstallError::BuildFailed {
                command: b.command.clone(),
                status: status.to_string(),
            });
        }
    }
    if std::fs::read(&manifest_file)? != before {
        return Err(InstallError::ManifestChanged(fetched.manifest.manifest.id.clone()));
    }
    Ok(())
}

/// Moves a fetched (and built) checkout into its generation directory and
/// registers it, replacing an earlier GitHub install of the same id.
pub fn commit(dirs: &PluginDirs, fetched: Fetched) -> Result<InstalledPlugin, InstallError> {
    let id = fetched.manifest.manifest.id.clone();
    let id_dir = dirs.installations_dir().join(escape_id(&id));
    std::fs::create_dir_all(&id_dir)?;
    let generation = std::fs::read_dir(&id_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str()?.rsplit_once('-')?.1.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let gen_dir = id_dir.join(format!("{}-{generation}", std::process::id()));
    std::fs::create_dir_all(&gen_dir)?;
    let dest = gen_dir.join("checkout");
    std::fs::rename(&fetched.checkout, &dest)?;
    if let Some(staging) = fetched.checkout.parent() {
        let _ = std::fs::remove_dir_all(staging);
    }
    let root = match &fetched.spec.subdir {
        Some(sub) => dest.join(sub),
        None => dest.clone(),
    };
    let entry = InstalledPlugin {
        id: id.clone(),
        manifest_path: root.join(manifest::MANIFEST_FILE),
        enabled: true,
        source: Source::Github {
            owner: fetched.spec.owner.clone(),
            repo: fetched.spec.repo.clone(),
            subdir: fetched.spec.subdir.clone(),
            requested_ref: fetched.requested_ref.clone(),
            resolved_commit: fetched.resolved_commit.clone(),
            managed_path: dest,
        },
        installed_unix_ms: (nanos() / 1_000_000) as u64,
    };
    let result = Registry::update(dirs, |reg| {
        let mut enabled = true;
        if let Some(existing) = reg.plugins.iter().find(|p| p.id == id) {
            if existing.source == Source::Local {
                return Err(RegistryError::AlreadyInstalled {
                    id: id.clone(),
                    source_desc: "a local link".into(),
                });
            }
            enabled = existing.enabled;
        }
        reg.plugins.retain(|p| p.id != id);
        let mut e = entry.clone();
        e.enabled = enabled;
        reg.plugins.push(e.clone());
        Ok(e)
    });
    match result {
        Ok(e) => {
            std::fs::create_dir_all(dirs.plugin_config_dir(&id))?;
            std::fs::create_dir_all(dirs.plugin_state_dir(&id))?;
            gc(dirs);
            Ok(e)
        }
        Err(RegistryError::AlreadyInstalled { .. }) => {
            let _ = std::fs::remove_dir_all(&gen_dir);
            Err(InstallError::LinkedLocally(id))
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&gen_dir);
            Err(e.into())
        }
    }
}

/// Refuses early when a local link already owns the id the spec would install.
pub fn check_not_linked(dirs: &PluginDirs, fetched: &Fetched) -> Result<(), InstallError> {
    let reg = Registry::read(dirs)?;
    match reg.get(&fetched.manifest.manifest.id) {
        Some(p) if p.source == Source::Local => Err(InstallError::LinkedLocally(p.id.clone())),
        _ => Ok(()),
    }
}

/// Discards a fetched checkout that won't be installed.
pub fn discard(fetched: Fetched) {
    if let Some(staging) = fetched.checkout.parent() {
        let _ = std::fs::remove_dir_all(staging);
    }
}

/// GitHub installs matching `targets` (plugin ids or `owner/repo[/subdir]`);
/// all of them when `targets` is empty. Linked plugins are never included.
pub fn github_installs(dirs: &PluginDirs, targets: &[String]) -> Result<Vec<InstalledPlugin>, InstallError> {
    let reg = Registry::read(dirs)?;
    let github: Vec<InstalledPlugin> = reg
        .plugins
        .into_iter()
        .filter(|p| matches!(p.source, Source::Github { .. }))
        .collect();
    if targets.is_empty() {
        return Ok(github);
    }
    let mut out = Vec::new();
    for t in targets {
        let spec = GithubSpec::parse(t).ok();
        let found: Vec<_> = github
            .iter()
            .filter(|p| p.id == *t || spec.as_ref().is_some_and(|s| s.matches(&p.source)))
            .cloned()
            .collect();
        if found.is_empty() {
            return Err(InstallError::NotInstalled(t.clone()));
        }
        out.extend(found);
    }
    Ok(out)
}

/// The spec and ref to fetch for updating an installed plugin.
pub fn update_source(p: &InstalledPlugin) -> Option<(GithubSpec, Option<String>, String)> {
    match &p.source {
        Source::Github {
            owner,
            repo,
            subdir,
            requested_ref,
            resolved_commit,
            ..
        } => Some((
            GithubSpec {
                owner: owner.clone(),
                repo: repo.clone(),
                subdir: subdir.clone(),
            },
            requested_ref.clone(),
            resolved_commit.clone(),
        )),
        Source::Local => None,
    }
}

/// Removes a plugin (by id or `owner/repo[/subdir]`) from the registry and
/// garbage-collects its files when nothing holds them.
pub fn uninstall(dirs: &PluginDirs, target: &str) -> Result<InstalledPlugin, InstallError> {
    let reg = Registry::read(dirs)?;
    let spec = GithubSpec::parse(target).ok();
    let id = reg
        .plugins
        .iter()
        .find(|p| p.id == target || spec.as_ref().is_some_and(|s| s.matches(&p.source)))
        .map(|p| p.id.clone())
        .ok_or_else(|| InstallError::NotInstalled(target.to_string()))?;
    let removed = registry::unlink(dirs, &id)?;
    gc(dirs);
    Ok(removed)
}

/// A running instance's claim on installation directories, so GC won't
/// delete files its plugins may still be using. Held for the life of the
/// value: the `.lock` file stays locked; the paths live in a `.json` beside it
/// (Windows locks are mandatory, so the locked file itself can't be read).
pub struct Lease {
    _lock: File,
    json: PathBuf,
}

fn leases_dir(dirs: &PluginDirs) -> PathBuf {
    dirs.state_dir.join("plugins").join("leases")
}

impl Lease {
    pub fn acquire(dirs: &PluginDirs, paths: &[PathBuf]) -> std::io::Result<Self> {
        let dir = leases_dir(dirs);
        std::fs::create_dir_all(&dir)?;
        let name = format!("{}-{}", std::process::id(), nanos());
        let lock = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(dir.join(format!("{name}.lock")))?;
        lock.lock()?;
        let json = dir.join(format!("{name}.json"));
        std::fs::write(&json, serde_json::to_vec(paths).unwrap_or_default())?;
        Ok(Self { _lock: lock, json })
    }

    /// Replaces the leased paths (after plugins reload).
    pub fn set_paths(&self, paths: &[PathBuf]) -> std::io::Result<()> {
        std::fs::write(&self.json, serde_json::to_vec(paths).unwrap_or_default())
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.json);
        let _ = std::fs::remove_file(self.json.with_extension("lock"));
    }
}

/// Paths leased by live instances; stale leases are removed.
fn leased_paths(dirs: &PluginDirs) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(leases_dir(dirs)) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for e in entries.filter_map(|e| e.ok()) {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "lock") {
            continue;
        }
        let json = path.with_extension("json");
        let live = match OpenOptions::new().write(true).open(&path) {
            Ok(f) => match f.try_lock() {
                Ok(()) => {
                    // Nobody holds it: the instance is gone.
                    let _ = f.unlock();
                    false
                }
                Err(_) => true,
            },
            Err(_) => false,
        };
        if live {
            if let Ok(bytes) = std::fs::read(&json)
                && let Ok(list) = serde_json::from_slice::<Vec<PathBuf>>(&bytes)
            {
                paths.extend(list);
            }
        } else {
            let _ = std::fs::remove_file(&json);
            let _ = std::fs::remove_file(&path);
        }
    }
    paths
}

/// The managed checkout of every registered GitHub plugin.
pub fn registered_paths(dirs: &PluginDirs) -> Vec<PathBuf> {
    Registry::read(dirs)
        .map(|r| managed_paths(&r.plugins))
        .unwrap_or_default()
}

/// The managed checkouts of those of `plugins` installed from GitHub.
pub fn managed_paths<'a>(plugins: impl IntoIterator<Item = &'a InstalledPlugin>) -> Vec<PathBuf> {
    plugins
        .into_iter()
        .filter_map(|p| match &p.source {
            Source::Github { managed_path, .. } => Some(managed_path.clone()),
            Source::Local => None,
        })
        .collect()
}

/// Deletes installation generations that are neither registered nor leased
/// by a running instance. Returns how many were removed.
pub fn gc(dirs: &PluginDirs) -> usize {
    let mut keep = registered_paths(dirs);
    keep.extend(leased_paths(dirs));
    let keep: Vec<PathBuf> = keep
        .iter()
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .collect();
    let mut removed = 0;
    let Ok(ids) = std::fs::read_dir(dirs.installations_dir()) else {
        return 0;
    };
    for id_dir in ids.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()) {
        if id_dir.file_name().is_some_and(|n| n == ".staging") {
            continue;
        }
        let Ok(gens) = std::fs::read_dir(&id_dir) else { continue };
        for gen_dir in gens.filter_map(|e| e.ok()).map(|e| e.path()) {
            let checkout = gen_dir.join("checkout");
            let canon = std::fs::canonicalize(&checkout).unwrap_or(checkout);
            if !keep.contains(&canon) && std::fs::remove_dir_all(&gen_dir).is_ok() {
                removed += 1;
            }
        }
        let _ = std::fs::remove_dir(&id_dir); // only succeeds when empty
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_specs() {
        let s = GithubSpec::parse("acme/tools").unwrap();
        assert_eq!(
            (s.owner.as_str(), s.repo.as_str(), s.subdir.as_deref()),
            ("acme", "tools", None)
        );
        let s = GithubSpec::parse("acme/tools.git/plugins/one/").unwrap();
        assert_eq!(s.repo, "tools");
        assert_eq!(s.subdir.as_deref(), Some("plugins/one"));
        assert_eq!(s.display(), "acme/tools/plugins/one");
        for bad in [
            "acme",
            "acme/",
            "/tools",
            "acme/../x",
            "ac me/tools",
            "acme/tools/../../etc",
        ] {
            assert!(GithubSpec::parse(bad).is_err(), "{bad}");
        }
    }
}
