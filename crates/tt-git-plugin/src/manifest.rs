//! `tt-git-plugin.toml`: the plugin manifest (herdr v1 format, renamed).
//!
//! ```toml
//! id = "acme.github"
//! name = "GitHub helpers"
//! version = "0.1.0"
//! min_tt_git_version = "0.1.0"
//! platforms = ["linux", "macos", "windows"]
//!
//! [[actions]]
//! id = "open-commit"
//! title = "Open commit on GitHub"
//! contexts = ["commit"]
//! command = ["node", "open-commit.js"]
//!
//! [[events]]
//! on = "push.completed"
//! command = ["./notify.sh"]
//! ```

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MANIFEST_FILE: &str = "tt-git-plugin.toml";
const MAX_ID_LEN: usize = 120;

/// Events a plugin can hook with `[[events]]`. High-volume events (repo
/// changes, selection moves) are only available through `events.subscribe`.
pub const HOOK_EVENTS: &[&str] = &[
    "app.started",
    "repo.opened",
    "head.changed",
    "branch.checked_out",
    "branch.created",
    "branch.deleted",
    "branch.renamed",
    "commit.created",
    "fetch.completed",
    "pull.completed",
    "push.completed",
    "files.staged",
    "files.unstaged",
    "files.discarded",
    "merge.completed",
    "rebase.completed",
    "cherry_pick.completed",
    "worktree.removed",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub min_tt_git_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
    #[serde(default)]
    pub build: Vec<CommandSpec>,
    #[serde(default)]
    pub startup: Vec<CommandSpec>,
    #[serde(default)]
    pub actions: Vec<ActionSpec>,
    #[serde(default)]
    pub events: Vec<EventSpec>,
    #[serde(default)]
    pub panes: Vec<PaneSpec>,
    #[serde(default)]
    pub link_handlers: Vec<LinkHandlerSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
}

/// Where an action is offered in the command palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionContext {
    Global,
    /// A commit is selected in the graph.
    Commit,
    /// A branch is selected in the branch list.
    Branch,
    /// A file is selected in the status pane.
    File,
    Selection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionSpec {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub contexts: Vec<ActionContext>,
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
}

impl ActionSpec {
    /// Contexts, defaulting to `global` when none are listed.
    pub fn effective_contexts(&self) -> Vec<ActionContext> {
        if self.contexts.is_empty() {
            vec![ActionContext::Global]
        } else {
            self.contexts.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventSpec {
    pub on: String,
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    Overlay,
    Popup,
    Split,
    Tab,
    Zoomed,
}

/// A popup dimension: a cell count or a percentage string like `"80%"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dimension {
    Cells(u16),
    Percent(String),
}

impl Dimension {
    /// Resolves against the available size.
    pub fn resolve(&self, available: u16) -> u16 {
        match self {
            Dimension::Cells(n) => (*n).min(available),
            Dimension::Percent(s) => {
                let pct: u32 = s.trim_end_matches('%').trim().parse().unwrap_or(80);
                ((available as u32 * pct.min(100)) / 100) as u16
            }
        }
    }

    fn is_valid(&self) -> bool {
        match self {
            Dimension::Cells(n) => *n > 0,
            Dimension::Percent(s) => s
                .strip_suffix('%')
                .and_then(|n| n.trim().parse::<u32>().ok())
                .is_some_and(|n| (1..=100).contains(&n)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneSpec {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub placement: Placement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Dimension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Dimension>,
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkHandlerSpec {
    pub id: String,
    pub title: String,
    /// Rust regex matched against clicked URLs.
    pub pattern: String,
    /// Local id of an action in the same plugin.
    pub action: String,
}

/// Why a manifest was rejected. The `code` strings match herdr's error codes
/// where one exists.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ManifestError {
    pub code: &'static str,
    pub message: String,
}

impl ManifestError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// A validated manifest and the non-fatal problems found in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedManifest {
    pub manifest: Manifest,
    /// Directory containing the manifest (commands run here).
    pub root: PathBuf,
    pub warnings: Vec<String>,
    /// `manifest.link_handlers`' patterns, compiled.
    pub link_patterns: LinkPatterns,
}

/// Compiled link handler patterns, one per handler in manifest order.
#[derive(Debug, Clone, Default)]
pub struct LinkPatterns(pub Vec<regex::Regex>);

impl PartialEq for LinkPatterns {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len() && self.0.iter().zip(&other.0).all(|(a, b)| a.as_str() == b.as_str())
    }
}

impl Eq for LinkPatterns {}

/// The current OS as used in `platforms` lists.
pub fn current_platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

const KNOWN_PLATFORMS: &[&str] = &["linux", "macos", "windows"];

/// Whether an item with this `platforms` list runs here.
pub fn platform_matches(platforms: &Option<Vec<String>>, platform: &str) -> bool {
    platforms.as_ref().is_none_or(|p| p.iter().any(|x| x == platform))
}

/// `path` may be the manifest file itself or the directory containing it.
pub fn manifest_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join(MANIFEST_FILE)
    } else {
        path.to_path_buf()
    }
}

/// Reads and validates a manifest. `host_version` is tt-git's own version.
pub fn load(path: &Path, host_version: &semver::Version) -> Result<LoadedManifest, ManifestError> {
    let file = manifest_path(path);
    let text = std::fs::read_to_string(&file)
        .map_err(|e| ManifestError::new("plugin_manifest_not_found", format!("{}: {e}", file.display())))?;
    let root = file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let manifest = parse(&text)?;
    let (warnings, link_patterns) = validate_compiling(&manifest, host_version)?;
    Ok(LoadedManifest {
        manifest,
        root,
        warnings,
        link_patterns: LinkPatterns(link_patterns),
    })
}

pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
    toml::from_str(text).map_err(|e| ManifestError::new("plugin_manifest_parse_failed", e.to_string()))
}

fn valid_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b':' | b'_' | b'-'))
}

