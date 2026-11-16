//! Which TinyHumans backend errors mean the session has expired.
//!
//! The core owns the decision to publish `SessionExpired` and its own markers
//! (`Session expired`, `SESSION_EXPIRED`, the local pre-flight guards); this
//! module owns the backend's wire shapes. [`register`] adds
//! [`is_backend_path_unauthorized`] to the core's session-expiry predicate
//! slot, so the core's `is_session_expired_error` (consulted by
//! `invoke_method` and the JSON-RPC classifier) recognises them.
//!
//! **Narrower than the original predicate** (fixed in issue #2286):
//!
//! The old predicate matched ANY `"401 + unauthorized"` pattern, which caused
//! downstream provider 401s (Discord bot token failures, BYO-key OpenAI /
//! Anthropic failures, Composio direct-mode errors) to clear the user's session
//! and log them out. The fix distinguishes between:
//!
//! - **Backend 401s** (`client::error`): formatted as
//!   `"{METHOD} /path failed (401 Unauthorized): {body}"`, e.g.
//!   `"GET /teams failed (401 Unauthorized): {"success":false}"`. These always
//!   start with an HTTP method verb followed by a space and a forward slash.
//! - **Provider / downstream 401s** (`api_error` in the core's
//!   `inference/provider/http_error.rs`): formatted as
//!   `"{ProviderName} API error (401 Unauthorized): {body}"` or
//!   `"Discord API error: ... (401): Unauthorized"`. These start with a
//!   provider name, NOT an HTTP method verb.
//!
//! `"invalid token"` is not a trigger either: too broad, it also matches
//! Discord / OAuth provider tokens.
//!
//! For inference-path backend 401s the managed provider ALREADY publishes
//! `SessionExpired` directly, so a miss here is no regression — the subscriber
//! is idempotent and a harmless double-publish would still be correct.

use std::sync::Arc;

use openhuman_embed::__host::core::observability::register_session_expiry_predicate;

/// The id the predicate registers under.
const PREDICATE_ID: &str = "tinyhumans";

/// Whether `msg` is a backend path 401: `"{METHOD} /path failed (401 …): …"`.
///
/// HEAD and OPTIONS are intentionally excluded — the backend client only
/// issues the five listed verbs (GET/POST/PUT/DELETE/PATCH) for REST JSON
/// endpoints. Anchored on the status in the `{METHOD} /path failed ({status} …)`
/// header itself — the first ` failed (` — so a 500 whose body mentions a 401
/// (`"status":401`, `upstream failed (401 …)`) is not an expired session.
pub fn is_backend_path_unauthorized(msg: &str) -> bool {
    const HEADER: &str = " failed (";
    ["GET /", "POST /", "PUT /", "DELETE /", "PATCH /"]
        .iter()
        .any(|verb| msg.starts_with(verb))
        && msg
            .find(HEADER)
            .is_some_and(|at| msg[at + HEADER.len()..].starts_with("401"))
}

/// The one status rule for the backend: only a 401 on a request authenticated
/// with the session credential means the session expired. 402/403/404 (plan,
/// tier, WAF, proxy) are refusals that keep the session, and a 401 on an API
/// key means the key was rejected — an API key has no session to expire.
pub fn backend_status_means_session_expired(status: u16, api_key: bool) -> bool {
    status == 401 && !api_key
}

/// Installs [`is_backend_path_unauthorized`] in the core's session-expiry
/// predicate slot (idempotent).
pub(crate) fn register() {
    log::debug!("[tinyhumans] session: registering the session-expiry predicate");
    register_session_expiry_predicate(PREDICATE_ID, Arc::new(is_backend_path_unauthorized));
}

#[cfg(test)]
#[path = "expiry_tests.rs"]
mod tests;
