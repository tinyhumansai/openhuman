//! Which controller errors mean the OpenHuman session has expired.
//!
//! [`invoke_method`](super::invoke::invoke_method) publishes `SessionExpired`
//! when [`is_session_expired_error`] matches, and the JSON-RPC transport in
//! `openhuman-rpc` uses the same predicate to keep those failures out of
//! Sentry.

/// Helper to determine if an error message indicates an expired or invalid
/// OpenHuman backend session.
///
/// **Narrower than the previous implementation** (fixed in issue #2286):
///
/// The old predicate matched ANY `"401 + unauthorized"` pattern, which caused
/// downstream provider 401s (Discord bot token failures, BYO-key OpenAI /
/// Anthropic failures, Composio direct-mode errors) to clear the user's session
/// and log them out. The fix distinguishes between:
///
/// - **OpenHuman backend 401s** (`authed_json` in `crates/openhuman-core/src/api/rest.rs`): formatted
///   as `"{METHOD} /path failed (401 Unauthorized): {body}"`, e.g.
///   `"GET /teams failed (401 Unauthorized): {"success":false}"`. These always
///   start with an HTTP method verb followed by a space and a forward slash.
/// - **Provider / downstream 401s** (`api_error` in
///   `crates/openhuman-core/src/inference/provider/http_error.rs`): formatted as
///   `"{ProviderName} API error (401 Unauthorized): {body}"` or
///   `"Discord API error: ... (401): Unauthorized"`. These start with a
///   provider name, NOT an HTTP method verb.
///
/// **What still triggers session expiry:**
/// - `"Session expired"` — explicit body text from the OpenHuman backend.
/// - `"no backend session token"` — pre-flight guard; auth profile is missing.
/// - `"session jwt required"` — local guard; JWT already cleared by a prior 401.
/// - `"SESSION_EXPIRED"` — scheduler-gate sentinel (exact case).
/// - HTTP-method-prefixed 401s (`GET /`, `POST /`, etc.) — backend path format.
///
/// **What no longer triggers session expiry (fixed in #2286):**
/// - Provider-prefixed 401s (`"Discord API error: ..."`, `"OpenAI API error ..."`)
/// - `"invalid token"` — too broad; also matches Discord / OAuth provider tokens.
///
/// Note: for inference-path OpenHuman backend 401s, `api_error` (in
/// `inference/provider/http_error.rs`) ALREADY publishes `SessionExpired`
/// directly, so there is no regression if this predicate misses them — the
/// subscriber is idempotent and a harmless double-publish would still be correct.
pub fn is_session_expired_error(msg: &str) -> bool {
    // Explicit session-expired markers from the OpenHuman backend / local
    // guards — delegated to the shared observability classifier so both the
    // Sentry expected-error pipeline and the JSON-RPC publish boundary stay
    // in lock-step.
    if crate::core::observability::is_session_expired_message(msg) {
        return true;
    }
    // OpenHuman backend path 401s via `authed_json`:
    // format is "{METHOD} /path failed (401 Unauthorized): {body}"
    // The HTTP-method prefix distinguishes these from provider-prefixed errors.
    // HEAD and OPTIONS are intentionally excluded — `authed_json` only issues
    // the five listed verbs (GET/POST/PUT/DELETE/PATCH) for REST JSON endpoints.
    // Anchored on the status in the `{METHOD} /path failed ({status} …)`
    // header itself — the first ` failed (` — so a 500 whose body mentions a
    // 401 (`"status":401`, `upstream failed (401 …)`) is not an expired session.
    const HEADER: &str = " failed (";
    ["GET /", "POST /", "PUT /", "DELETE /", "PATCH /"]
        .iter()
        .any(|verb| msg.starts_with(verb))
        && msg
            .find(HEADER)
            .is_some_and(|at| msg[at + HEADER.len()..].starts_with("401"))
}

/// The one status rule for the OpenHuman backend: only a 401 on a request
/// authenticated with the session credential means the session expired.
/// 402/403/404 (plan, tier, WAF, proxy) are refusals that keep the session,
/// and a 401 on an API key means the key was rejected — an API key has no
/// session to expire.
pub fn backend_status_means_session_expired(status: u16, api_key: bool) -> bool {
    status == 401 && !api_key
}

/// Detect auth-looking failures that are not specific enough to clear the
/// OpenHuman session. This is only for diagnostics; it must not feed the
/// `SessionExpired` publish path.
///
/// Matches a generic `401 Unauthorized` OR a bare `"invalid token"` string,
/// either of which can come from BYO-key providers, Composio, channels, or
/// other scoped downstream calls. Used exclusively for diagnostic logging
/// at the `invoke_method` call site so provider auth failures are visible
/// in the logs without being misclassified as session expiry.
pub fn is_unconfirmed_unauthorized_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    (lower.contains("401") && lower.contains("unauthorized")) || lower.contains("invalid token")
}

#[cfg(test)]
#[path = "session_expiry_tests.rs"]
mod tests;
