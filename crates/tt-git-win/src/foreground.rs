//! Letting another process take the foreground.
//!
//! Windows only lets a process bring its window forward if it's the one the
//! user is using. A copy of tt-git-gui that hands its folders to the
//! running one is (Explorer or the taskbar just started it), so it passes
//! that on before the running copy raises its window.

use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;

/// Lets process `pid` bring its windows forward. False if Windows refused
/// (this process isn't allowed to either).
pub fn allow(pid: u32) -> bool {
    // SAFETY: AllowSetForegroundWindow takes a plain process id and touches
    // no memory of ours.
    unsafe { AllowSetForegroundWindow(pid) }.is_ok()
}
