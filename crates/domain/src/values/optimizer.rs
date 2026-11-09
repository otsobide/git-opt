use std::fmt;

use crate::InvalidValue;

/// Stable identifier of a format strategy (`oxipng`, `mozjpeg`…). It is used
/// in settings, cache keys and reports, so it never changes once published.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OptimizerId(String);

impl OptimizerId {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let value = value.into();
        let ok = !value.is_empty()
            && value.len() <= 32
            && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if ok { Ok(Self(value)) } else { Err(InvalidValue::new("optimizer id", format!("{value:?}"))) }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OptimizerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Everything that changes a strategy's output for a given input (tool,
/// version family, options). Part of the cache key: a new profile means
/// files are evaluated again.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Profile(String);

impl Profile {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimizer_ids_are_slugs() {
        assert!(OptimizerId::new("oxipng").is_ok());
        assert!(OptimizerId::new("Oxi PNG").is_err());
        assert!(OptimizerId::new("").is_err());
    }
}
