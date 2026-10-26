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
//! - [`zoom`] is how big it all is, and [`look`] which style it's drawn in.
//!
//! `docs/gui.md` says how to port an area.

use gpui::component::Root;
use gpui::{App, AppContext, KeyBinding, NoAction, WindowHandle, WindowOptions};
use tokio::sync::mpsc::UnboundedReceiver;
use tt_git_app::Action;

pub mod app_id;
pub mod context_menu;
pub mod dialogs;
pub mod dock;
pub mod font;
pub mod frame;
pub mod gpui_tokio;
pub mod help_bar;
pub mod jump_list;
pub mod keys;
pub mod launcher_entry;
pub mod layout;
pub mod look;
pub mod menus;
pub mod model;
pub mod notify;
pub mod overlay;
pub mod panes;
pub mod quick_look;
pub mod rows;
pub mod services;
pub mod shell;
pub mod start;
pub mod taskbar;
pub mod terminal;
#[cfg(test)]
pub mod testing;
pub mod text;
pub mod theme;
pub mod tt_git_bin;
pub mod ui;
pub mod update;
pub mod window_state;
pub mod windows;
pub mod zoom;

pub use windows::{NewApp, set_launcher};

/// The icons gpui-component draws, the title bar's window controls among
/// them. `main` registers it; GPUI's test context has no asset source, so
/// windows in tests draw no icons.
pub use gpui::assets::Assets;

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
    menus::init(cx);
    notify::init(cx);
    dock::init(cx);
}

/// Opens a window on `app`, which [`App::new`](tt_git_app::App::new) made
/// along with `rx` and which is set up (themes, plugins) but not started.
/// It goes where the last window was ([`window_state`]), moved along if
/// other repository windows are open. [`windows::open_repo`] is the way in
/// for a repository that may have a window already.
pub fn open_window(
    app: tt_git_app::App,
    rx: UnboundedReceiver<Action>,
    cx: &mut App,
) -> gpui::Result<WindowHandle<Root>> {
    let title = app.terminal_title();
    let root = app.repo().root().to_path_buf();
    let open = windows::repo_windows(cx).len();
    let options = WindowOptions {
        window_bounds: Some(window_state::next_bounds(open, cx)),
        // C1: the app draws the title bar, with the menus (menus::bar).
        titlebar: Some(menus::bar::titlebar_options(title)),
        app_owns_titlebar_drag: true,
        icon: menus::icon::window_icon(),
        app_id: Some(app_id::APP_ID.into()),
        ..Default::default()
    };
    let font = font::MonoFont::new(cx);
    let handle = cx.open_window(options, |window, cx| {
        let model = cx.new(|cx| model::AppModel::new(app, rx, cx));
        let shell = cx.new(|cx| shell::Shell::new(model, font, window, cx));
        cx.new(|cx| Root::new(shell, window, cx))
    })?;
    windows::register(handle.into(), &root, cx);
    Ok(handle)
}
