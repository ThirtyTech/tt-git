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
    /// The color theme, by name (see `themes`); unset is `default`.
    pub theme: Option<String>,
    /// Paint the theme's background instead of leaving the terminal's.
    pub theme_background: bool,
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
    /// Check in the background whether each branch and worktree would merge
    /// into the base branch cleanly.
    pub check_merges: bool,
}

impl Default for BranchesConfig {
    fn default() -> Self {
        Self {
            unmerged_exclude: vec!["archive".into()],
            safe_branches: ["master", "main", "production", "prod"].map(String::from).to_vec(),
            check_merges: true,
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
            theme: None,
            theme_background: false,
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
    /// Draw the branch lanes at startup (`L` toggles them).
    pub lanes: bool,
    /// Started in a subfolder of the repository, narrow the graph and
    /// Status to it at startup (`.` toggles it).
    pub filter_to_cwd: bool,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            max_commits: 20_000,
            max_lanes: 16,
            current_includes_upstream: true,
            dates: DateStyle::Relative,
            lanes: true,
            filter_to_cwd: false,
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

/// `text` (a config.toml) with top-level `key` set to the TOML value
/// `value`: its line replaced if it has one, otherwise a line added above the
/// first table. Everything else, comments included, is left as it was.
pub fn set_top_level(text: &str, key: &str, value: &str) -> String {
    let line = format!("{key} = {value}");
    let mut lines: Vec<&str> = text.lines().collect();
    let first_table = lines
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .unwrap_or(lines.len());
    let is_key = |l: &str| {
        l.trim_start()
            .strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    };
    let top = &lines[..first_table];
    match top.iter().position(|l| is_key(l)) {
        Some(at) => lines[at] = &line,
        None => {
            // After the last top-level key, or at the top when there's none.
            let at = top
                .iter()
                .rposition(|l| l.contains('=') && !l.trim_start().starts_with('#'))
                .map_or(0, |i| i + 1);
            if lines.get(at).is_some_and(|l| l.trim_start().starts_with('[')) {
                lines.insert(at, "");
            }
            lines.insert(at, &line);
        }
    }
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = lines.join(eol);
    out.push_str(eol);
    out
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
    fn set_top_level_keeps_the_rest() {
        assert_eq!(set_top_level("", "theme", "\"nord\""), "theme = \"nord\"\n");
        // Goes after the top-level keys, `theme_background` isn't `theme`,
        // and a table's own `theme` is left alone.
        let text = "# mine\nmouse = false\ntheme_background = true\n\n[graph]\ntheme = \"x\"\n";
        assert_eq!(
            set_top_level(text, "theme", "\"nord\""),
            "# mine\nmouse = false\ntheme_background = true\ntheme = \"nord\"\n\n[graph]\ntheme = \"x\"\n"
        );
        let text = "theme   = \"dracula\" # dark\n[graph]\nmax_lanes = 4\n";
        assert_eq!(
            set_top_level(text, "theme", "\"nord\""),
            "theme = \"nord\"\n[graph]\nmax_lanes = 4\n"
        );
        assert_eq!(
            set_top_level("[graph]\n", "theme", "\"nord\""),
            "theme = \"nord\"\n\n[graph]\n"
        );
        let cfg = Config::parse(&set_top_level("[graph]\nmax_lanes = 4\n", "theme", "\"nord\"")).unwrap();
        assert_eq!(cfg.theme.as_deref(), Some("nord"));
    }

    #[test]
    fn missing_file_is_default() {
        let cfg = Config::load(Path::new("definitely/not/here.toml")).unwrap();
        assert_eq!(cfg.layout, Layout::default());
    }
}
