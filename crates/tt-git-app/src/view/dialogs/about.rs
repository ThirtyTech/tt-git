//! What the About dialog (`Modal::About`) says in both front ends. Each
//! draws it itself and adds a line naming the toolkit it's built with and a
//! pointer to the other front end.

/// The dialog's title.
pub const ABOUT_TITLE: &str = "About tt-git";

/// Under the version.
pub const ABOUT_TAGLINE: &str = "A git client with a commit graph, unified branches and plugins.";

/// Who made it.
pub const ABOUT_DEVELOPER: &str = "Developer: Jonathan Sheely @ ThirtyTech Inc";

/// tt-git's source.
pub const REPO_URL: &str = "https://github.com/thirtytech/tt-git";

/// The keys that close it, as its last line shows them.
pub const ABOUT_HINT: &str = "esc close";

/// `tt-git <version>`, the dialog's heading.
pub fn about_heading() -> String {
    format!("tt-git {}", env!("CARGO_PKG_VERSION"))
}
