//! Clocks, compact relative dates and local clock times.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Source of "now", injectable so rendered dates are deterministic in tests.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// Current unix time in seconds.
    fn now(&self) -> i64;

    /// Seconds east of UTC that local time was at unix time `t`. UTC
    /// unless overridden, so test clocks show the same times everywhere.
    fn utc_offset(&self, _t: i64) -> i64 {
        0
    }
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

    fn utc_offset(&self, t: i64) -> i64 {
        use chrono::{Local, Offset, TimeZone};
        Local
            .timestamp_opt(t, 0)
            .earliest()
            .map_or(0, |d| d.offset().fix().local_minus_utc().into())
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

const MIN: i64 = 60;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;
const WEEK: i64 = 7 * DAY;
const MONTH: i64 = 30 * DAY;
const YEAR: i64 = 365 * DAY;

/// Compact age like `now`, `5m`, `3h`, `2d`, `3w`, `4mo`, `2y`.
pub fn relative(now: i64, then: i64) -> String {
    let secs = (now - then).max(0);
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

/// Date and 12-hour time like `2026-10-17 01:30 PM`, `utc_offset` seconds
/// east of UTC (see [`Clock::utc_offset`]).
pub fn clock_time(then: i64, utc_offset: i64) -> String {
    match chrono::DateTime::from_timestamp(then.saturating_add(utc_offset), 0) {
        Some(t) => t.naive_utc().format("%Y-%m-%d %I:%M %p").to_string(),
        None => "?".into(),
    }
}

/// The first time after `now` at which [`relative`]`(t, then)` stops
/// being `relative(now, then)`, assuming the clock moves forward. Lets the
/// UI redraw dates only when one of them would change.
pub fn relative_changes_at(now: i64, then: i64) -> i64 {
    let secs = (now - then).max(0);
    // The age at which the text next changes: the next multiple of the
    // current unit, or the start of the next unit if that comes first.
    let next = |unit: i64, cap: i64| ((secs / unit + 1) * unit).min(cap);
    let age = match secs {
        s if s < MIN => MIN,
        s if s < HOUR => next(MIN, HOUR),
        s if s < DAY => next(HOUR, DAY),
        s if s < WEEK => next(DAY, WEEK),
        s if s < MONTH => next(WEEK, MONTH),
        s if s < YEAR => next(MONTH, YEAR),
        _ => next(YEAR, i64::MAX),
    };
    then.saturating_add(age)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_change_times() {
        let then = 1_000_000_000;
        // Ages around every unit boundary, plus dates in the future.
        let mut ages: Vec<i64> = vec![-1_000, -60, -1, 0, 1, 30];
        for unit in [MIN, HOUR, DAY, WEEK, MONTH, YEAR] {
            for k in [1, 2, 3, 4, 11, 12, 13, 51, 52, 59, 60] {
                for d in [-1, 0, 1] {
                    ages.push(unit * k + d);
                }
            }
        }
        for age in ages {
            let now = then + age;
            let at = relative_changes_at(now, then);
            let shown = relative(now, then);
            assert!(at > now, "age {age}: {at} is not after {now}");
            // The text is monotonic in age, so unchanged just before the
            // change time means unchanged all the way there.
            assert_eq!(relative(at - 1, then), shown, "age {age}");
            assert_ne!(relative(at, then), shown, "age {age}");
        }
        // Every second of the first two hours, exhaustively.
        for now in then - 100..then + 2 * HOUR {
            let at = relative_changes_at(now, then);
            assert!(at > now);
            assert!((now..at).all(|t| relative(t, then) == relative(now, then)), "{now}");
            assert_ne!(relative(at, then), relative(now, then), "{now}");
        }
    }

    #[test]
    fn clock_times() {
        // 2026-10-17 13:30:59 UTC.
        let t = 1_792_243_859;
        assert_eq!(clock_time(t, 0), "2026-10-17 01:30 PM");
        assert_eq!(clock_time(t, -4 * HOUR), "2026-10-17 09:30 AM");
        assert_eq!(clock_time(t, 11 * HOUR), "2026-10-18 12:30 AM");
        assert_eq!(clock_time(t - 90 * MIN, 0), "2026-10-17 12:00 PM");
        assert_eq!(clock_time(0, 0), "1970-01-01 12:00 AM");
    }

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
