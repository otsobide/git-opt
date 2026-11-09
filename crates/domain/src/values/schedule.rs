use chrono::{DateTime, Duration, Utc};

/// How often a repository is checked: a base interval plus a random delay,
/// so that many repositories do not all fetch at the same moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollPolicy {
    interval_minutes: u32,
    jitter_minutes: u32,
}

impl PollPolicy {
    pub const MAX_INTERVAL_MINUTES: u32 = 7 * 24 * 60;
    pub const MAX_JITTER_MINUTES: u32 = 24 * 60;

    /// Values are clamped to 1 minute–7 days and 0–24 hours.
    pub fn new(interval_minutes: u32, jitter_minutes: u32) -> Self {
        Self {
            interval_minutes: interval_minutes.clamp(1, Self::MAX_INTERVAL_MINUTES),
            jitter_minutes: jitter_minutes.min(Self::MAX_JITTER_MINUTES),
        }
    }
    pub fn interval_minutes(&self) -> u32 {
        self.interval_minutes
    }
    pub fn jitter_minutes(&self) -> u32 {
        self.jitter_minutes
    }

    /// `now + interval + random(0..=jitter)`. `pick(max)` must return a
    /// number in `0..=max` (the caller decides where randomness comes from).
    pub fn next_check(&self, now: DateTime<Utc>, pick: impl FnOnce(u64) -> u64) -> DateTime<Utc> {
        let max_jitter = u64::from(self.jitter_minutes) * 60;
        let jitter = pick(max_jitter).min(max_jitter);
        now + Duration::minutes(i64::from(self.interval_minutes)) + Duration::seconds(jitter as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_check_stays_within_interval_and_jitter() {
        let now = DateTime::<Utc>::from_timestamp(1_000_000, 0).unwrap();
        let p = PollPolicy::new(20, 20);
        assert_eq!(p.next_check(now, |_| 0) - now, Duration::minutes(20));
        assert_eq!(p.next_check(now, |max| max) - now, Duration::minutes(40));
        assert_eq!(p.next_check(now, |_| u64::MAX) - now, Duration::minutes(40), "pick is clamped");
    }

    #[test]
    fn values_are_clamped() {
        let p = PollPolicy::new(0, 10_000);
        assert_eq!((p.interval_minutes(), p.jitter_minutes()), (1, PollPolicy::MAX_JITTER_MINUTES));
    }
}
