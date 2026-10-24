//! User configuration loaded from `config.toml`.

use std::path::{Path, PathBuf};

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
    /// Regexes `tab` steps through in the `/` message filter. Unset, they're
    /// the subject prefixes the repo uses most, such as `^feat`.
    pub message_presets: Option<Vec<String>>,
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
            message_presets: None,
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

/// A repository's committed config file, in its working tree's root.
pub const SHARED_FILE: &str = ".tt-git.toml";
/// A repository's own config file that isn't committed, in its common git
/// directory so every worktree reads the same one.
pub const LOCAL_FILE: &str = "tt-git.toml";

/// Settings that run programs, which a committed [`SHARED_FILE`] can't set:
/// opening someone else's repo mustn't run their commands.
const SHARED_IGNORED: &[&[&str]] = &[&["editor"], &["agent"], &["keys", "command"]];

/// The config files that apply to a repository, in the order they're read:
/// the global file, then [`SHARED_FILE`] in `root`, then [`LOCAL_FILE`] in
/// `common_dir`. Later files win.
pub fn layer_paths(global: &Path, root: &Path, common_dir: &Path) -> [PathBuf; 3] {
    [
        global.to_path_buf(),
        root.join(SHARED_FILE),
        common_dir.join(LOCAL_FILE),
    ]
}

/// The global config file read once, ready to have each repository's own
/// files laid over it.
#[derive(Debug, Clone)]
pub struct GlobalConfig {
    path: PathBuf,
    table: toml::Table,
}

/// A repository's config: the global file with its own files laid over.
#[derive(Debug, Clone)]
pub struct LayeredConfig {
    pub config: Config,
    /// Keys left out of [`SHARED_FILE`] because only the user's own files
    /// can set them, like `editor.command`.
    pub ignored: Vec<String>,
}

impl GlobalConfig {
    /// Reads `path`; a missing file is the defaults.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let table = read_layer(path)?.unwrap_or_default();
        Ok(Self {
            path: path.to_path_buf(),
            table,
        })
    }

    /// The config for the repository at `root` with git directory
    /// `common_dir`: each file in [`layer_paths`] that exists merged over the
    /// one before. Tables merge key by key; anything else, arrays and
    /// `layout` included, is replaced whole.
    pub fn for_repo(&self, root: &Path, common_dir: &Path) -> Result<LayeredConfig, ConfigError> {
        let [_, shared, local] = layer_paths(&self.path, root, common_dir);
        let mut table = self.table.clone();
        let mut ignored = Vec::new();
        // A merged config that doesn't load is blamed on the last file in it.
        let mut last = &self.path;
        if let Some(mut layer) = read_layer(&shared)? {
            last = &shared;
            for key in SHARED_IGNORED {
                if remove_path(&mut layer, key) {
                    ignored.push(key.join("."));
                }
            }
            merge(&mut table, layer, true);
        }
        if let Some(layer) = read_layer(&local)? {
            last = &local;
            merge(&mut table, layer, true);
        }
        let config = to_config(last, table)?;
        Ok(LayeredConfig { config, ignored })
    }
}

/// `path` parsed as a table, checked on its own so a mistake is reported
/// against the file it's in; `None` when it doesn't exist.
fn read_layer(path: &Path) -> Result<Option<toml::Table>, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.display().to_string(),
                source,
            });
        }
    };
    let parse_error = |source| ConfigError::Parse {
        path: path.display().to_string(),
        source,
    };
    let table: toml::Table = toml::from_str(&text).map_err(parse_error)?;
    toml::Value::Table(table.clone())
        .try_into::<Config>()
        .map_err(parse_error)?;
    Ok(Some(table))
}

/// `table` as a [`Config`]; errors are reported against `path`.
fn to_config(path: &Path, table: toml::Table) -> Result<Config, ConfigError> {
    toml::Value::Table(table)
        .try_into()
        .map_err(|source| ConfigError::Parse {
            path: path.display().to_string(),
            source,
        })
}

