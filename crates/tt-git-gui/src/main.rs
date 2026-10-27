// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

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
    /// Repositories to open, each in its own window (defaults to the
    /// current directory).
    paths: Vec<PathBuf>,

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

    /// Open the start window rather than a repository (how an update
    /// restarts a copy that had no repository open).
    #[arg(long, hide = true)]
    start_window: bool,

    /// Show the folder picker and open the repositories picked (the jump
    /// list's Open repository… task).
    #[arg(long, conflicts_with = "paths")]
    open_repository: bool,

    /// Run as the update's waiter on this instances folder: wait for every
    /// instance to quit, then start them again.
    #[arg(long, hide = true, value_name = "DIR")]
    finish_update: Option<PathBuf>,

    /// With --finish-update: the installer to run before starting them.
    #[arg(long, hide = true, value_name = "FILE", requires = "finish_update")]
    installer: Option<PathBuf>,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let args = Cli::parse();
    let _log_guard = init_tracing();
    if let Some(dir) = &args.finish_update {
        return finish_update(dir, args.installer.as_deref());
    }

    let config_path = args.config.unwrap_or_else(tt_git_core::paths::config_file);
    let global = GlobalConfig::load(&config_path)?;
    // One zoom and font for every window, from the global file alone.
    let ui = global.config()?.ui;
    let zoom = ui.zoom();
    let style = ui.style;
    let font = tt_git_gui::font::UiFont {
        family: ui.font().map(|f| f.to_string().into()),
        size: ui.font_size(),
    };
    let filter_to_cwd = if args.filter_to_cwd {
        Some(true)
    } else if args.no_filter_to_cwd {
        Some(false)
    } else {
        None
    };
    let mut paths = args.paths.into_iter();
    let named = paths.len() > 0;
    // The rest open in windows of their own once the first is up.
    let first_path = paths.next();
    let more: Vec<PathBuf> = paths.collect();
    // Outside a repository, the start window asks which to open (saying
    // why, when the folder was named on the command line).
    let mut not_a_repo = None;
    let open_repository = args.open_repository;
    let repo = if args.start_window || open_repository {
        None
    } else {
        let repo_path = match first_path {
            Some(p) => p,
            None => std::env::current_dir().wrap_err("reading current directory")?,
        };
        match Repo::discover(&repo_path, &GitCli::new(&repo_path)) {
            Ok(repo) => Some(repo),
            Err(e @ GitError::NotARepo(_)) => {
                tracing::info!("{e}: showing the start window");
                not_a_repo = named.then(|| e.to_string());
                None
            }
            Err(e) => return Err(e).wrap_err("opening repository"),
        }
    };
    let check_for_updates = global.config()?.ui.check_for_updates;
    let no_plugins = args.no_plugins;
    // Plugins call `$TTGIT_BIN_PATH <subcommand>`; the GUI has none, so
    // they get the TUI's `tt-git`.
    let bin_path = if no_plugins {
        None
    } else {
        tt_git_gui::tt_git_bin::find()
    };
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
                bin_path: bin_path.clone(),
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

    gpui::application().with_assets(tt_git_gui::Assets).run(move |cx| {
        tt_git_gui::init(cx);
        tt_git_gui::app_id::set(cx);
        tt_git_gui::notify::install_portal(cx);
        tt_git_gui::notify::install_icon(&tt_git_core::paths::state_dir());
        tt_git_gui::zoom::set(zoom, cx);
        tt_git_gui::font::set(font, cx);
        tt_git_gui::look::set(style, cx);
        tt_git_gui::window_state::install(tt_git_gui::window_state::default_path(), cx);
        tt_git_gui::dock::install(cx);
        tt_git_gui::jump_list::install(cx);
        tt_git_gui::launcher_entry::install(cx);
        tt_git_gui::quick_look::install(cx);
        tt_git_gui::set_launcher(Rc::new(new_app), cx);
        tt_git_gui::windows::quit_when_last_closes(cx);
        tt_git_gui::services::install(cx);
        start_updates(check_for_updates, cx);
        let opened = match first {
            Some((app, rx)) => tt_git_gui::open_window(app, rx, cx).map(|_| ()),
            None => tt_git_gui::start::open(not_a_repo, cx).map(|start| {
                if open_repository {
                    tt_git_gui::start::pick(start, cx);
                }
            }),
        };
        if let Err(e) = opened {
            eprintln!("tt-git-gui: couldn't open a window: {e}");
            cx.quit();
        }
        for path in &more {
            if let Err(e) = tt_git_gui::windows::open_path(path, cx) {
                tracing::warn!("{e}");
            }
        }
        cx.activate(true);
    });
    tracing::info!("quit");
    Ok(())
}

/// Sets up updating: the real updater, this process among the running
/// instances (so an update restarts it), and the daily check unless
/// `[ui] check_for_updates` is off. Clears out what an earlier update set
/// aside.
fn start_updates(check: bool, cx: &mut gpui::App) {
    use tt_git_gui::update::{self, Releases, install, instances};

    let state = tt_git_core::paths::state_dir();
    let installed = Releases::detect();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        install::remove_leftovers(dir);
        // The old bundle sits next to tt-git.app.
        if let Ok(install::Install::AppBundle { app }) = &installed
            && let Some(apps) = app.parent()
        {
            install::remove_leftovers(apps);
        }
    }
    update::set_updater(
        Arc::new(Releases {
            work: state.join("gui-update"),
        }),
        cx,
    );
    let launch = match &installed {
        Ok(install) => install.launch(install::Os::CURRENT),
        Err(_) => std::env::current_exe().unwrap_or_default(),
    };
    match instances::Instance::register(&state.join(instances::DIR), launch) {
        Ok(instance) => update::join(instance, cx),
        Err(e) => tracing::warn!("couldn't register with the other instances: {e}"),
    }
    if check {
        update::check_in_background(state.join(update::CHECK_FILE), cx);
    }
}

/// `--finish-update`: the waiter an update starts.
fn finish_update(dir: &std::path::Path, installer: Option<&std::path::Path>) -> Result<()> {
    use tt_git_gui::update::instances;

    tracing::info!("waiting for tt-git to quit, to restart it");
    match instances::finish(dir, installer, &mut instances::System, instances::WAIT) {
        Ok(done) => tracing::info!("restarted {} instances", done.started),
        Err(e) => tracing::error!("finishing the update: {e}"),
    }
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
