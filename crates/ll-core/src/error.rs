use crate::money::MoneyError;
use serde::{Deserialize, Serialize};

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Money(#[from] MoneyError),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("storage error: {0}")]
    Store(String),
    #[error("inference error: {0}")]
    Inference(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("integration error: {0}")]
    Integration(String),
    #[error("cancelled")]
    Cancelled,
}

/// Stable, serializable classification the UI switches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode { Invalid, NotFound, Validation, Storage, Inference, Io, Integration, Cancelled }

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Invalid => "invalid", ErrorCode::NotFound => "not_found",
            ErrorCode::Validation => "validation", ErrorCode::Storage => "storage",
            ErrorCode::Inference => "inference", ErrorCode::Io => "io",
            ErrorCode::Integration => "integration", ErrorCode::Cancelled => "cancelled",
        }
    }
}

impl Error {
    pub fn storage(error: impl std::fmt::Display) -> Self { Self::Store(error.to_string()) }
    pub fn validation(message: impl Into<String>) -> Self { Self::Validation(message.into()) }
    pub fn parse(message: impl Into<String>) -> Self { Self::Invalid(message.into()) }

    pub fn code(&self) -> ErrorCode {
        match self {
            Error::Invalid(_) | Error::Money(_) => ErrorCode::Invalid,
            Error::NotFound(_) => ErrorCode::NotFound,
            Error::Validation(_) => ErrorCode::Validation,
            Error::Store(_) => ErrorCode::Storage,
            Error::Inference(_) => ErrorCode::Inference,
            Error::Io(_) => ErrorCode::Io,
            Error::Integration(_) => ErrorCode::Integration,
            Error::Cancelled => ErrorCode::Cancelled,
        }
    }

    /// Consulted by the job queue's backoff logic.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Error::Io(_) | Error::Inference(_) | Error::Integration(_))
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self { Self::Io(error.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codes_and_retryability() {
        assert_eq!(Error::from(MoneyError::NoDigits).code(), ErrorCode::Invalid);
        assert!(Error::Io("disk".into()).is_retryable());
        assert!(!Error::Validation("x".into()).is_retryable());
        assert_eq!(serde_json::to_string(&ErrorCode::NotFound).unwrap(), "\"not_found\"");
    }
}
