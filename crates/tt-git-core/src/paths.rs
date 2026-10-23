//! Per-user directories for config and state.
//!
//! Follows the same rule as herdr: `XDG_CONFIG_HOME`/`XDG_STATE_HOME` when set,
//! otherwise `~/.config/tt-git` on Unix (including macOS) and `%APPDATA%\tt-git`
//! on Windows. `TTGIT_CONFIG_DIR` / `TTGIT_STATE_DIR` override everything, which
//! is what tests use to stay hermetic.

use std::path::PathBuf;

use etcetera::BaseStrategy;

pub const APP_DIR: &str = "tt-git";

/// Directory holding `config.toml`, `plugins.json` and plugin config dirs.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = env_path("TTGIT_CONFIG_DIR") {
        return dir;
    }
    match etcetera::choose_base_strategy() {
        Ok(base) => base.config_dir().join(APP_DIR),
        Err(_) => PathBuf::from(".").join(APP_DIR),
    }
}

/// Directory holding logs and per-plugin state.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = env_path("TTGIT_STATE_DIR") {
        return dir;
    }
    match etcetera::choose_base_strategy() {
        Ok(base) => base.state_dir().unwrap_or_else(|| base.data_dir()).join(APP_DIR),
        Err(_) => PathBuf::from(".").join(APP_DIR),
    }
}

/// The user's home directory.
pub fn home_dir() -> Option<PathBuf> {
    etcetera::home_dir().ok()
}

/// Path of the user's main config file.
pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}
