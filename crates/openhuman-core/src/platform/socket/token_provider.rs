//! Core credential callback passed through the hosted socket port.
//!
//! Keeping credential lookup at this seam lets the host refresh credentials
//! before each SDK-owned connection attempt.
//!
//! ## Why a callback instead of `String`?
//!
//! The connection transport receives the latest session token before each
//! attempt rather than retaining an expired credential across reconnects.

pub(super) use super::TokenProvider;
use std::sync::Arc;

/// The provider is asynchronous because session lookup may use the configured
/// credential store. TinyHumans bridges it to the SDK callback via the shared
/// storage blocking runtime.

/// Build a static provider that always returns the same token value.
///
/// Used by the core-facing static-token connect operation.
///
/// For **live** session-token refresh, build a provider via
/// `token_provider_from_config` (used by `handle_connect_with_session`).
pub(super) fn static_token_provider(token: String) -> TokenProvider {
    Arc::new(move || {
        let token = token.clone();
        Box::pin(async move {
            if token.trim().is_empty() {
                Err("empty session token — authenticate first".to_string())
            } else {
                Ok(token)
            }
        })
    })
}

/// Build a provider that reads the latest app-session token from the profile
/// store on every call. The channel relay requires a user session.
///
/// The TinyHumans adapter calls this again before every SDK reconnect attempt.
pub(crate) fn token_provider_from_config(config: Arc<crate::config::Config>) -> TokenProvider {
    Arc::new(move || {
        let config = Arc::clone(&config);
        Box::pin(async move {
            let token = crate::credentials::session_state::get_session_token(&config)
                .await
                .map_err(|e| format!("failed to read session token: {e}"))?
                .ok_or_else(|| "no session token stored — user must log in first".to_string())?;
            if crate::credentials::session_state::is_local_session_token(&token) {
                return Err("offline local session cannot connect to hosted socket".to_string());
            }
            Ok(token)
        })
    })
}

#[cfg(test)]
#[path = "token_provider_tests.rs"]
mod tests;
