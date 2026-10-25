//! Test helpers for tt-git.
//!
//! - [`Sandbox`]: hermetic throwaway repos (fake `$HOME`, fixed author and
//!   dates, optional bare remotes).
//! - [`TestDriver`]: runs the real `App` in-process on a `TestBackend` and
//!   waits for idle after every key.
//! - [`mock::Mock`]: the mock project the docs screenshots show.

mod driver;
pub mod mock;
mod plugin;
mod sandbox;

pub use driver::{NOW, TestDriver};
pub use plugin::{PluginFixture, read_records};
pub use sandbox::{BASE_TIME, Sandbox};
