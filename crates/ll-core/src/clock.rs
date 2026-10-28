use crate::date::Date;
use time::OffsetDateTime;

pub fn now_unix() -> i64 { OffsetDateTime::now_utc().unix_timestamp() }

/// Injected so "last quarter" and "date is in the future" are deterministic under test.
pub trait Clock: Send + Sync {
    fn today(&self) -> Date;
    fn now_unix(&self) -> i64;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn today(&self) -> Date { Date::from_unix_seconds(now_unix()) }
    fn now_unix(&self) -> i64 { now_unix() }
}

pub struct FixedClock(pub Date);
impl Clock for FixedClock {
    fn today(&self) -> Date { self.0 }
    fn now_unix(&self) -> i64 { self.0.to_days() * 86_400 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_clock_is_fixed() {
        let today = Date::new(2026, 9, 27).unwrap();
        let c = FixedClock(today);
        assert_eq!(c.today(), today);
        assert_eq!(c.now_unix(), c.now_unix());
    }
}
