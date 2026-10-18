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
    let (mut app, mut rx) = App::new(config, repo, tt_git_core::time::system_clock())?;
    if !args.no_plugins {
        app.enable_plugins(PluginOptions {
            dirs: PluginDirs::from_env(),
            bin_path: std::env::current_exe().ok(),
            socket: true,
            extra_env: Vec::new(),
        });
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .wrap_err("starting tokio runtime")?;

    // Installed after color-eyre so ratatui's panic hook restores the
    // terminal before the report is printed.
    let mut terminal = ratatui::init();
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
/// terminal because the UI owns it.
fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::EnvFilter;

    let dir = tt_git_core::paths::state_dir().join("logs");
    std::fs::create_dir_all(&dir).ok()?;
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
