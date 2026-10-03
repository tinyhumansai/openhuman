//! The memory RPC error taxonomy.
//!
//! Every `openhuman.memory_*` failure carries one of five stable codes in its
//! structured error `data.code` (and `data.kind`): `MEMORY_OFF`,
//! `UNSUPPORTED`, `INVALID_REQUEST`, `UNAUTHORIZED` or `ENGINE`. Messages never
//! carry a credential: engine errors are already sanitised by TinyMemory, and
//! the host adds only ids and reasons.

use serde_json::json;

use crate::core::StructuredRpcError;

/// `data.code` when no usable engine is configured.
pub const MEMORY_OFF: &str = "MEMORY_OFF";
/// `data.code` when the engine does not offer the requested operation.
pub const UNSUPPORTED: &str = "UNSUPPORTED";
/// `data.code` for a malformed or refused request.
pub const INVALID_REQUEST: &str = "INVALID_REQUEST";
/// `data.code` when the engine rejected the credential.
pub const UNAUTHORIZED: &str = "UNAUTHORIZED";
/// `data.code` for the engine's own failure.
pub const ENGINE: &str = "ENGINE";

/// A memory operation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MemoryError {
    /// Memory is off: no usable engine.
    #[error("memory is off: {0}")]
    Off(String),
    /// The engine does not offer this.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The request was malformed or refused.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// The engine rejected the credential.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// The engine (or the host around it) failed.
    #[error("engine error: {0}")]
    Engine(String),
}

impl MemoryError {
    /// The stable wire code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Off(_) => MEMORY_OFF,
            Self::Unsupported(_) => UNSUPPORTED,
            Self::InvalidRequest(_) => INVALID_REQUEST,
            Self::Unauthorized(_) => UNAUTHORIZED,
            Self::Engine(_) => ENGINE,
        }
    }

    /// Shorthand for [`Self::InvalidRequest`].
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }

    /// The structured JSON-RPC error. Everything except an engine failure is
    /// an expected user-visible state, so the boundary does not report it.
    #[must_use]
    pub fn to_structured(&self) -> StructuredRpcError {
        StructuredRpcError {
            message: self.to_string(),
            data: Some(json!({ "kind": self.code(), "code": self.code() })),
            expected_user_state: !matches!(self, Self::Engine(_)),
        }
    }
}

impl From<tinymemory::Error> for MemoryError {
    fn from(error: tinymemory::Error) -> Self {
        use tinymemory::Error as E;
        match error {
            E::Unsupported(message) => Self::Unsupported(message),
            E::InvalidRequest(message) | E::NotFound(message) => Self::InvalidRequest(message),
            E::Unauthorized(message) => Self::Unauthorized(message),
            E::Config(message) => Self::InvalidRequest(message),
            E::Conflict(message) | E::Unavailable(message) | E::Engine(message) => {
                Self::Engine(message)
            }
        }
    }
}

impl From<MemoryError> for String {
    fn from(error: MemoryError) -> Self {
        error.to_structured().encode()
    }
}

/// Result alias for memory operations.
pub type MemoryResult<T> = Result<T, MemoryError>;

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
