/// A value object refused its input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind}: {reason}")]
pub struct InvalidValue {
    pub kind: &'static str,
    pub reason: String,
}

impl InvalidValue {
    pub(crate) fn new(kind: &'static str, reason: impl Into<String>) -> Self {
        Self { kind, reason: reason.into() }
    }
}
