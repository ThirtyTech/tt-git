//! The Win32 and COM calls behind tt-git-gui's Windows integration, behind
//! safe functions.
//!
//! The workspace forbids `unsafe`; this crate is the only one that allows
//! it, so every unsafe block in tt-git lives here, each with a `// SAFETY:`
//! comment. Nothing here knows about GPUI: windows come in as their raw
//! `HWND` value. Off Windows the crate is empty, so callers gate their use
//! with `#[cfg(windows)]`.

#![cfg(windows)]

pub mod jump_list;
pub mod taskbar;
