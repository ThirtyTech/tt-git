//! tt-git's desktop app: draws the app from `tt-git-app` with GPUI and
//! feeds it the window's input, as `tt-git-tui` does for a terminal.
//!
//! - [`model::AppModel`] owns the app and runs it.
//! - [`shell::Shell`] is the window: the layout, the help bar and what
//!   opens over the panes, and the keys.
//! - [`panes`], [`dialogs`] and [`terminal`] draw each area, one file each.
//! - [`frame::PaneFrame`], [`rows::StyledRows`], [`text::StyledLine`],
//!   [`theme::GuiTheme`], [`font::MonoFont`] and [`ui::Ui`] are what they
//!   draw with.
//!
//! `docs/gui.md` says how to port an area.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::component::Root;
use gpui::{
    App, AppContext, Bounds, Global, KeyBinding, NoAction, TitlebarOptions, WindowBounds, WindowHandle, WindowOptions,
    px, size,
};
use tokio::sync::mpsc::UnboundedReceiver;
use tt_git_app::Action;
use tt_git_core::Repo;

pub mod dialogs;
pub mod font;
pub mod frame;
pub mod gpui_tokio;
pub mod help_bar;
pub mod keys;
pub mod layout;
pub mod model;
pub mod overlay;
pub mod panes;
pub mod rows;
pub mod shell;
pub mod terminal;
#[cfg(test)]
pub mod testing;
pub mod text;
pub mod theme;
pub mod ui;

/// The key context of the window's content. tt-git's keymap resolves every
/// key; GPUI key bindings that would take one first are switched off in it.
pub const CONTEXT: &str = "TtGit";

/// Keys gpui-component binds around the window's content (its `Root`
/// moves focus with tab and copies with ctrl+c), which tt-git's keymap
/// uses itself.
const TAKEN_KEYS: &[&str] = &["tab", "shift-tab", "ctrl-c"];

/// Sets GPUI and gpui-component up, and starts the tokio runtime the
/// app's jobs run on. Call once, before [`open_window`].
pub fn init(cx: &mut App) {
    gpui::init(cx);
    gpui_tokio::init(cx);
    cx.bind_keys(
        TAKEN_KEYS
            .iter()
            .map(|key| KeyBinding::new(key, NoAction, Some(CONTEXT))),
    );
}

/// Opens a window on `app`, which [`App::new`](tt_git_app::App::new) made
/// along with `rx` and which is set up (themes, plugins) but not started.
pub fn open_window(
    app: tt_git_app::App,
    rx: UnboundedReceiver<Action>,
    cx: &mut App,
) -> gpui::Result<WindowHandle<Root>> {
    let title = app.terminal_title();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1280.), px(800.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        app_id: Some("tt-git".into()),
        ..Default::default()
    };
    let font = font::MonoFont::new(cx);
    cx.open_window(options, |window, cx| {
        let model = cx.new(|cx| model::AppModel::new(app, rx, cx));
        let shell = cx.new(|cx| shell::Shell::new(model, font, window, cx));
        cx.new(|cx| Root::new(shell, window, cx))
    })
}

/// Makes the app for a repository: what `tt-git-gui`'s `main` does at
/// startup, kept for when the app asks to open another worktree.
pub type NewApp = dyn Fn(Repo) -> Result<(tt_git_app::App, UnboundedReceiver<Action>), String>;

struct Launcher(Rc<NewApp>);

impl Global for Launcher {}

/// Remembers how to make the app for a repository, so opening another
/// worktree opens it in a new window, as if tt-git had been started there.
pub fn set_launcher(new_app: Rc<NewApp>, cx: &mut App) {
    cx.set_global(Launcher(new_app));
}

/// Opens `repo` in a new window, with the launcher from [`set_launcher`].
/// Called before the old window closes, so the app doesn't quit between.
fn reopen(repo: Repo, cx: &mut App) {
    let Some(launcher) = cx.try_global::<Launcher>().map(|l| l.0.clone()) else {
        tracing::warn!("no launcher to open {}", repo.root().display());
        return;
    };
    // So plugins start where tt-git is, as the TUI does.
    if let Err(e) = std::env::set_current_dir(repo.root()) {
        tracing::warn!("couldn't change to {}: {e}", repo.root().display());
    }
    let root: PathBuf = repo.root().to_path_buf();
    match launcher(repo) {
        Ok((app, rx)) => {
            if let Err(e) = open_window(app, rx, cx) {
                tracing::error!("couldn't open {}: {e}", root.display());
            }
        }
        Err(e) => tracing::error!("couldn't open {}: {e}", root.display()),
    }
}
