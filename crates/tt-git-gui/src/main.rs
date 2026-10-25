// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::rc::Rc;

use clap::Parser;
use color_eyre::eyre::{Result, WrapErr};
use tt_git_app::App;
use tt_git_app::app::PluginOptions;
use tt_git_app::config::GlobalConfig;
use tt_git_app::themes::{ThemeDirs, ThemeOptions};
use tt_git_core::{GitCli, GitError, Repo};
use tt_git_plugin::registry::PluginDirs;

/// tt-git's desktop app: a git client with a commit graph, unified branches
/// and plugins.
#[derive(Debug, Parser)]
#[command(name = "tt-git-gui", version, about)]
struct Cli {
    /// Repository to open (defaults to the current directory).
    path: Option<PathBuf>,

    /// Use this config file instead of the default location.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Don't load plugins or open the API socket.
    #[arg(long)]
    no_plugins: bool,

    /// Started in a subfolder, narrow the graph and Status to it
    /// (overrides `graph.filter_to_cwd`).
    #[arg(long, overrides_with = "no_filter_to_cwd")]
    filter_to_cwd: bool,

    /// Start with the whole repository shown, even if `graph.filter_to_cwd`
    /// is set.
    #[arg(long, overrides_with = "filter_to_cwd")]
    no_filter_to_cwd: bool,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let args = Cli::parse();
    let _log_guard = init_tracing();

    let config_path = args.config.unwrap_or_else(tt_git_core::paths::config_file);
    let global = GlobalConfig::load(&config_path)?;
    let filter_to_cwd = if args.filter_to_cwd {
        Some(true)
    } else if args.no_filter_to_cwd {
        Some(false)
    } else {
        None
    };
    let named = args.path.is_some();
    let repo_path = match args.path {
        Some(p) => p,
        None => std::env::current_dir().wrap_err("reading current directory")?,
    };
    // Outside a repository, the start window asks which to open (saying
    // why, when the folder was named on the command line).
    let mut not_a_repo = None;
    let repo = match Repo::discover(&repo_path, &GitCli::new(&repo_path)) {
        Ok(repo) => Some(repo),
        Err(e @ GitError::NotARepo(_)) => {
            tracing::info!("{e}: showing the start window");
            not_a_repo = named.then(|| e.to_string());
            None
        }
        Err(e) => return Err(e).wrap_err("opening repository"),
    };
    let no_plugins = args.no_plugins;
    // Each repository, and each one opened later, gets its own files laid
    // over the global config.
    let new_app = move |repo: Repo| -> Result<_, String> {
        let mut config = global
            .for_repo(repo.root(), repo.common_dir())
            .map_err(|e| e.to_string())?;
        if let Some(on) = filter_to_cwd {
            config.graph.filter_to_cwd = on;
        }
        let (mut app, rx) = App::new(config, repo, tt_git_core::time::system_clock()).map_err(|e| e.to_string())?;
        app.enable_themes(ThemeOptions {
            dirs: ThemeDirs::from_env(),
            config_path: Some(config_path.clone()),
        });
        if !no_plugins {
            app.enable_plugins(PluginOptions {
                dirs: PluginDirs::from_env(),
                bin_path: None,
                socket: true,
                extra_env: Vec::new(),
            });
        }
        Ok((app, rx))
    };
    // Made before any window opens, so a bad config is reported on the
    // console.
    let first = match repo {
        Some(repo) => Some(new_app(repo).map_err(color_eyre::eyre::Report::msg)?),
        None => None,
    };

    gpui::application().run(move |cx| {
        tt_git_gui::init(cx);
        tt_git_gui::window_state::install(tt_git_gui::window_state::default_path(), cx);
        tt_git_gui::set_launcher(Rc::new(new_app), cx);
        tt_git_gui::windows::quit_when_last_closes(cx);
        let opened = match first {
            Some((app, rx)) => tt_git_gui::open_window(app, rx, cx).map(|_| ()),
            None => tt_git_gui::start::open(not_a_repo, cx).map(|_| ()),
        };
        if let Err(e) = opened {
            eprintln!("tt-git-gui: couldn't open a window: {e}");
            cx.quit();
        }
        cx.activate(true);
    });
    Ok(())
}

/// Logs to `<state_dir>/logs/tt-git-gui.log`; the level comes from
/// `TTGIT_LOG` (an `EnvFilter` string, default `info`).
fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::EnvFilter;

    let dir = tt_git_core::paths::state_dir().join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let appender = tracing_appender::rolling::never(dir, "tt-git-gui.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("TTGIT_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .init();
    Some(guard)
}
