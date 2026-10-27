//! The OS calls behind tt-git-gui's Windows and macOS integration, behind
//! safe functions: Win32 and COM on Windows, AppKit through `objc2` on
//! macOS. Also [`power`], which both apps use for background fetching, on
//! Linux too (over D-Bus, with no unsafe code).
//!
//! The workspace forbids `unsafe`; this crate is the only one that allows
//! it, so every unsafe block in tt-git lives here, each with a `// SAFETY:`
//! comment. Nothing here knows about GPUI: windows come in as their raw
//! `HWND` value. Each module is built only on its OS, so callers gate their
//! use with the same `cfg`.

#[cfg(windows)]
pub mod foreground;
#[cfg(windows)]
pub mod jump_list;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub mod power;
#[cfg(target_os = "macos")]
pub mod quick_look;
#[cfg(target_os = "macos")]
pub mod services;
#[cfg(windows)]
pub mod taskbar;
#[cfg(windows)]
pub mod toast;
