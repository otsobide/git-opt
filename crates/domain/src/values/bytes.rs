use std::fmt;

/// A size in bytes, displayed in binary units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ByteSize(u64);

impl ByteSize {
    pub const fn new(bytes: u64) -> Self {
        Self(bytes)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
        let mut value = self.0 as f64;
        let mut unit = 0;
        while value >= 1024.0 && unit < UNITS.len() - 1 {
            value /= 1024.0;
            unit += 1;
        }
        if unit == 0 { write!(f, "{} B", self.0) } else { write!(f, "{value:.2} {}", UNITS[unit]) }
    }
}

/// Size before and after an optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Savings {
    pub before: ByteSize,
    pub after: ByteSize,
}

impl Savings {
    pub const fn new(before: ByteSize, after: ByteSize) -> Self {
        Self { before, after }
    }
    pub fn saved(&self) -> ByteSize {
        ByteSize(self.before.0.saturating_sub(self.after.0))
    }
    /// Saved fraction of the original size, in percent.
    pub fn percent(&self) -> f64 {
        if self.before.0 == 0 { 0.0 } else { self.saved().0 as f64 * 100.0 / self.before.0 as f64 }
    }
    pub fn is_reduction(&self) -> bool {
        self.after < self.before
    }
}

impl std::iter::Sum for Savings {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), |acc, s| {
            Self::new(ByteSize(acc.before.0 + s.before.0), ByteSize(acc.after.0 + s.after.0))
        })
    }
}

/// The minimum gain worth a change: both a percentage and an absolute size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavingsThreshold {
    min_percent: f64,
    min_bytes: u64,
}

impl SavingsThreshold {
    pub fn new(min_percent: f64, min_bytes: u64) -> Self {
        Self { min_percent: min_percent.clamp(0.0, 99.0), min_bytes }
    }
    /// Lets any reduction through (useful in tests).
    pub fn any_reduction() -> Self {
        Self { min_percent: 0.0, min_bytes: 1 }
    }
    /// `Err(reason)` when the gain is too small to be worth a change.
    pub fn check(&self, savings: &Savings) -> Result<(), String> {
        let (saved, percent) = (savings.saved().get(), savings.percent());
        if !savings.is_reduction() {
            Err("no reduction".into())
        } else if saved < self.min_bytes || percent < self.min_percent {
            Err(format!("only {saved} bytes ({percent:.2}%)"))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(ByteSize::new(512).to_string(), "512 B");
        assert_eq!(ByteSize::new(1536).to_string(), "1.50 KiB");
        assert_eq!(ByteSize::new(5 * 1024 * 1024).to_string(), "5.00 MiB");
    }

    #[test]
    fn savings_compute_percent_and_sum() {
        let s = Savings::new(ByteSize::new(2048), ByteSize::new(1024));
        assert_eq!(s.saved(), ByteSize::new(1024));
        assert_eq!(s.percent(), 50.0);
        let total: Savings = [s, Savings::new(ByteSize::new(100), ByteSize::new(100))].into_iter().sum();
        assert_eq!(total, Savings::new(ByteSize::new(2148), ByteSize::new(1124)));
    }

    #[test]
    fn threshold_needs_both_minimums() {
        let t = SavingsThreshold::new(1.0, 256);
        assert!(t.check(&Savings::new(ByteSize::new(10_000), ByteSize::new(9_000))).is_ok());
        assert!(t.check(&Savings::new(ByteSize::new(10_000), ByteSize::new(9_900))).is_err(), "100 bytes");
        assert!(t.check(&Savings::new(ByteSize::new(1_000_000), ByteSize::new(999_000))).is_err(), "0.1%");
        assert!(t.check(&Savings::new(ByteSize::new(10), ByteSize::new(12))).is_err(), "growth");
    }
}
