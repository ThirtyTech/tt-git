//! IOKit's power sources for the battery, `NSProcessInfo` for Low Power
//! Mode, and an `NWPathMonitor` (the Network framework's C API) for an
//! expensive or constrained (Low Data Mode) connection.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2_foundation::NSProcessInfo;

use super::PowerState;

/// `kIOPSTimeRemainingUnlimited`: on mains power. Anything else (a time
/// left, or `kIOPSTimeRemainingUnknown` while it's worked out) is battery.
const TIME_REMAINING_UNLIMITED: f64 = -2.0;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPSGetTimeRemainingEstimate() -> f64;
}

#[link(name = "Network", kind = "framework")]
unsafe extern "C" {
    fn nw_path_monitor_create() -> *mut c_void;
    fn nw_path_monitor_set_queue(monitor: *mut c_void, queue: *mut c_void);
    fn nw_path_monitor_set_update_handler(monitor: *mut c_void, handler: &block2::Block<dyn Fn(*mut c_void)>);
    fn nw_path_monitor_start(monitor: *mut c_void);
    fn nw_path_is_expensive(path: *mut c_void) -> bool;
    fn nw_path_is_constrained(path: *mut c_void) -> bool;
}

// libSystem, which every program links.
unsafe extern "C" {
    fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
}

/// What the path monitor last said: [`UNKNOWN`] until its first update.
static PATH: AtomicU8 = AtomicU8::new(UNKNOWN);
const UNKNOWN: u8 = 0;
const CHEAP: u8 = 1;
const COSTLY: u8 = 2;

/// How long the first call waits for the monitor's first update.
const FIRST_UPDATE: Duration = Duration::from_millis(500);

pub fn state() -> PowerState {
    // SAFETY: takes no arguments and only reads the power sources.
    let remaining = unsafe { IOPSGetTimeRemainingEstimate() };
    PowerState {
        on_battery: remaining != TIME_REMAINING_UNLIMITED,
        saving_power: NSProcessInfo::processInfo().isLowPowerModeEnabled(),
        metered: metered(),
    }
}

/// Starts the path monitor the first time, then reads its last update.
fn metered() -> bool {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        start_monitor();
        let until = Instant::now() + FIRST_UPDATE;
        while PATH.load(Ordering::Relaxed) == UNKNOWN && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    PATH.load(Ordering::Relaxed) == COSTLY
}

/// A monitor that runs for the life of the process, updating [`PATH`] on a
/// global dispatch queue whenever the network path changes.
fn start_monitor() {
    let handler = RcBlock::new(|path: *mut c_void| {
        // SAFETY: the monitor passes a valid `nw_path_t` that it keeps alive
        // for the length of the handler.
        let costly = unsafe { nw_path_is_expensive(path) || nw_path_is_constrained(path) };
        PATH.store(if costly { COSTLY } else { CHEAP }, Ordering::Relaxed);
    });
    // SAFETY: `nw_path_monitor_create` returns a new monitor that's never
    // released, so it stays valid. The default-priority global queue
    // (identifier 0, flags 0) always exists. The monitor copies the handler
    // block, so it may be dropped after the call. The handler only touches
    // a static.
    unsafe {
        let monitor = nw_path_monitor_create();
        if monitor.is_null() {
            return;
        }
        nw_path_monitor_set_queue(monitor, dispatch_get_global_queue(0, 0));
        nw_path_monitor_set_update_handler(monitor, &handler);
        nw_path_monitor_start(monitor);
    }
}
