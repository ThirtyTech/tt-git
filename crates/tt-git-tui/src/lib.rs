//! ratatui user interface for tt-git: draws the app from `tt-git-app` and
//! feeds it terminal input.

pub mod convert;
pub mod draw;
pub mod input;
pub mod runner;
pub mod tui_theme;

pub use tt_git_app::{
    Action, App, Config, action, agent, app, component, config, editor, filter, geom, jobs, keymap, layout, loaders,
    modal, mouse, panes, pty_pane, search, style, syntax, theme, themes, view, watcher, widgets,
};

pub use draw::Screen;
