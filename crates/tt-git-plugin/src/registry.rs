//! The installed-plugin registry, `<config_dir>/plugins.json`.
//!
//! Installs are global per user. Every read-modify-write holds an exclusive
//! lock on `plugins.json.lock` so concurrent `tt-git plugin …` commands and
//! running instances don't clobber each other.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::manifest::{self, LoadedManifest};

/// Where plugin files live, rooted at tt-git's config and state dirs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDirs {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl PluginDirs {
    pub fn from_env() -> Self {
        Self {
            config_dir: tt_git_core::paths::config_dir(),
            state_dir: tt_git_core::paths::state_dir(),
        }
    }

    pub fn registry_file(&self) -> PathBuf {
        self.config_dir.join("plugins.json")
    }

    fn lock_file(&self) -> PathBuf {
        self.config_dir.join("plugins.json.lock")
    }

    /// Managed checkouts of GitHub installs.
    pub fn installations_dir(&self) -> PathBuf {
        self.config_dir.join("plugins").join("github-installations")
    }

    /// User-editable files for a plugin (`TTGIT_PLUGIN_CONFIG_DIR`).
    pub fn plugin_config_dir(&self, id: &str) -> PathBuf {
        self.config_dir.join("plugins").join("config").join(escape_id(id))
    }

    /// Plugin-private state (`TTGIT_PLUGIN_STATE_DIR`).
    pub fn plugin_state_dir(&self, id: &str) -> PathBuf {
        self.state_dir.join("plugins").join(escape_id(id))
    }
}

