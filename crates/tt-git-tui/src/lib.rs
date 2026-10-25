//! ratatui user interface for tt-git.

pub mod action;
pub mod agent;
pub mod app;
pub mod component;
pub mod config;
pub mod convert;
pub mod draw;
pub mod editor;
pub mod filter;
pub mod geom;
pub mod input;
pub mod jobs;
pub mod keymap;
pub mod layout;
pub mod loaders;
pub mod modal;
pub mod mouse;
pub mod panes;
pub mod pty_pane;
pub mod runner;
pub mod search;
pub mod style;
pub mod syntax;
pub mod theme;
pub mod themes;
pub mod tui_theme;
pub mod watcher;
pub mod widgets;

/// Stack size for threads that only wait on or move bytes between pipes,
/// files and channels (no parsing or deep calls), instead of the 2 MiB
/// default.
pub(crate) const IO_THREAD_STACK: usize = 256 * 1024;

pub use action::Action;
pub use app::App;
pub use config::Config;
pub use draw::Screen;
