use git_opt_domain::InvalidValue;

/// Errors a caller (the HTTP adapter) can tell apart.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    /// The request conflicts with the current state (duplicate, busy…).
    #[error("{0}")]
    Conflict(String),
    /// The request itself is invalid.
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<InvalidValue> for AppError {
    fn from(e: InvalidValue) -> Self {
        Self::Invalid(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
