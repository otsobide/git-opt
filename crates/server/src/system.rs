//! The system clock and random source, for the `Clock` and `Random` ports.

use chrono::{DateTime, Utc};
use git_opt_domain::ports::{Clock, Random};

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub struct FastRandom;

impl Random for FastRandom {
    fn up_to(&self, max: u64) -> u64 {
        fastrand::u64(0..=max)
    }
}
