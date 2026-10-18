//! Clocks and compact relative dates.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Source of "now", injectable so rendered dates are deterministic in tests.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// Current unix time in seconds.
    fn now(&self) -> i64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

/// A clock frozen at a fixed instant.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now(&self) -> i64 {
        self.0
    }
}

pub type SharedClock = Arc<dyn Clock>;

pub fn system_clock() -> SharedClock {
    Arc::new(SystemClock)
}

/// Compact age like `now`, `5m`, `3h`, `2d`, `3w`, `4mo`, `2y`.
pub fn relative(now: i64, then: i64) -> String {
    let secs = (now - then).max(0);
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;
    const MONTH: i64 = 30 * DAY;
    const YEAR: i64 = 365 * DAY;
    match secs {
        s if s < MIN => "now".into(),
        s if s < HOUR => format!("{}m", s / MIN),
        s if s < DAY => format!("{}h", s / HOUR),
        s if s < WEEK => format!("{}d", s / DAY),
        s if s < MONTH => format!("{}w", s / WEEK),
        s if s < YEAR => format!("{}mo", s / MONTH),
        s => format!("{}y", s / YEAR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_ages() {
        let now = 1_000_000_000;
        let cases = [
            (0, "now"),
            (59, "now"),
            (60, "1m"),
            (3_599, "59m"),
            (3_600, "1h"),
            (86_400 * 2, "2d"),
            (86_400 * 7, "1w"),
            (86_400 * 45, "1mo"),
            (86_400 * 800, "2y"),
            (-500, "now"),
        ];
        for (ago, want) in cases {
            assert_eq!(relative(now, now - ago), want, "{ago}s ago");
        }
    }
}
