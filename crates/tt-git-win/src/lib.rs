//! The OS calls behind tt-git-gui's Windows and macOS integration, behind
//! safe functions: Win32 and COM on Windows, AppKit through `objc2` on
//! macOS.
//!
//! The workspace forbids `unsafe`; this crate is the only one that allows
//! it, so every unsafe block in tt-git lives here, each with a `// SAFETY:`
//! comment. Nothing here knows about GPUI: windows come in as their raw
//! `HWND` value. Each module is built only on its OS, so callers gate their
//! use with the same `cfg`.

#[cfg(windows)]
pub mod jump_list;
#[cfg(target_os = "macos")]
pub mod services;
#[cfg(windows)]
pub mod taskbar;
