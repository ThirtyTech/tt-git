//! tt-git's app: its state, what each key and git operation does, and the
//! background jobs, without drawing or terminal input. Each front end (the
//! TUI in `tt-git-tui`) draws the state and feeds the app its input.

pub mod action;
pub mod agent;
pub mod app;
pub mod component;
pub mod config;
pub mod editor;
pub mod filter;
pub mod geom;
pub mod jobs;
pub mod keymap;
pub mod layout;
pub mod loaders;
pub mod modal;
pub mod mouse;
pub mod new_commits;
pub mod panes;
pub mod pty_pane;
pub mod search;
pub mod style;
pub mod syntax;
pub mod theme;
pub mod themes;
pub mod view;
pub mod watcher;
pub mod widgets;

/// Stack size for threads that only wait on or move bytes between pipes,
/// files and channels (no parsing or deep calls), instead of the 2 MiB
/// default.
pub const IO_THREAD_STACK: usize = 256 * 1024;

pub use action::Action;
pub use app::App;
pub use config::Config;
