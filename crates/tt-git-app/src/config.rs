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
    pub status: StatusConfig,
    pub branches: BranchesConfig,
    pub plugins: PluginsConfig,
    pub editor: EditorConfig,
    pub agent: AgentConfig,
    pub ui: UiConfig,
    pub fetch: FetchConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StatusConfig {
    /// List the changed files as a folder tree at startup (`` ` `` toggles
    /// it and saves the choice here).
    pub tree: bool,
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
            status: StatusConfig::default(),
            branches: BranchesConfig::default(),
            plugins: PluginsConfig::default(),
            editor: EditorConfig::default(),
            agent: AgentConfig::default(),
            ui: UiConfig::default(),
            fetch: FetchConfig::default(),
        }
    }
}

/// Fetching on its own while a repository is open.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FetchConfig {
    /// Minutes between the fetches of every remote tt-git makes in the
    /// background (`git fetch --all --prune`); `0` turns them off.
    pub interval: u32,
}

impl FetchConfig {
    /// The interval, or `None` when background fetching is off.
    pub fn background_interval(&self) -> Option<std::time::Duration> {
        (self.interval > 0).then(|| std::time::Duration::from_secs(u64::from(self.interval) * 60))
    }
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self { interval: 5 }
    }
}

/// The desktop app's own settings. The TUI ignores them.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// How big everything is drawn, in percent: text, spacing and widgets
    /// together, as a browser zooms. Zooming in the app saves it here.
    pub zoom: u16,
    /// The font everything is drawn in, by family name; unset is the OS's
    /// usual monospace font. Picking one in the app saves it here.
    pub font: Option<String>,
    /// The font size at 100% zoom, in pixels.
    pub font_size: f32,
    /// How the window is drawn: as a desktop app (cards, the OS's UI font
    /// around the panes) or as the terminal app is.
    pub style: UiStyle,
    /// Look for a new tt-git release in the background, once a day, and
    /// say in the help bar when there is one.
    pub check_for_updates: bool,
    /// Tell the desktop when new commits arrive while the window isn't
    /// focused: a notification, and a badge on the app's icon where the
    /// desktop has one.
    pub notifications: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            zoom: DEFAULT_ZOOM,
            font: None,
            font_size: DEFAULT_FONT_SIZE,
            style: UiStyle::default(),
            check_for_updates: true,
            notifications: false,
        }
    }
}

impl UiConfig {
    /// [`zoom`](Self::zoom), kept within [`ZOOM_LEVELS`].
    pub fn zoom(&self) -> u16 {
        clamp_zoom(self.zoom)
    }

    /// [`font`](Self::font), with an empty name as unset.
    pub fn font(&self) -> Option<&str> {
        self.font.as_deref().map(str::trim).filter(|f| !f.is_empty())
    }

    /// [`font_size`](Self::font_size), kept within [`FONT_SIZES`]; the
    /// default if it isn't a number.
    pub fn font_size(&self) -> f32 {
        if self.font_size.is_finite() {
            self.font_size.clamp(*FONT_SIZES.start(), *FONT_SIZES.end())
        } else {
            DEFAULT_FONT_SIZE
        }
    }
}

/// How the desktop app draws its window (`[ui] style`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiStyle {
    /// Panes as rounded cards with headers, a status bar of key chips and
    /// dialogs with shadows, in the OS's UI font around the panes.
    #[default]
    Native,
    /// Bordered boxes, a help bar and dialogs in one monospace font, as
    /// the terminal app draws them.
    Terminal,
}

impl UiStyle {
    /// The other style.
    pub fn toggled(self) -> Self {
        match self {
            UiStyle::Native => UiStyle::Terminal,
            UiStyle::Terminal => UiStyle::Native,
        }
    }

    /// Its name in config.toml.
    pub fn name(self) -> &'static str {
        match self {
            UiStyle::Native => "native",
            UiStyle::Terminal => "terminal",
        }
    }
}

pub const DEFAULT_ZOOM: u16 = 100;

pub const DEFAULT_FONT_SIZE: f32 = 13.;

/// The font sizes `[ui] font_size` can be, in pixels.
pub const FONT_SIZES: std::ops::RangeInclusive<f32> = 6.0..=72.0;

