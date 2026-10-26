//! The Win32 and COM calls behind tt-git-gui's Windows integration, and the
//! Objective-C ones behind its macOS integration, behind safe functions.
//!
//! The workspace forbids `unsafe`; this crate is the only one that allows
//! it, so every unsafe block in tt-git lives here, each with a `// SAFETY:`
//! comment. Nothing here knows about GPUI: windows come in as their raw
//! `HWND` value. Each module is built only on its OS, so callers gate their
//! use with `#[cfg(windows)]` or `#[cfg(target_os = "macos")]`.

#[cfg(windows)]
pub mod jump_list;
#[cfg(target_os = "macos")]
pub mod quick_look;
#[cfg(windows)]
pub mod taskbar;
