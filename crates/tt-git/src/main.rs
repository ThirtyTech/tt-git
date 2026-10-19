use std::path::PathBuf;

use clap::Parser;
use color_eyre::eyre::{Result, WrapErr};
use tt_git_core::{GitCli, GitError, Repo};
use tt_git_plugin::registry::PluginDirs;
use tt_git_tui::app::PluginOptions;
use tt_git_tui::{App, Config};

mod cli;
mod test_plugin;

/// A terminal git client with a commit graph, unified branches and plugins.
#[derive(Debug, Parser)]
#[command(name = "tt-git", version, about, args_conflicts_with_subcommands = true)]
struct Cli {
    /// Repository to open (defaults to the current directory).
    path: Option<PathBuf>,

    /// Use this config file instead of the default location.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Don't load plugins or open the API socket.
    #[arg(long)]
    no_plugins: bool,

    #[command(subcommand)]
    command: Option<cli::Cmd>,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let args = Cli::parse();
    if let Some(cmd) = args.command {
        return cli::run(cmd);
    }
    let _log_guard = init_tracing();

    let config_path = args.config.unwrap_or_else(tt_git_core::paths::config_file);
    let config = Config::load(&config_path)?;
    let repo_path = match args.path {
        Some(p) => p,
        None => std::env::current_dir().wrap_err("reading current directory")?,
    };
    let repo = match Repo::discover(&repo_path, &GitCli::new(&repo_path)) {
        Ok(repo) => repo,
        Err(e @ GitError::NotARepo(_)) => {
            eprintln!("tt-git: {e}");
            std::process::exit(1);
        }
        Err(e) => return Err(e).wrap_err("opening repository"),
    };
    let title = format!("{} - tt-git", repo.name());
    let (mut app, mut rx) = App::new(config, repo, tt_git_core::time::system_clock())?;
    if !args.no_plugins {
        app.enable_plugins(PluginOptions {
            dirs: PluginDirs::from_env(),
            bin_path: std::env::current_exe().ok(),
            socket: true,
            extra_env: Vec::new(),
        });
    }

    // The UI loop is the only task; git work runs on the blocking pool
    // (`spawn_blocking`), so worker threads would only sit idle. Only the
    // timer driver is needed (the tick); terminal input has its own thread.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .wrap_err("starting tokio runtime")?;

    // Installed after color-eyre so ratatui's panic hook restores the
    // terminal before the report is printed.
    let mut terminal = ratatui::init();
    // Like lazygit's "<repo> - Lazygit": lets terminal multiplexers and their
    // plugins tell tt-git is running when the OS only reports the shell.
    let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::SetTitle(&title));
    let mouse = app.wants_mouse();
    if mouse {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
    }
    let result = runtime.block_on(tt_git_tui::runner::run(&mut terminal, &mut app, &mut rx));
    if mouse {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    }
    ratatui::restore();
    result.wrap_err("running the UI")
}

/// Logs to `<state_dir>/logs/tt-git.log`; the level comes from `TTGIT_LOG`
/// (an `EnvFilter` string, default `info`). Logging never goes to the
/// terminal because the UI owns it. A log over [`LOG_ROTATE_BYTES`] is
/// first moved to `tt-git.log.1`.
fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::EnvFilter;

    let dir = tt_git_core::paths::state_dir().join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    rotate_log(&dir.join("tt-git.log"), LOG_ROTATE_BYTES);
    let appender = tracing_appender::rolling::never(dir, "tt-git.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("TTGIT_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .init();
    Some(guard)
}

/// Every instance appends to the same log, so it's cut down at startup.
const LOG_ROTATE_BYTES: u64 = 10 * 1024 * 1024;

/// Moves `log` to `<log>.1` (replacing an older one) when it's over `max`
/// bytes. Failing is fine: another instance may hold either file open.
fn rotate_log(log: &std::path::Path, max: u64) {
    if std::fs::metadata(log).is_ok_and(|m| m.len() > max) {
        let mut old = log.as_os_str().to_owned();
        old.push(".1");
        let _ = std::fs::rename(log, old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_big_log_moves_aside_replacing_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("tt-git.log");
        let old = dir.path().join("tt-git.log.1");

        rotate_log(&log, 4); // none yet
        assert!(!old.exists());
        std::fs::write(&log, "1234").unwrap();
        rotate_log(&log, 4);
        assert!(log.exists() && !old.exists(), "at the limit stays");

        std::fs::write(&old, "older").unwrap();
        std::fs::write(&log, "12345").unwrap();
        rotate_log(&log, 4);
        assert!(!log.exists());
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "12345");
    }
}