/// Local ids (actions, panes, link handlers) may not contain dots, because
/// they are qualified as `<plugin id>.<local id>`.
fn valid_local_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
}

/// Validates a parsed manifest, returning warnings for soft problems.
pub fn validate(m: &Manifest, host_version: &semver::Version) -> Result<Vec<String>, ManifestError> {
    validate_compiling(m, host_version).map(|(warnings, _)| warnings)
}

/// [`validate`], also returning the link handlers' compiled patterns.
fn validate_compiling(
    m: &Manifest,
    host_version: &semver::Version,
) -> Result<(Vec<String>, Vec<regex::Regex>), ManifestError> {
    let mut warnings = Vec::new();
    let mut link_patterns = Vec::with_capacity(m.link_handlers.len());
    if !valid_plugin_id(&m.id) {
        return Err(ManifestError::new(
            "invalid_plugin_id",
            format!(
                "plugin id {:?} must be 1-{MAX_ID_LEN} ASCII letters, digits or . : _ -",
                m.id
            ),
        ));
    }
    if m.name.trim().is_empty() {
        return Err(ManifestError::new("invalid_plugin_name", "name must not be empty"));
    }
    if semver::Version::parse(&m.version).is_err() {
        warnings.push(format!("version {:?} is not semver", m.version));
    }
    let min = semver::Version::parse(&m.min_tt_git_version).map_err(|e| {
        ManifestError::new(
            "invalid_plugin_min_tt_git_version",
            format!("min_tt_git_version {:?}: {e}", m.min_tt_git_version),
        )
    })?;
    if &min > host_version {
        return Err(ManifestError::new(
            "plugin_requires_newer_tt_git",
            format!("{} requires tt-git {min} or newer (this is {host_version})", m.id),
        ));
    }

    let check_platforms = |p: &Option<Vec<String>>, warnings: &mut Vec<String>| -> Result<(), ManifestError> {
        if let Some(list) = p {
            for name in list {
                if !KNOWN_PLATFORMS.contains(&name.as_str()) {
                    return Err(ManifestError::new(
                        "invalid_plugin_platform",
                        format!("unknown platform {name:?} (expected linux, macos or windows)"),
                    ));
                }
            }
            if list.is_empty() {
                warnings.push("empty platforms list: nothing will run".into());
            }
        }
        Ok(())
    };
    match &m.platforms {
        None => warnings.push("no platforms listed; assuming all".into()),
        p => check_platforms(p, &mut warnings)?,
    }

    let check_command = |what: &str, cmd: &[String]| -> Result<(), ManifestError> {
        if cmd.is_empty() || cmd[0].trim().is_empty() {
            return Err(ManifestError::new(
                "invalid_plugin_command",
                format!("{what}: command must be a non-empty argv array"),
            ));
        }
        Ok(())
    };
    for (i, b) in m.build.iter().enumerate() {
        check_command(&format!("build[{i}]"), &b.command)?;
        check_platforms(&b.platforms, &mut warnings)?;
    }
    for (i, s) in m.startup.iter().enumerate() {
        check_command(&format!("startup[{i}]"), &s.command)?;
        check_platforms(&s.platforms, &mut warnings)?;
    }

    let mut ids = HashSet::new();
    for a in &m.actions {
        if !valid_local_id(&a.id) {
            return Err(ManifestError::new(
                "invalid_plugin_action_id",
                format!("action id {:?} must be ASCII letters, digits or : _ - (no dots)", a.id),
            ));
        }
        if !ids.insert(a.id.as_str()) {
            return Err(ManifestError::new(
                "duplicate_plugin_action_id",
                format!("duplicate action id {:?}", a.id),
            ));
        }
        check_command(&format!("action {}", a.id), &a.command)?;
        check_platforms(&a.platforms, &mut warnings)?;
    }

    for (i, e) in m.events.iter().enumerate() {
        check_command(&format!("events[{i}]"), &e.command)?;
        check_platforms(&e.platforms, &mut warnings)?;
        if !HOOK_EVENTS.contains(&e.on.as_str()) {
            warnings.push(format!("unknown event {:?}", e.on));
        }
    }

    let mut pane_ids = HashSet::new();
    for p in &m.panes {
        if !valid_local_id(&p.id) {
            return Err(ManifestError::new(
                "invalid_plugin_pane_id",
                format!("pane id {:?} must be ASCII letters, digits or : _ - (no dots)", p.id),
            ));
        }
        if !pane_ids.insert(p.id.as_str()) {
            return Err(ManifestError::new(
                "duplicate_plugin_pane_id",
                format!("duplicate pane id {:?}", p.id),
            ));
        }
        check_command(&format!("pane {}", p.id), &p.command)?;
        check_platforms(&p.platforms, &mut warnings)?;
        for d in p.width.iter().chain(p.height.iter()) {
            if !d.is_valid() {
                return Err(ManifestError::new(
                    "invalid_plugin_pane_size",
                    format!(
                        "pane {}: size {d:?} must be a positive cell count or \"1%\"-\"100%\"",
                        p.id
                    ),
                ));
            }
        }
        if p.placement != Placement::Popup && (p.width.is_some() || p.height.is_some()) {
            warnings.push(format!("pane {}: width/height only apply to popup placement", p.id));
        }
    }

    let mut link_ids = HashSet::new();
    for l in &m.link_handlers {
        if !valid_local_id(&l.id) {
            return Err(ManifestError::new(
                "invalid_plugin_link_handler_id",
                format!("link handler id {:?} must be ASCII letters, digits or : _ -", l.id),
            ));
        }
        if !link_ids.insert(l.id.as_str()) {
            return Err(ManifestError::new(
                "duplicate_plugin_link_handler_id",
                format!("duplicate link handler id {:?}", l.id),
            ));
        }
        match regex::Regex::new(&l.pattern) {
            Ok(re) => link_patterns.push(re),
            Err(e) => {
                return Err(ManifestError::new(
                    "invalid_plugin_link_pattern",
                    format!("link handler {}: {e}", l.id),
                ));
            }
        }
        if !ids.contains(l.action.as_str()) {
            return Err(ManifestError::new(
                "unknown_plugin_link_action",
                format!("link handler {} names unknown action {:?}", l.id, l.action),
            ));
        }
    }
    Ok((warnings, link_patterns))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> semver::Version {
        semver::Version::new(0, 5, 0)
    }

    const BASE: &str = r#"
id = "acme.tools"
name = "Acme"
version = "0.1.0"
min_tt_git_version = "0.1.0"
platforms = ["linux", "macos", "windows"]
"#;

    fn check(extra: &str) -> Result<Vec<String>, ManifestError> {
        let m = parse(&format!("{BASE}{extra}"))?;
        validate(&m, &host())
    }

    fn code(extra: &str) -> &'static str {
        check(extra).unwrap_err().code
    }

    #[test]
    fn full_manifest_is_valid() {
        let warnings = check(
            r#"
[[build]]
command = ["cargo", "build", "--release"]

[[startup]]
command = ["./restore"]

[[actions]]
id = "open-commit"
title = "Open commit"
contexts = ["commit", "global"]
command = ["node", "open.js"]

[[events]]
on = "push.completed"
command = ["./notify"]

[[panes]]
id = "board"
title = "Board"
placement = "popup"
width = "80%"
height = 20
command = ["board"]

[[link_handlers]]
id = "issue"
title = "Open issue"
pattern = '^https://github\.com/.+/issues/\d+$'
action = "open-commit"
"#,
        )
        .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn rejections() {
        let cases: &[(&str, &str)] = &[
            (
                "[[actions]]\nid = \"a.b\"\ntitle = \"x\"\ncommand = [\"x\"]\n",
                "invalid_plugin_action_id",
            ),
            (
                "[[actions]]\nid = \"a\"\ntitle = \"x\"\ncommand = [\"x\"]\n[[actions]]\nid = \"a\"\ntitle = \"y\"\ncommand = [\"y\"]\n",
                "duplicate_plugin_action_id",
            ),
            (
                "[[actions]]\nid = \"a\"\ntitle = \"x\"\ncommand = []\n",
                "invalid_plugin_command",
            ),
            (
                "[[panes]]\nid = \"p\"\ntitle = \"x\"\ncommand = [\"x\"]\n[[panes]]\nid = \"p\"\ntitle = \"x\"\ncommand = [\"x\"]\n",
                "duplicate_plugin_pane_id",
            ),
            (
                "[[panes]]\nid = \"p\"\ntitle = \"x\"\nplacement = \"popup\"\nwidth = \"150%\"\ncommand = [\"x\"]\n",
                "invalid_plugin_pane_size",
            ),
            (
                "[[link_handlers]]\nid = \"l\"\ntitle = \"x\"\npattern = \".*\"\naction = \"missing\"\n",
                "unknown_plugin_link_action",
            ),
            (
                "[[actions]]\nid = \"a\"\ntitle = \"x\"\ncommand = [\"x\"]\n[[link_handlers]]\nid = \"l\"\ntitle = \"x\"\npattern = \"(\"\naction = \"a\"\n",
                "invalid_plugin_link_pattern",
            ),
            (
                "[[startup]]\ncommand = [\"x\"]\nplatforms = [\"beos\"]\n",
                "invalid_plugin_platform",
            ),
            ("bogus = 1\n", "plugin_manifest_parse_failed"),
        ];
        for (extra, want) in cases {
            assert_eq!(code(extra), *want, "for:\n{extra}");
        }
    }

    #[test]
    fn id_and_version_rules() {
        let mk = |id: &str, min: &str| {
            format!(
                "id = \"{id}\"\nname = \"n\"\nversion = \"1.0.0\"\nmin_tt_git_version = \"{min}\"\nplatforms = []\n"
            )
        };
        let v = |s: String| validate(&parse(&s).unwrap(), &host());
        assert_eq!(v(mk("has space", "0.1.0")).unwrap_err().code, "invalid_plugin_id");
        assert_eq!(v(mk(&"x".repeat(121), "0.1.0")).unwrap_err().code, "invalid_plugin_id");
        assert!(v(mk("ok.id:with-chars_1", "0.1.0")).is_ok());
        assert_eq!(
            v(mk("a", "not-semver")).unwrap_err().code,
            "invalid_plugin_min_tt_git_version"
        );
        assert_eq!(v(mk("a", "9.0.0")).unwrap_err().code, "plugin_requires_newer_tt_git");
        assert!(v(mk("a", "0.5.0")).is_ok(), "equal version is fine");
    }

    #[test]
    fn soft_problems_are_warnings() {
        let m = parse(
            "id = \"a\"\nname = \"n\"\nversion = \"one\"\nmin_tt_git_version = \"0.1.0\"\n\
             [[events]]\non = \"push.complete\"\ncommand = [\"x\"]\n\
             [[panes]]\nid = \"p\"\ntitle = \"t\"\nwidth = 10\ncommand = [\"x\"]\n",
        )
        .unwrap();
        let w = validate(&m, &host()).unwrap();
        assert!(w.iter().any(|x| x.contains("not semver")), "{w:?}");
        assert!(w.iter().any(|x| x.contains("no platforms")), "{w:?}");
        assert!(w.iter().any(|x| x.contains("unknown event \"push.complete\"")), "{w:?}");
        assert!(w.iter().any(|x| x.contains("only apply to popup")), "{w:?}");
    }

    #[test]
    fn dimensions_and_platforms() {
        assert_eq!(Dimension::Percent("50%".into()).resolve(80), 40);
        assert_eq!(Dimension::Cells(200).resolve(80), 80);
        assert!(platform_matches(&None, "windows"));
        assert!(platform_matches(&Some(vec!["windows".into()]), "windows"));
        assert!(!platform_matches(&Some(vec!["linux".into()]), "windows"));
    }

    #[test]
    fn action_contexts_default_to_global() {
        let a = ActionSpec {
            id: "a".into(),
            title: "t".into(),
            description: None,
            contexts: vec![],
            command: vec!["x".into()],
            platforms: None,
        };
        assert_eq!(a.effective_contexts(), vec![ActionContext::Global]);
    }
}
