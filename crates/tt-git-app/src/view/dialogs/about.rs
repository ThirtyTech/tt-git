//! What the About dialog (`Modal::About`) says in both front ends. Each
//! draws it itself and adds a line naming the toolkit it's built with and a
//! link to the other front end's install page.

/// The dialog's title.
pub const ABOUT_TITLE: &str = "About tt-git";

/// Under the version.
pub const ABOUT_TAGLINE: &str = "An extendable git client designed to answer questions for you and your agents.";

/// Who made it.
pub const ABOUT_DEVELOPER: &str = "Developer: Jonathan Sheely @ ThirtyTech Inc";

/// tt-git's source.
pub const REPO_URL: &str = "https://github.com/thirtytech/tt-git";

/// Installing the desktop app, for the terminal's About.
pub const GUI_INSTALL_URL: &str = "https://thirtytech.github.io/tt-git/desktop/install/";

/// Installing the terminal app, for the desktop app's About.
pub const TUI_INSTALL_URL: &str = "https://thirtytech.github.io/tt-git/getting-started/install/";

/// The keys that close it, as its last line shows them.
pub const ABOUT_HINT: &str = "esc close";

/// `tt-git <version>`, the dialog's heading.
pub fn about_heading() -> String {
    format!("tt-git {}", env!("CARGO_PKG_VERSION"))
}
