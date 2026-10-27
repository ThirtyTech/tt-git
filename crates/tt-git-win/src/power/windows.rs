//! `GetSystemPowerStatus` for the battery and battery saver, and the
//! internet connection profile's cost (WinRT `NetworkInformation`) for a
//! metered connection.

use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use super::PowerState;

/// `SYSTEM_POWER_STATUS::ACLineStatus` on battery (1 is on mains, 255
/// unknown).
const AC_OFFLINE: u8 = 0;
/// `SYSTEM_POWER_STATUS::SystemStatusFlag` with battery saver on.
const BATTERY_SAVER_ON: u8 = 1;

pub fn state() -> PowerState {
    let mut power = SYSTEM_POWER_STATUS::default();
    // SAFETY: `power` is a valid, writable SYSTEM_POWER_STATUS that outlives
    // the call, which only fills it in.
    let read = unsafe { GetSystemPowerStatus(&mut power) };
    let (on_battery, saving_power) = match read {
        Ok(()) => (
            power.ACLineStatus == AC_OFFLINE,
            power.SystemStatusFlag == BATTERY_SAVER_ON,
        ),
        Err(e) => {
            tracing::debug!("power state: GetSystemPowerStatus: {e}");
            (false, false)
        }
    };
    PowerState {
        on_battery,
        saving_power,
        metered: metered().unwrap_or_else(|e| {
            // No internet connection has no profile, which is fine.
            tracing::debug!("power state: connection cost: {e}");
            false
        }),
    }
}

/// Whether the internet connection costs by the byte, as Microsoft's
/// guidance for metered connections reads `ConnectionCost`.
fn metered() -> windows::core::Result<bool> {
    let cost = NetworkInformation::GetInternetConnectionProfile()?.GetConnectionCost()?;
    let kind = cost.NetworkCostType()?;
    Ok(kind == NetworkCostType::Fixed
        || kind == NetworkCostType::Variable
        || cost.Roaming()?
        || cost.OverDataLimit()?)
}
