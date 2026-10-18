//! Writing throwaway plugins for tests.
//!
//! Commands run the hidden `tt-git __test-plugin <steps…>` subcommand (see
//! `crates/tt-git/src/test_plugin.rs`), so fixtures behave the same on every
//! OS. Pass the path of a built `tt-git` binary (`env!("CARGO_BIN_EXE_tt-git")`
//! in the `tt-git` crate's tests).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use tt_git_plugin::registry::PluginDirs;

use crate::Sandbox;

impl Sandbox {
    /// Plugin registry and per-plugin dirs inside the sandbox (matching the
    /// `TTGIT_CONFIG_DIR` / `TTGIT_STATE_DIR` in [`Sandbox::env`]).
    pub fn plugin_dirs(&self) -> PluginDirs {
        PluginDirs {
            config_dir: self.root().join("tt-config"),
            state_dir: self.root().join("tt-state"),
        }
    }
}

/// Builds a `tt-git-plugin.toml` whose commands run the test plugin.
pub struct PluginFixture {
    dir: PathBuf,
    bin: PathBuf,
    toml: String,
}

fn toml_str(s: &str) -> String {
    // TOML basic strings: escape backslashes (Windows paths) and quotes.
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl PluginFixture {
    /// A plugin `id` under `<sandbox>/plugin-src/<id>`, run via `bin`.
    pub fn new(sb: &Sandbox, id: &str, bin: &Path) -> Self {
        let dir = sb.root().join("plugin-src").join(id.replace([':', '.'], "_"));
        let toml = format!(
            "id = {}\nname = \"Test {id}\"\nversion = \"0.1.0\"\nmin_tt_git_version = \"0.1.0\"\nplatforms = [\"linux\", \"macos\", \"windows\"]\n",
            toml_str(id)
        );
        Self {
            dir,
            bin: bin.to_path_buf(),
            toml,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn argv(&self, steps: &[&str]) -> String {
        let mut parts = vec![toml_str(&self.bin.to_string_lossy()), toml_str("__test-plugin")];
        parts.extend(steps.iter().map(|s| toml_str(s)));
        format!("[{}]", parts.join(", "))
    }

    pub fn action(mut self, id: &str, title: &str, contexts: &[&str], steps: &[&str]) -> Self {
        let contexts = contexts.iter().map(|c| toml_str(c)).collect::<Vec<_>>().join(", ");
        let _ = write!(
            self.toml,
            "\n[[actions]]\nid = {}\ntitle = {}\ncontexts = [{contexts}]\ncommand = {}\n",
            toml_str(id),
            toml_str(title),
            self.argv(steps)
        );
        self
    }

    pub fn event(mut self, on: &str, steps: &[&str]) -> Self {
        let _ = write!(
            self.toml,
            "\n[[events]]\non = {}\ncommand = {}\n",
            toml_str(on),
            self.argv(steps)
        );
        self
    }

    pub fn startup(mut self, steps: &[&str]) -> Self {
        let _ = write!(self.toml, "\n[[startup]]\ncommand = {}\n", self.argv(steps));
        self
    }

    pub fn pane(mut self, id: &str, title: &str, placement: &str, steps: &[&str]) -> Self {
        let _ = write!(
            self.toml,
            "\n[[panes]]\nid = {}\ntitle = {}\nplacement = {}\ncommand = {}\n",
            toml_str(id),
            toml_str(title),
            toml_str(placement),
            self.argv(steps)
        );
        self
    }

    /// Appends raw TOML (for fields the builder doesn't cover).
    pub fn raw(mut self, toml: &str) -> Self {
        self.toml.push('\n');
        self.toml.push_str(toml);
        self
    }

    /// Writes the manifest and returns the plugin directory.
    pub fn write(self) -> PathBuf {
        std::fs::create_dir_all(&self.dir).expect("create plugin dir");
        std::fs::write(self.dir.join(tt_git_plugin::MANIFEST_FILE), &self.toml).expect("write manifest");
        self.dir
    }

    /// Writes the manifest and links it (enabled) into the sandbox registry.
    pub fn install(self, sb: &Sandbox) -> PathBuf {
        let dir = self.write();
        tt_git_plugin::registry::link(&sb.plugin_dirs(), &dir, true, &tt_git_plugin::host_version())
            .expect("link test plugin");
        dir
    }
}

/// Reads the JSON lines a `record` step wrote.
pub fn read_records(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
