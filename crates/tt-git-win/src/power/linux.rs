//! Over the system bus: UPower's `OnBattery`, power-profiles-daemon's
//! `ActiveProfile`, and NetworkManager's `Metered`. A service that isn't
//! running leaves its part `false`.

use std::sync::OnceLock;

use zbus::blocking::{Connection, Proxy};

use super::PowerState;

/// NetworkManager's `NMMetered`: `NM_METERED_YES` and `NM_METERED_GUESS_YES`.
const METERED: [u32; 2] = [1, 3];

pub fn state() -> PowerState {
    let Some(bus) = bus() else {
        return PowerState::default();
    };
    let on_battery = property::<bool>(bus, "org.freedesktop.UPower", "/org/freedesktop/UPower", "OnBattery");
    // The service took UPower's name in 0.30; older ones only have this.
    let profile = property::<String>(
        bus,
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
        "ActiveProfile",
    )
    .or_else(|| {
        property::<String>(
            bus,
            "net.hadess.PowerProfiles",
            "/net/hadess/PowerProfiles",
            "ActiveProfile",
        )
    });
    let metered = property::<u32>(
        bus,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "Metered",
    );
    PowerState {
        on_battery: on_battery.unwrap_or(false),
        saving_power: profile.as_deref() == Some("power-saver"),
        metered: metered.is_some_and(|m| METERED.contains(&m)),
    }
}

/// The system bus, connected once.
fn bus() -> Option<&'static Connection> {
    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    BUS.get_or_init(|| {
        Connection::system()
            .inspect_err(|e| tracing::debug!("power state: no system bus: {e}"))
            .ok()
    })
    .as_ref()
}

/// Property `name` of the object at `path` on `service`, whose interface is
/// named like the service.
fn property<T>(bus: &Connection, service: &'static str, path: &'static str, name: &str) -> Option<T>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    let proxy = Proxy::new(bus, service, path, service).ok()?;
    proxy
        .get_property(name)
        .inspect_err(|e| tracing::debug!("power state: {service} {name}: {e}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `busctl get-property`'s answer, like `b true`, or `None` when the
    /// service isn't there.
    fn busctl(service: &str, path: &str, name: &str) -> Option<String> {
        let out = std::process::Command::new("busctl")
            .args(["--system", "get-property", service, path, service, name])
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Reads what `busctl` reads, on a machine with the services.
    #[test]
    fn matches_busctl() {
        let state = state();
        if let Some(on) = busctl("org.freedesktop.UPower", "/org/freedesktop/UPower", "OnBattery") {
            assert_eq!(state.on_battery, on == "b true", "{on}");
        }
        if let Some(m) = busctl(
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "Metered",
        ) {
            assert_eq!(state.metered, m == "u 1" || m == "u 3", "{m}");
        }
        if let Some(p) = busctl(
            "org.freedesktop.UPower.PowerProfiles",
            "/org/freedesktop/UPower/PowerProfiles",
            "ActiveProfile",
        ) {
            assert_eq!(state.saving_power, p == "s \"power-saver\"", "{p}");
        }
    }
}
