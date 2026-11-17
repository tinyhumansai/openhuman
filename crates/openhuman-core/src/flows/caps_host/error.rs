//! Errors returned by the host-owned capability adapters.

use std::fmt;

#[derive(Debug)]
pub(crate) enum CapsError {
    Capability(String),
}

pub(crate) type EngineError = CapsError;

impl fmt::Display for CapsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capability(message) => write!(f, "capability error: {message}"),
        }
    }
}

impl std::error::Error for CapsError {}

pub(crate) type Result<T> = std::result::Result<T, CapsError>;

impl From<String> for CapsError {
    fn from(message: String) -> Self {
        Self::Capability(message)
    }
}

impl From<&str> for CapsError {
    fn from(message: &str) -> Self {
        Self::Capability(message.to_owned())
    }
}
