//! Time, as one injectable source.
//!
//! Timestamps are stored as fixed-width RFC 3339 UTC strings with millisecond
//! precision (`2026-11-06T09:30:00.000Z`), so they sort and compare correctly
//! as plain strings in SQL.

use std::sync::Mutex;

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};

/// A point in time, formatted for the database on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(DateTime<Utc>);

impl Timestamp {
    pub fn to_db(self) -> String {
        self.0.to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    /// Seconds since the Unix epoch.
    pub fn unix(self) -> i64 {
        self.0.timestamp()
    }

    pub fn minus(self, delta: TimeDelta) -> Self {
        Self(self.0 - delta)
    }

    pub fn plus(self, delta: TimeDelta) -> Self {
        Self(self.0 + delta)
    }
}

impl From<DateTime<Utc>> for Timestamp {
    fn from(value: DateTime<Utc>) -> Self {
        Self(value)
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The real clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp(Utc::now())
    }
}

/// A clock tests can move by hand.
pub struct ManualClock(Mutex<DateTime<Utc>>);

impl ManualClock {
    pub fn new(start: DateTime<Utc>) -> Self {
        Self(Mutex::new(start))
    }

    pub fn advance(&self, delta: TimeDelta) {
        let mut now = self.0.lock().expect("clock lock");
        *now += delta;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        Timestamp(*self.0.lock().expect("clock lock"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn db_format_is_fixed_width_and_sortable() {
        let a = Timestamp(Utc.with_ymd_and_hms(2026, 11, 6, 9, 5, 0).unwrap());
        let b = a.plus(TimeDelta::milliseconds(7));
        assert_eq!(a.to_db(), "2026-11-06T09:05:00.000Z");
        assert_eq!(b.to_db(), "2026-11-06T09:05:00.007Z");
        assert!(a.to_db() < b.to_db());
    }
}
