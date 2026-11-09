use chrono::{DateTime, Utc};

/// The current time (a port so that scheduling can be tested).
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// Randomness for jitter (a port so that scheduling can be tested).
pub trait Random: Send + Sync {
    /// A number in `0..=max`.
    fn up_to(&self, max: u64) -> u64;
}
