//! Whether the machine is on battery, saving power or on a metered
//! connection, for background fetching to back off. One file per OS, each
//! with a `state()`; whatever an OS can't say counts as `false`.

#[cfg(target_os = "linux")]
#[path = "power/linux.rs"]
mod os;
#[cfg(target_os = "macos")]
#[path = "power/macos.rs"]
mod os;
#[cfg(windows)]
#[path = "power/windows.rs"]
mod os;

/// The power and network conditions now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PowerState {
    /// Running on battery rather than mains power.
    pub on_battery: bool,
    /// Battery saver (Windows), Low Power Mode (macOS) or the power-saver
    /// profile (Linux) is on.
    pub saving_power: bool,
    /// The connection is metered or otherwise costly: a metered network
    /// (Windows, NetworkManager), roaming or over its data limit (Windows),
    /// or expensive or in Low Data Mode (macOS).
    pub metered: bool,
}

/// Asks the OS. It may take a moment (a D-Bus round trip on Linux), so call
/// it off the UI thread.
pub fn state() -> PowerState {
    os::state()
}
