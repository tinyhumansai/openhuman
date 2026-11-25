//! The TinyHumans API key — the one credential a library runtime holds.
//!
//! Library mode has no user login. The key is installed into the runtime's
//! own credential store (beside its `config.toml`, never in it) before the
//! core boots, and from then on every managed-backend call authenticates
//! with it: managed inference as `Authorization: Bearer <key>`, SDK REST
//! routes as `x-api-key`. See `openhuman_core::credentials::api_key`
//! for the core side.

/// A TinyHumans API key.
///
/// Newtype so the value cannot be confused with a provider bearer or a session
/// JWT at a call site, and so `Debug` never prints it.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wrap a key. Blankness is checked at
    /// [`RuntimeBuilder::build`](super::RuntimeBuilder::build), not here, so
    /// `From` can stay infallible.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// Whether the key is empty or whitespace.
    pub fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }

    /// The raw key, for the one place that stores it.
    pub(crate) fn expose(&self) -> &str {
        self.0.trim()
    }
}

impl From<&str> for ApiKey {
    fn from(key: &str) -> Self {
        Self::new(key)
    }
}

impl From<String> for ApiKey {
    fn from(key: String) -> Self {
        Self::new(key)
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

#[cfg(test)]
#[path = "api_key_tests.rs"]
mod tests;