/// Lays `over` onto `base`. `top` is set for the file's top level, where
/// `layout` is replaced rather than merged (two halves of different trees
/// aren't a tree).
fn merge(base: &mut toml::Table, over: toml::Table, top: bool) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) if !(top && key == "layout") => merge(b, o, false),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// Removes the key at `path` (`["keys", "command"]`), saying whether it was there.
fn remove_path(table: &mut toml::Table, path: &[&str]) -> bool {
    match path {
        [] => false,
        [key] => table.remove(*key).is_some(),
        [key, rest @ ..] => match table.get_mut(*key) {
            Some(toml::Value::Table(inner)) => remove_path(inner, rest),
            _ => false,
        },
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

    /// A global file, a repo root and a git dir in a temp dir, each file
    /// written when given.
    fn layers(global: Option<&str>, shared: Option<&str>, local: Option<&str>) -> (tempfile::TempDir, GlobalConfig) {
        let dir = tempfile::tempdir().unwrap();
        let [g, s, l] = layer_paths(&dir.path().join("config.toml"), dir.path(), &dir.path().join(".git"));
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        for (path, text) in [(&g, global), (&s, shared), (&l, local)] {
            if let Some(text) = text {
                std::fs::write(path, text).unwrap();
            }
        }
        let global = GlobalConfig::load(&g).unwrap();
        (dir, global)
    }

    fn for_repo(dir: &tempfile::TempDir, global: &GlobalConfig) -> Result<LayeredConfig, ConfigError> {
        global.for_repo(dir.path(), &dir.path().join(".git"))
    }

    #[test]
    fn project_files_override_the_global_one() {
        let (dir, global) = layers(
            Some("mouse = false\n[graph]\nmax_lanes = 4\nmessage_presets = [\"^a\", \"^b\"]\n"),
            Some("[graph]\nmax_lanes = 8\nmessage_presets = []\n"),
            Some("[graph]\nlanes = false\n"),
        );
        let cfg = for_repo(&dir, &global).unwrap().config;
        assert!(!cfg.mouse, "untouched global key kept");
        assert_eq!(cfg.graph.max_lanes, 8, "shared file wins over global");
        assert_eq!(cfg.graph.message_presets, Some(vec![]), "arrays are replaced");
        assert!(!cfg.graph.lanes, "local file merges into the same table");
        assert_eq!(cfg.graph.max_commits, 20_000, "defaults fill the rest");
    }

    #[test]
    fn local_file_wins_over_shared() {
        let (dir, global) = layers(None, Some("theme = \"nord\"\n"), Some("theme = \"dracula\"\n"));
        assert_eq!(
            for_repo(&dir, &global).unwrap().config.theme.as_deref(),
            Some("dracula")
        );
    }

    #[test]
    fn no_project_files_is_the_global_config() {
        let (dir, global) = layers(Some("syntax = false\n"), None, None);
        let layered = for_repo(&dir, &global).unwrap();
        assert!(!layered.config.syntax);
        assert!(layered.ignored.is_empty());
    }

    #[test]
    fn layout_is_replaced_not_merged() {
        let panes = "children = [{ pane = \"graph\" }, { pane = \"status\" }, { pane = \"branches\" }, { pane = \"details\" }]\n";
        let mine = format!("[layout]\nsplit = \"columns\"\n{panes}");
        // Merged, the global `size` would survive.
        let (dir, global) = layers(
            Some(&format!("[layout]\nsplit = \"rows\"\nsize = 50\n{panes}")),
            Some(&mine),
            None,
        );
        let cfg = for_repo(&dir, &global).unwrap().config;
        assert_eq!(cfg.layout, Config::parse(&mine).unwrap().layout);
    }

    #[test]
    fn shared_file_cannot_set_commands() {
        let shared = "[editor]\ncommand = \"evil {file}\"\n[agent]\ncommand = \"evil\"\n\
                      [[keys.command]]\nkey = \"ctrl+o\"\ntype = \"plugin_action\"\ncommand = \"a.b.c\"\n\
                      [keys.global]\nquit = \"x\"\n";
        let (dir, global) = layers(Some("[editor]\ncommand = \"mine {file}\"\n"), Some(shared), None);
        let layered = for_repo(&dir, &global).unwrap();
        assert_eq!(layered.ignored, ["editor", "agent", "keys.command"]);
        assert_eq!(layered.config.editor.command.as_deref(), Some("mine {file}"));
        assert_eq!(layered.config.agent.command, "claude");
        assert!(layered.config.keys.command.is_empty());
        // The rest of the file still applies.
        let km = layered.config.keymap().unwrap();
        assert_eq!(km.lookup(Context::Global, "x".parse().unwrap()), Some(Command::Quit));
    }

    #[test]
    fn local_file_can_set_commands() {
        let (dir, global) = layers(None, None, Some("[editor]\ncommand = \"mine {file}\"\n"));
        let layered = for_repo(&dir, &global).unwrap();
        assert!(layered.ignored.is_empty());
        assert_eq!(layered.config.editor.command.as_deref(), Some("mine {file}"));
    }

    #[test]
    fn a_bad_project_file_is_named() {
        let (dir, global) = layers(None, Some("[graph]\nmax_lanes = \"many\"\n"), None);
        let err = for_repo(&dir, &global).unwrap_err().to_string();
        assert!(err.contains(SHARED_FILE), "{err}");
        let (dir, global) = layers(None, None, Some("nope = 1\n"));
        let err = for_repo(&dir, &global).unwrap_err().to_string();
        assert!(err.contains(LOCAL_FILE) && err.contains("nope"), "{err}");
    }

    #[test]
    fn missing_file_is_default() {
        let cfg = Config::load(Path::new("definitely/not/here.toml")).unwrap();
        assert_eq!(cfg.layout, Layout::default());
    }
}
