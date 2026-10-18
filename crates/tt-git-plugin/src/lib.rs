//! herdr-style plugin host for tt-git.
//!
//! Plugins are plain subprocesses described by a `tt-git-plugin.toml`
//! manifest. tt-git starts them for actions (keybindings, the command
//! palette, `plugin.action.invoke`), event hooks, startup and panes; they talk
//! back through the `tt-git` CLI (`$TTGIT_BIN_PATH`) or directly over the
//! NDJSON socket at `$TTGIT_SOCKET_PATH`. There is no SDK, sandbox or
//! permission model, by design: the CLI is the plugin API.

pub mod host;
pub mod install;
pub mod manifest;
pub mod registry;
pub mod runtime;
pub mod socket;

pub use host::{ActionInfo, HostError, PluginHost};
pub use manifest::{MANIFEST_FILE, Manifest};
pub use registry::{PluginDirs, PluginRecord, Registry};
pub use runtime::{CommandLog, CommandStatus, EventEnvelope, HostEnv, InvocationContext, Runner};

/// tt-git's version, compared against `min_tt_git_version`.
pub fn host_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is semver")
}
