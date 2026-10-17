//! The memory RPC error taxonomy.
//!
//! Every `openhuman.memory_*` failure carries one of seven stable codes in its
//! structured error `data.code` (and `data.kind`): `MEMORY_OFF`,
//! `UNSUPPORTED`, `INVALID_REQUEST`, `UNAUTHORIZED`, `INSUFFICIENT_CREDITS`,
//! `UNAVAILABLE` or `ENGINE`. The two account-wide refusals are split out of
//! `ENGINE` so a caller can tell "top up" and "try again later" apart from an
//! engine fault, and neither from an empty memory (#6718). Messages never
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
/// `data.code` when the hosted engine refused for an exhausted credit balance.
pub const INSUFFICIENT_CREDITS: &str = "INSUFFICIENT_CREDITS";
/// `data.code` when the engine could not be reached or is overloaded.
pub const UNAVAILABLE: &str = "UNAVAILABLE";
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
    /// The hosted engine refused: the account is out of credits.
    #[error("insufficient credits: {0}")]
    InsufficientCredits(String),
    /// The engine could not be reached, timed out or is overloaded.
    #[error("unavailable: {0}")]
    Unavailable(String),
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
            Self::InsufficientCredits(_) => INSUFFICIENT_CREDITS,
            Self::Unavailable(_) => UNAVAILABLE,
            Self::Engine(_) => ENGINE,
        }
    }

    /// Shorthand for [`Self::InvalidRequest`].
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }

    /// Whether this refuses every request on the account, not just this one:
    /// memory off, a rejected credential, no credits, or no engine to reach.
    /// A loop over items stops on these instead of skipping every item, and
    /// a queued job is not charged an attempt for them.
    #[must_use]
    pub fn is_account_wide(&self) -> bool {
        matches!(
            self,
            Self::Off(_)
                | Self::Unauthorized(_)
                | Self::InsufficientCredits(_)
                | Self::Unavailable(_)
        )
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

impl From<tinymemory_api::Error> for MemoryError {
    fn from(error: tinymemory_api::Error) -> Self {
        use tinymemory_api::Error as E;
        // A 402 arrives as `Engine` with the backend's code on its message.
        let credits = tinymemory_integrations::cortex::is_insufficient_credits(&error);
        match error {
            E::Unsupported(message) => Self::Unsupported(message),
            E::InvalidRequest(message) | E::NotFound(message) => Self::InvalidRequest(message),
            E::Unauthorized(message) => Self::Unauthorized(message),
            E::Config(message) => Self::InvalidRequest(message),
            E::Unavailable(message) => Self::Unavailable(message),
            E::Engine(message) if credits => Self::InsufficientCredits(message),
            E::Conflict(message) | E::Engine(message) => Self::Engine(message),
        }
    }
}

impl MemoryError {
    /// The account-wide refusal behind a recall section's skip reason, if it
    /// was one.
    ///
    /// TinyMemory's holistic recall does not fail when a section's read
    /// fails: it skips the section and keeps the engine error's text as the
    /// reason. That text is `tinymemory_api::Error`'s `Display`, so the
    /// variant can be read back from its tag (`unauthorized: `,
    /// `unavailable: `, `engine error: [USER_INSUFFICIENT_CREDITS] …`).
    #[must_use]
    pub fn refusal_from_skip_reason(reason: &str) -> Option<Self> {
        use tinymemory_api::Error as E;
        let error = if let Some(message) = reason.strip_prefix("unauthorized: ") {
            E::Unauthorized(message.to_string())
        } else if let Some(message) = reason.strip_prefix("unavailable: ") {
            E::Unavailable(message.to_string())
        } else {
            let message = reason.strip_prefix("engine error: ")?;
            E::Engine(message.to_string())
        };
        Some(Self::from(error)).filter(Self::is_account_wide)
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
