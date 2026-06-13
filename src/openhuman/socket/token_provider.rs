//! Token-provider abstraction for the WebSocket reconnect loop.
//!
//! Separating this into its own module isolates the token contract for unit
//! testing and keeps token-refresh logic out of the connection-loop hot path.
//!
//! ## Why a callback instead of `String`?
//!
//! The original `ws_loop` captured the session token once at spawn time and
//! reused it for every retry. When the backend invalidates a JWT mid-session
//! (rotation, explicit sign-out, server-side expiry), all subsequent reconnect
//! attempts carry the same dead token — exactly the "Invalid token" retry storm
//! tracked as TAURI-RUST-9C (#2892). A callback lets the loop re-read the
//! latest token from the profile store before each attempt.

use std::sync::Arc;

/// Callable that returns the current session token on demand.
///
/// `Ok(token)` — a non-empty token was available; use it for the next attempt.
/// `Err(reason)` — no token is stored (user logged out, profile corrupt); the
///   caller should surface `reason` and exit the reconnect loop.
///
/// The provider is intentionally **synchronous** — reading a token from the
/// profile store is a lock + disk read, not an async I/O round-trip. Wrapping
/// it in `Arc` lets it be cheaply cloned into the spawned task without
/// requiring `async_fn_in_trait` or boxing.
pub(super) type TokenProvider = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

/// Returns `true` iff the failure reason carries both the Socket.IO CONNECT
/// prefix AND the `"invalid token"` sentinel — a strict double anchor to avoid
/// misclassifying unrelated bare 401s (e.g. an upstream HTTP error message
/// that happens to contain `"invalid token"`).
///
/// The upstream shape produced by `read_sio_connect_ack()` is:
/// ```text
/// Socket.IO connect error: Invalid token
/// ```
/// Matching is case-insensitive so capitalisation variants are also caught.
///
/// This function is `pub(super)` so `ws_loop.rs` and the tests module can call
/// it without exporting it beyond the `socket` domain.
pub(super) fn is_invalid_token_error(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    // Primary anchor: "invalid token" preceded by "socket.io connect error"
    // produced by our own `read_sio_connect_ack()`.
    lower.contains("socket.io connect error") && lower.contains("invalid token")
}

/// Build a static provider that always returns the same token value.
///
/// Used by `SocketManager::connect(url, token)` so the public API does not
/// change: existing callers that already have a token in hand (e.g. the CLI,
/// integration tests) can still pass it as a `&str` without touching the
/// provider layer. On each reconnect the loop will re-call the provider — in
/// this case it returns the same cloned string — which is equivalent to the
/// previous behaviour.
///
/// For **live** session-token refresh, build a provider via
/// `token_provider_from_config` (used by `handle_connect_with_session`).
pub(super) fn static_token_provider(token: String) -> TokenProvider {
    Arc::new(move || {
        if token.trim().is_empty() {
            Err("empty session token — authenticate first".to_string())
        } else {
            Ok(token.clone())
        }
    })
}

/// Build a provider that reads the latest session token from the profile store
/// on every call.
///
/// This is the **live-refresh** path used by `handle_connect_with_session`:
/// when the loop retries after a disconnect it will see any token that was
/// refreshed or re-stored since the previous attempt.
pub(super) fn token_provider_from_config(
    config: Arc<crate::openhuman::config::Config>,
) -> TokenProvider {
    Arc::new(move || {
        crate::api::jwt::get_session_token(&config)
            .map_err(|e| format!("failed to read session token: {e}"))?
            .ok_or_else(|| "no session token stored — user must log in first".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_invalid_token_error_matches_exact_sio_wire_shape() {
        assert!(is_invalid_token_error(
            "Socket.IO connect error: Invalid token"
        ));
    }

    #[test]
    fn is_invalid_token_error_is_case_insensitive() {
        assert!(is_invalid_token_error(
            "socket.io connect error: invalid token"
        ));
        assert!(is_invalid_token_error(
            "SOCKET.IO CONNECT ERROR: INVALID TOKEN"
        ));
    }

    #[test]
    fn is_invalid_token_error_requires_both_anchors() {
        // "invalid token" without the "socket.io connect error" prefix must
        // not fire — otherwise bare upstream 401s from other contexts would
        // trigger the fast-fail session-expiry path.
        assert!(!is_invalid_token_error("invalid token"));
        assert!(!is_invalid_token_error("auth error: invalid token"));
        // The SIO connect error prefix without the "invalid token" body must
        // not fire either — a server-side config error, for instance.
        assert!(!is_invalid_token_error(
            "socket.io connect error: namespace not found"
        ));
    }

    #[test]
    fn is_invalid_token_error_returns_false_for_unrelated_errors() {
        assert!(!is_invalid_token_error(
            "WebSocket connect: connection refused"
        ));
        assert!(!is_invalid_token_error("EIO OPEN: timeout"));
        assert!(!is_invalid_token_error(""));
    }

    #[test]
    fn static_provider_returns_token() {
        let provider = static_token_provider("my-token".to_string());
        assert_eq!(provider().unwrap(), "my-token");
    }

    #[test]
    fn static_provider_rejects_empty_token() {
        let provider = static_token_provider("".to_string());
        assert!(provider().is_err());
        let provider2 = static_token_provider("   ".to_string());
        assert!(provider2().is_err());
    }

    #[test]
    fn static_provider_returns_same_token_on_repeated_calls() {
        let provider = static_token_provider("tok-abc".to_string());
        // Simulates multiple reconnect attempts — must always return the same
        // cloned token (static provider semantics).
        assert_eq!(provider().unwrap(), provider().unwrap());
    }
}