/// Makes a plugin id safe as a single path component on every OS (`:` is
/// not allowed in Windows file names).
pub fn escape_id(id: &str) -> String {
    id.replace(':', "_")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    Local,
    Github {
        owner: String,
        repo: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subdir: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requested_ref: Option<String>,
        resolved_commit: String,
        managed_path: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub id: String,
    /// The manifest file (re-read every time the registry is loaded).
    pub manifest_path: PathBuf,
    pub enabled: bool,
    pub source: Source,
    pub installed_unix_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub plugins: Vec<InstalledPlugin>,
}

/// An installed plugin with its manifest re-read from disk. A missing or
/// broken manifest keeps the entry, with the problem in `warnings`.
#[derive(Debug, Clone)]
pub struct PluginRecord {
    pub installed: InstalledPlugin,
    pub manifest: Option<LoadedManifest>,
    pub warnings: Vec<String>,
}

impl PluginRecord {
    /// Enabled, with a valid manifest that supports this platform.
    pub fn is_runnable(&self) -> bool {
        self.installed.enabled
            && self
                .manifest
                .as_ref()
                .is_some_and(|m| manifest::platform_matches(&m.manifest.platforms, manifest::current_platform()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("plugins.json is corrupt: {0}")]
    Corrupt(#[from] serde_json::Error),
    #[error(transparent)]
    Manifest(#[from] manifest::ManifestError),
    #[error("plugin_not_found: no installed plugin {0:?}")]
    NotFound(String),
    #[error("plugin_already_installed: {id} is already installed from {source_desc}")]
    AlreadyInstalled { id: String, source_desc: String },
    #[error("platform_unsupported: {id} does not support {platform}")]
    PlatformUnsupported { id: String, platform: &'static str },
}

/// Exclusive lock on the registry for one read-modify-write.
struct Lock(File);

impl Lock {
    fn acquire(dirs: &PluginDirs) -> Result<Self, RegistryError> {
        std::fs::create_dir_all(&dirs.config_dir)?;
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dirs.lock_file())?;
        f.lock()?;
        Ok(Self(f))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Registry {
    /// Reads the registry (empty if it doesn't exist yet).
    pub fn read(dirs: &PluginDirs) -> Result<Self, RegistryError> {
        match std::fs::read(dirs.registry_file()) {
            Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => Ok(Self::default()),
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn write(&self, dirs: &PluginDirs) -> Result<(), RegistryError> {
        std::fs::create_dir_all(&dirs.config_dir)?;
        let tmp = dirs.config_dir.join("plugins.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, dirs.registry_file())?;
        Ok(())
    }

    /// Runs `f` on the registry under the lock and saves the result.
    pub fn update<T>(
        dirs: &PluginDirs,
        f: impl FnOnce(&mut Registry) -> Result<T, RegistryError>,
    ) -> Result<T, RegistryError> {
        let _lock = Lock::acquire(dirs)?;
        let mut reg = Self::read(dirs)?;
        let out = f(&mut reg)?;
        reg.write(dirs)?;
        Ok(out)
    }

    pub fn get(&self, id: &str) -> Option<&InstalledPlugin> {
        self.plugins.iter().find(|p| p.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut InstalledPlugin, RegistryError> {
        self.plugins
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| RegistryError::NotFound(id.to_string()))
    }

    /// Every installed plugin with its manifest loaded.
    pub fn records(&self, host_version: &semver::Version) -> Vec<PluginRecord> {
        self.plugins
            .iter()
            .map(|p| match manifest::load(&p.manifest_path, host_version) {
                Ok(m) => PluginRecord {
                    warnings: m.warnings.clone(),
                    installed: p.clone(),
                    manifest: Some(m),
                },
                Err(e) => PluginRecord {
                    installed: p.clone(),
                    manifest: None,
                    warnings: vec![e.to_string()],
                },
            })
            .collect()
    }
}

/// Registers a local plugin directory (or manifest path) for development.
/// No build step runs. Re-linking the same id updates its path.
pub fn link(
    dirs: &PluginDirs,
    path: &Path,
    enabled: bool,
    host_version: &semver::Version,
) -> Result<PluginRecord, RegistryError> {
    let file = manifest::manifest_path(path);
    let file = std::fs::canonicalize(&file).unwrap_or(file);
    let loaded = manifest::load(&file, host_version)?;
    let id = loaded.manifest.id.clone();
    if !manifest::platform_matches(&loaded.manifest.platforms, manifest::current_platform()) {
        return Err(RegistryError::PlatformUnsupported {
            id,
            platform: manifest::current_platform(),
        });
    }
    let installed = Registry::update(dirs, |reg| {
        if let Some(existing) = reg.plugins.iter().find(|p| p.id == id)
            && existing.source != Source::Local
        {
            return Err(RegistryError::AlreadyInstalled {
                id: id.clone(),
                source_desc: "GitHub".into(),
            });
        }
        reg.plugins.retain(|p| p.id != id);
        let entry = InstalledPlugin {
            id: id.clone(),
            manifest_path: file.clone(),
            enabled,
            source: Source::Local,
            installed_unix_ms: now_ms(),
        };
        reg.plugins.push(entry.clone());
        Ok(entry)
    })?;
    std::fs::create_dir_all(dirs.plugin_config_dir(&id))?;
    std::fs::create_dir_all(dirs.plugin_state_dir(&id))?;
    Ok(PluginRecord {
        warnings: loaded.warnings.clone(),
        installed,
        manifest: Some(loaded),
    })
}

/// Removes a plugin from the registry. Files are left in place.
pub fn unlink(dirs: &PluginDirs, id: &str) -> Result<InstalledPlugin, RegistryError> {
    Registry::update(dirs, |reg| {
        let pos = reg
            .plugins
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| RegistryError::NotFound(id.to_string()))?;
        Ok(reg.plugins.remove(pos))
    })
}

pub fn set_enabled(dirs: &PluginDirs, id: &str, enabled: bool) -> Result<InstalledPlugin, RegistryError> {
    Registry::update(dirs, |reg| {
        let p = reg.get_mut(id)?;
        p.enabled = enabled;
        Ok(p.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> semver::Version {
        semver::Version::new(0, 1, 0)
    }

    fn dirs(tmp: &Path) -> PluginDirs {
        PluginDirs {
            config_dir: tmp.join("config"),
            state_dir: tmp.join("state"),
        }
    }

    fn write_plugin(dir: &Path, id: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(manifest::MANIFEST_FILE),
            format!(
                "id = \"{id}\"\nname = \"Test\"\nversion = \"0.1.0\"\nmin_tt_git_version = \"0.1.0\"\n\
                 [[actions]]\nid = \"hello\"\ntitle = \"Hello\"\ncommand = [\"echo\", \"hi\"]\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn link_list_enable_unlink() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let plugin = tmp.path().join("p");
        write_plugin(&plugin, "acme:one");

        let rec = link(&d, &plugin, true, &host()).unwrap();
        assert_eq!(rec.installed.id, "acme:one");
        assert!(rec.is_runnable());
        assert!(d.plugin_config_dir("acme:one").ends_with("acme_one"));
        assert!(d.plugin_config_dir("acme:one").is_dir());
        assert!(d.plugin_state_dir("acme:one").is_dir());

        // Relinking replaces rather than duplicates.
        link(&d, &plugin.join(manifest::MANIFEST_FILE), false, &host()).unwrap();
        let reg = Registry::read(&d).unwrap();
        assert_eq!(reg.plugins.len(), 1);
        assert!(!reg.plugins[0].enabled);

        set_enabled(&d, "acme:one", true).unwrap();
        assert!(Registry::read(&d).unwrap().plugins[0].enabled);

        unlink(&d, "acme:one").unwrap();
        assert!(Registry::read(&d).unwrap().plugins.is_empty());
        assert!(matches!(unlink(&d, "acme:one"), Err(RegistryError::NotFound(_))));
        assert!(plugin.join(manifest::MANIFEST_FILE).exists(), "files are kept");
    }

    #[test]
    fn broken_manifest_keeps_entry_with_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let plugin = tmp.path().join("p");
        write_plugin(&plugin, "acme.two");
        link(&d, &plugin, true, &host()).unwrap();
        std::fs::write(plugin.join(manifest::MANIFEST_FILE), "garbage = [").unwrap();
        let recs = Registry::read(&d).unwrap().records(&host());
        assert_eq!(recs.len(), 1);
        assert!(recs[0].manifest.is_none());
        assert!(!recs[0].is_runnable());
        assert!(recs[0].warnings[0].contains("plugin_manifest_parse_failed"));
    }

    #[test]
    fn link_rejects_invalid_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let err = link(&d, &tmp.path().join("nope"), true, &host()).unwrap_err();
        assert!(err.to_string().contains("plugin_manifest_not_found"), "{err}");
    }

    #[test]
    fn concurrent_updates_do_not_lose_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let d = d.clone();
                let plugin = tmp.path().join(format!("p{i}"));
                write_plugin(&plugin, &format!("acme.p{i}"));
                std::thread::spawn(move || link(&d, &plugin, true, &host()).unwrap())
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(Registry::read(&d).unwrap().plugins.len(), 8);
    }
}