/// The zoom levels zooming in and out steps through, in percent: a
/// browser's, from 50 to 300.
pub const ZOOM_LEVELS: &[u16] = &[50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];

/// A way to change the zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomStep {
    In,
    Out,
    Reset,
}

/// `percent` within the smallest and largest of [`ZOOM_LEVELS`].
pub fn clamp_zoom(percent: u16) -> u16 {
    percent.clamp(ZOOM_LEVELS[0], ZOOM_LEVELS[ZOOM_LEVELS.len() - 1])
}

/// The zoom after `step` from `percent`: the next level up or down (from a
/// zoom between levels, the nearest one that way), or the default. At the
/// end of the levels it stays put.
pub fn step_zoom(percent: u16, step: ZoomStep) -> u16 {
    let percent = clamp_zoom(percent);
    match step {
        ZoomStep::In => ZOOM_LEVELS.iter().copied().find(|&z| z > percent).unwrap_or(percent),
        ZoomStep::Out => ZOOM_LEVELS
            .iter()
            .rev()
            .copied()
            .find(|&z| z < percent)
            .unwrap_or(percent),
        ZoomStep::Reset => DEFAULT_ZOOM,
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

impl GlobalConfig {
    /// Reads `path`; a missing file is the defaults.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let table = read_layer(path)?.unwrap_or_default();
        Ok(Self {
            path: path.to_path_buf(),
            table,
        })
    }

    /// The global file's settings on their own, for what applies to the
    /// whole app rather than one repository (the desktop app's `[ui]`).
    pub fn config(&self) -> Result<Config, ConfigError> {
        to_config(&self.path, self.table.clone())
    }

    /// The config for the repository at `root` with git directory
    /// `common_dir`: each file in [`layer_paths`] that exists merged over the
    /// one before. Tables merge key by key; anything else, arrays and
    /// `layout` included, is replaced whole.
    pub fn for_repo(&self, root: &Path, common_dir: &Path) -> Result<Config, ConfigError> {
        let [_, shared, local] = layer_paths(&self.path, root, common_dir);
        let mut table = self.table.clone();
        // A merged config that doesn't load is blamed on the last file in it.
        let mut last = &self.path;
        for path in [&shared, &local] {
            if let Some(layer) = read_layer(path)? {
                last = path;
                merge(&mut table, layer, true);
            }
        }
        to_config(last, table)
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

/// `text` (a config.toml) with `key` in table `[table]` set to the TOML
/// value `value`: its line replaced if the table has one, otherwise a line
/// added at the end of the table, or the table added at the end of the file.
/// Everything else, comments included, is left as it was.
pub fn set_in_table(text: &str, table: &str, key: &str, value: &str) -> String {
    let line = format!("{key} = {value}");
    let header = format!("[{table}]");
    let mut lines: Vec<&str> = text.lines().collect();
    let is_header = |l: &str| l.trim_start().starts_with('[');
    let is_key = |l: &str| {
        l.trim_start()
            .strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    };
    match lines.iter().position(|l| l.trim() == header) {
        Some(start) => {
            let end = lines[start + 1..]
                .iter()
                .position(|l| is_header(l))
                .map_or(lines.len(), |i| start + 1 + i);
            match lines[start + 1..end].iter().position(|l| is_key(l)) {
                Some(at) => lines[start + 1 + at] = &line,
                None => {
                    // After the table's last non-blank line.
                    let at = lines[start + 1..end]
                        .iter()
                        .rposition(|l| !l.trim().is_empty())
                        .map_or(start + 1, |i| start + 2 + i);
                    lines.insert(at, &line);
                }
            }
        }
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push("");
            }
            lines.push(&header);
            lines.push(&line);
        }
    }
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = lines.join(eol);
    out.push_str(eol);
    out
}

/// `text` (a config.toml) without `key` in table `[table]`. Everything
/// else, comments and the table's header included, is left as it was.
pub fn remove_in_table(text: &str, table: &str, key: &str) -> String {
    let header = format!("[{table}]");
    let mut lines: Vec<&str> = text.lines().collect();
    let is_key = |l: &str| {
        l.trim_start()
            .strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    };
    let Some(start) = lines.iter().position(|l| l.trim() == header) else {
        return text.to_string();
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |i| start + 1 + i);
    let Some(at) = lines[start + 1..end].iter().position(|l| is_key(l)) else {
        return text.to_string();
    };
    lines.remove(start + 1 + at);
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
    fn background_fetch_interval() {
        let minutes = |text| Config::parse(text).unwrap().fetch.background_interval();
        assert_eq!(minutes(""), Some(std::time::Duration::from_secs(5 * 60)));
        assert_eq!(
            minutes("[fetch]\ninterval = 30"),
            Some(std::time::Duration::from_secs(30 * 60))
        );
        assert_eq!(minutes("[fetch]\ninterval = 0"), None);
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
    fn set_in_table_keeps_the_rest() {
        assert_eq!(set_in_table("", "status", "tree", "true"), "[status]\ntree = true\n");
        // A new table goes at the end, after a blank line.
        assert_eq!(
            set_in_table("mouse = false\n", "status", "tree", "true"),
            "mouse = false\n\n[status]\ntree = true\n"
        );
        // Replaced in its own table only; `tree_x` isn't `tree`.
        let text = "tree = 1\n[status]\ntree_x = 2\ntree = false # mine\n\n[graph]\ntree = 3\n";
        assert_eq!(
            set_in_table(text, "status", "tree", "true"),
            "tree = 1\n[status]\ntree_x = 2\ntree = true\n\n[graph]\ntree = 3\n"
        );
        // Added after the table's last line, before the blank one.
        assert_eq!(
            set_in_table("[status]\n# note\n\n[graph]\n", "status", "tree", "true"),
            "[status]\n# note\ntree = true\n\n[graph]\n"
        );
        let text = set_in_table("[graph]\r\nmax_lanes = 4\r\n", "status", "tree", "true");
        assert_eq!(text, "[graph]\r\nmax_lanes = 4\r\n\r\n[status]\r\ntree = true\r\n");
        assert!(Config::parse(&text).unwrap().status.tree);
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

    fn for_repo(dir: &tempfile::TempDir, global: &GlobalConfig) -> Result<Config, ConfigError> {
        global.for_repo(dir.path(), &dir.path().join(".git"))
    }

    #[test]
    fn project_files_override_the_global_one() {
        let (dir, global) = layers(
            Some("mouse = false\n[graph]\nmax_lanes = 4\nmessage_presets = [\"^a\", \"^b\"]\n"),
            Some("[graph]\nmax_lanes = 8\nmessage_presets = []\n"),
            Some("[graph]\nlanes = false\n"),
        );
        let cfg = for_repo(&dir, &global).unwrap();
        assert!(!cfg.mouse, "untouched global key kept");
        assert_eq!(cfg.graph.max_lanes, 8, "shared file wins over global");
        assert_eq!(cfg.graph.message_presets, Some(vec![]), "arrays are replaced");
        assert!(!cfg.graph.lanes, "local file merges into the same table");
        assert_eq!(cfg.graph.max_commits, 20_000, "defaults fill the rest");
    }

    #[test]
    fn local_file_wins_over_shared() {
        let (dir, global) = layers(None, Some("theme = \"nord\"\n"), Some("theme = \"dracula\"\n"));
        assert_eq!(for_repo(&dir, &global).unwrap().theme.as_deref(), Some("dracula"));
    }

    #[test]
    fn no_project_files_is_the_global_config() {
        let (dir, global) = layers(Some("syntax = false\n"), None, None);
        assert!(!for_repo(&dir, &global).unwrap().syntax);
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
        let cfg = for_repo(&dir, &global).unwrap();
        assert_eq!(cfg.layout, Config::parse(&mine).unwrap().layout);
    }

    #[test]
    fn shared_file_can_set_anything() {
        let shared = "[editor]\ncommand = \"repo {file}\"\n[agent]\ncommand = \"repo-agent\"\n\
                      [plugins]\npane_release_keys = [\"f11\"]\n\
                      [[keys.command]]\nkey = \"ctrl+o\"\ntype = \"plugin_action\"\ncommand = \"a.b.c\"\n";
        let (dir, global) = layers(Some("[editor]\ncommand = \"mine {file}\"\n"), Some(shared), None);
        let cfg = for_repo(&dir, &global).unwrap();
        assert_eq!(cfg.editor.command.as_deref(), Some("repo {file}"));
        assert_eq!(cfg.agent.command, "repo-agent");
        assert_eq!(cfg.plugins.pane_release_keys, ["f11"]);
        assert_eq!(cfg.keys.command.len(), 1);
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

    #[test]
    fn ui_zoom_defaults_and_clamps() {
        assert_eq!(Config::default().ui.zoom(), 100);
        assert_eq!(
            Config::parse(
                "[ui]
zoom = 125
"
            )
            .unwrap()
            .ui
            .zoom(),
            125
        );
        assert_eq!(
            Config::parse(
                "[ui]
zoom = 10
"
            )
            .unwrap()
            .ui
            .zoom(),
            50
        );
        assert_eq!(
            Config::parse(
                "[ui]
zoom = 900
"
            )
            .unwrap()
            .ui
            .zoom(),
            300
        );
    }

    #[test]
    fn ui_font_defaults_and_clamps() {
        let ui = Config::default().ui;
        assert_eq!((ui.font(), ui.font_size()), (None, 13.));
        let ui = Config::parse("[ui]\nfont = \"Fira Code\"\nfont_size = 15.5\n")
            .unwrap()
            .ui;
        assert_eq!((ui.font(), ui.font_size()), (Some("Fira Code"), 15.5));
        let ui = Config::parse("[ui]\nfont = \"  \"\nfont_size = 2\n").unwrap().ui;
        assert_eq!((ui.font(), ui.font_size()), (None, 6.));
        assert_eq!(Config::parse("[ui]\nfont_size = 500\n").unwrap().ui.font_size(), 72.);
    }

    #[test]
    fn ui_notifications_are_off_by_default() {
        assert!(!Config::default().ui.notifications);
        assert!(Config::parse("[ui]\nnotifications = true\n").unwrap().ui.notifications);
    }

    #[test]
    fn ui_style_defaults_to_native() {
        assert_eq!(Config::default().ui.style, UiStyle::Native);
        let ui = Config::parse("[ui]\nstyle = \"terminal\"\n").unwrap().ui;
        assert_eq!(ui.style, UiStyle::Terminal);
        assert_eq!(ui.style.toggled().name(), "native");
        assert!(Config::parse("[ui]\nstyle = \"fancy\"\n").is_err());
    }

    #[test]
    fn remove_in_table_keeps_the_rest() {
        let text = "font = 1\n[ui]\nfont_size = 14\nfont = \"Fira Code\" # mine\n\n[graph]\nfont = 3\n";
        assert_eq!(
            remove_in_table(text, "ui", "font"),
            "font = 1\n[ui]\nfont_size = 14\n\n[graph]\nfont = 3\n"
        );
        assert_eq!(remove_in_table(text, "status", "font"), text, "no such table");
        assert_eq!(
            remove_in_table("[ui]\nzoom = 110\n", "ui", "font"),
            "[ui]\nzoom = 110\n"
        );
        assert_eq!(
            remove_in_table("[ui]\r\nfont = \"A\"\r\nzoom = 1\r\n", "ui", "font"),
            "[ui]\r\nzoom = 1\r\n"
        );
    }

    #[test]
    fn zoom_steps_through_the_levels() {
        assert_eq!(step_zoom(100, ZoomStep::In), 110);
        assert_eq!(step_zoom(100, ZoomStep::Out), 90);
        assert_eq!(step_zoom(115, ZoomStep::In), 125, "between levels, the next one up");
        assert_eq!(step_zoom(115, ZoomStep::Out), 110);
        assert_eq!(step_zoom(300, ZoomStep::In), 300);
        assert_eq!(step_zoom(50, ZoomStep::Out), 50);
        assert_eq!(step_zoom(175, ZoomStep::Reset), 100);
    }

    #[test]
    fn global_config_alone() {
        let (_dir, global) = layers(
            Some(
                "[ui]
zoom = 150
",
            ),
            Some(
                "[ui]
zoom = 80
",
            ),
            None,
        );
        assert_eq!(global.config().unwrap().ui.zoom(), 150);
    }
}
