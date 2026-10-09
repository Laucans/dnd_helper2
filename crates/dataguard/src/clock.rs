//! The engine's time: tombstones, parking, expiry and holds all read it, so
//! the tests drive it by hand and never wait in real time.

use std::sync::Mutex;

use chrono::{DateTime, Utc};

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock that moves only when told to.
pub struct ManualClock(Mutex<DateTime<Utc>>);

impl ManualClock {
    pub fn new(at: DateTime<Utc>) -> Self {
        Self(Mutex::new(at))
    }

    pub fn advance(&self, by: std::time::Duration) {
        let by = chrono::Duration::from_std(by).expect("a duration within chrono's range");
        let mut now = self.0.lock().expect("clock lock");
        *now += by;
    }

    pub fn set(&self, at: DateTime<Utc>) {
        *self.0.lock().expect("clock lock") = at;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().expect("clock lock")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn manual_clock_moves_only_when_told() {
        let start = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let clock = ManualClock::new(start);
        assert_eq!(clock.now(), start);
        assert_eq!(clock.now(), start);
        clock.advance(Duration::from_secs(90));
        assert_eq!(clock.now(), start + chrono::Duration::seconds(90));
        clock.set(start);
        assert_eq!(clock.now(), start);
    }
}
