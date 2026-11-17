//! Errors a live voice session reports to its client.

use tinyagents_live::tinyliveagents::Error as LiveError;

/// A failure with a stable, content-free `code` the UI can switch on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveVoiceError {
    pub code: &'static str,
    pub message: String,
}

impl LiveVoiceError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The provider needs a key or sign-in it does not have.
    pub fn not_configured(message: impl Into<String>) -> Self {
        Self::new("not_configured", message)
    }

    /// The request named something that does not exist.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message)
    }

    /// The TinyHumans backend refused or failed.
    pub fn backend(message: impl Into<String>) -> Self {
        Self::new("backend", message)
    }

    /// Something inside the core failed (agent build, config).
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }
}

impl From<LiveError> for LiveVoiceError {
    fn from(error: LiveError) -> Self {
        let code = match &error {
            LiveError::Unauthorized => "unauthorized",
            LiveError::InsufficientCredits => "insufficient_credits",
            LiveError::RateLimited => "rate_limited",
            LiveError::Timeout => "timeout",
            LiveError::InvalidConfig(_) => "invalid_request",
            LiveError::Connect(_) => "connect",
            _ => "provider",
        };
        Self::new(code, error.to_string())
    }
}

impl std::fmt::Display for LiveVoiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for LiveVoiceError {}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
