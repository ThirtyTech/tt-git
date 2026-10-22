//! User configuration loaded from `config.toml`.

use std::path::Path;

use serde::Deserialize;

use crate::agent::AgentConfig;
use crate::editor::EditorConfig;
use crate::keymap::{Keymap, KeymapError, KeysConfig};
use crate::layout::Layout;
use crate::panes::DateStyle;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Refresh automatically when files or refs change on disk.
    pub watch: bool,
    /// Click to focus/select, wheel to scroll, ctrl+click links.
    pub mouse: bool,
    /// Color code by language in blame and diffs.
    pub syntax: bool,
    pub keys: KeysConfig,
    pub layout: Layout,
    pub graph: GraphConfig,
    pub branches: BranchesConfig,
    pub plugins: PluginsConfig,
    pub editor: EditorConfig,
    pub agent: AgentConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BranchesConfig {
    /// Unmerged branches whose name contains any of these are hidden.
    pub unmerged_exclude: Vec<String>,
    /// Long-lived branches, by exact name (on any remote), never listed on
    /// the merged and unmerged tabs.
    pub safe_branches: Vec<String>,
}

impl Default for BranchesConfig {
    fn default() -> Self {
        Self {
            unmerged_exclude: vec!["archive".into()],
            safe_branches: ["master", "main", "production", "prod"].map(String::from).to_vec(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginsConfig {
    /// Keys that leave (and, for overlays/popups, close) a plugin pane.
    pub pane_release_keys: Vec<String>,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            pane_release_keys: vec!["ctrl+\\".into(), "f12".into()],
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            watch: true,
            mouse: true,
            syntax: true,
            keys: KeysConfig::default(),
            layout: Layout::default(),
            graph: GraphConfig::default(),
            branches: BranchesConfig::default(),
            plugins: PluginsConfig::default(),
            editor: EditorConfig::default(),
            agent: AgentConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GraphConfig {
    /// Commits loaded at a time; scrolling near the end loads this many more.
    pub max_commits: usize,
    /// Lanes drawn before the rest collapse into `…`.
    pub max_lanes: usize,
    /// Show `HEAD`'s upstream alongside it in current-branch mode.
    pub current_includes_upstream: bool,
    /// How the date column reads at startup (`t` toggles it).
    pub dates: DateStyle,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            max_commits: 20_000,
            max_lanes: 16,
            current_includes_upstream: true,
            dates: DateStyle::Relative,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading {path}: {source}")]
    Read { path: String, source: std::io::Error },
    #[error("parsing {path}: {source}")]
    Parse { path: String, source: toml::de::Error },
    #[error(transparent)]
    Keymap(#[from] KeymapError),
}

impl Config {
    /// Loads `path`, or returns the defaults when it does not exist.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|source| ConfigError::Parse {
                path: path.display().to_string(),
                source,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Read {
                path: path.display().to_string(),
                source,
            }),
        }
    }

    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn keymap(&self) -> Result<Keymap, KeymapError> {
        Keymap::from_config(&self.keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{Command, Context};

    #[test]
    fn parses_full_config() {
        let cfg = Config::parse(
            r#"
            mouse = false
            [keys.global]
            quit = "x"
            "#,
        )
        .unwrap();
        assert!(!cfg.mouse);
        assert!(cfg.watch);
        assert!(cfg.syntax);
        assert!(!Config::parse("syntax = false").unwrap().syntax);
        let km = cfg.keymap().unwrap();
        assert_eq!(km.lookup(Context::Global, "x".parse().unwrap()), Some(Command::Quit));
    }

    #[test]
    fn rejects_unknown_command() {
        let err = Config::parse("[keys.global]\nfly = \"f\"").unwrap_err();
        assert!(err.to_string().contains("unknown variant `fly`"), "{err}");
    }

    #[test]
    fn missing_file_is_default() {
        let cfg = Config::load(Path::new("definitely/not/here.toml")).unwrap();
        assert_eq!(cfg.layout, Layout::default());
    }
}
