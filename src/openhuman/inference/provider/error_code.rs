//! Backend `errorCode` extraction + Sentry-ownership decision.
//!
//! The OpenHuman **managed backend** (PR #870 / backend `tinyhumansai/backend#870`)
//! stamps every inference error response with a stable machine-readable
//! `errorCode` field in the JSON body, e.g.
//!
//! ```json
//! {"error":{"message":"Rate limited","errorCode":"RATE_LIMITED","retryAfter":30}}
//! ```
//!
//! That body is the only thing that distinguishes a **managed** failure (our
//! operator key / account / quota / routing) from a **BYO** failure (the user
//! runs their own provider key, no `errorCode` is present). The presence of an
//! `errorCode` is therefore the single load-bearing signal for two decisions:
//!
//! 1. **Classification** ([`super::super::super::web_chat::web_errors::classify_inference_error`]):
//!    when an `errorCode` is present, branch on it FIRST and ignore the
//!    substring heuristics; when it is absent, fall back to the substring
//!    ladder (the BYO / direct-provider path, whose "check your API key" /
//!    "check your model settings" copy is correct there).
//! 2. **Sentry ownership** (`api_error` / `before_send` / `expected_error_kind`):
//!    any response carrying an `errorCode` is owned by the backend (it already
//!    paged, or it is expected user-state) so the FE must **not** double-report
//!    — with the single exception of a backend-flagged **malformed**
//!    `BAD_REQUEST`, which means the client built a payload the backend
//!    couldn't parse (a client bug worth paging). See the spec's "golden rule"
//!    (F2) and the malformed-`BAD_REQUEST` carve-out (F8/B8).
//!
//! Everything in this module operates on the **already-flattened error string**
//! (`"OpenHuman API error (429 …): {…errorCode…}"`) because the typed provider
//! error is collapsed to a `String` at the native-bus boundary before it
//! reaches the channel classifier or the higher-layer re-report sites.

use super::openhuman_backend_model;

/// A recognised backend `errorCode` token (PR #870).
///
/// Unknown / future tokens are intentionally NOT represented here — they are
/// still detected as "an `errorCode` is present" by
/// [`extract_backend_error_code_token`] (so the Sentry golden rule still
/// applies), but they fall through to the substring ladder for *display*
/// classification rather than guessing a bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendErrorCode {
    RateLimited,
    UserInsufficientCredits,
    UpstreamUnavailable,
    ModelUnavailable,
    PayloadTooLarge,
    ContextLengthExceeded,
    BadRequest,
    InternalError,
}

impl BackendErrorCode {
    /// Parse a canonical (upper-cased) token into a known variant.
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "RATE_LIMITED" => Some(Self::RateLimited),
            "USER_INSUFFICIENT_CREDITS" => Some(Self::UserInsufficientCredits),
            "UPSTREAM_UNAVAILABLE" => Some(Self::UpstreamUnavailable),
            "MODEL_UNAVAILABLE" => Some(Self::ModelUnavailable),
            "PAYLOAD_TOO_LARGE" => Some(Self::PayloadTooLarge),
            "CONTEXT_LENGTH_EXCEEDED" => Some(Self::ContextLengthExceeded),
            "BAD_REQUEST" => Some(Self::BadRequest),
            "INTERNAL_ERROR" => Some(Self::InternalError),
            _ => None,
        }
    }
}

/// Extract the raw (upper-cased) `errorCode` token from a flattened error
/// string, or `None` when the body carries no `errorCode` field.
///
/// Returns the token **even if it is not a recognised [`BackendErrorCode`]** —
/// the mere presence of an `errorCode` means the error came through the managed
/// backend, which is what the Sentry golden rule keys on. Display
/// classification narrows further via [`BackendErrorCode::from_token`].
///
/// The key match is case-insensitive (`"errorCode"` / `"errorcode"` /
/// `"ERRORCODE"`) so a re-cased or re-serialised body still resolves; the
/// extracted **value** is upper-cased before return so callers can compare
/// against the canonical tokens regardless of how the backend cased them.
pub fn extract_backend_error_code_token(err: &str) -> Option<String> {
    // `to_ascii_lowercase` is byte-length preserving (it only remaps ASCII
    // bytes in place), so a byte index found in `lower` is also valid in the
    // original `err` — we search the lowercased copy for the key but read the
    // value out of the original to keep the token's casing intact for the
    // (defensive) upper-casing below.
    let lower = err.to_ascii_lowercase();
    const KEY: &str = "\"errorcode\"";
    let key_idx = lower.find(KEY)?;
    let after_key = &err[key_idx + KEY.len()..];
    // Skip ONLY the JSON separators (whitespace + the colon) and then require a
    // quoted string value. A non-string value (`"errorCode":null` / a number)
    // must NOT be treated as a present code — otherwise the old
    // `trim_start_matches(|c| c != '"')` skipped past the `null` and latched
    // onto the *next* key's opening quote, returning a bogus token and wrongly
    // marking the error backend-owned (CodeRabbit). `strip_prefix('"')` returns
    // `None` for a non-string value, so we bail correctly.
    let after_colon = after_key.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ':');
    let stripped = after_colon.strip_prefix('"')?;
    let end = stripped.find('"')?;
    let token = stripped[..end].trim().to_ascii_uppercase();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

/// Whether the flattened error string is a **managed-backend** envelope (the
/// `errorCode` contract only holds for errors that came through the OpenHuman
/// managed backend, `"OpenHuman API error (...)"` /
/// `"OpenHuman streaming API error (...)"`).
///
/// Load-bearing for the managed-vs-BYO distinction: a BYO / direct-provider
/// body that merely happens to carry an `errorCode`-shaped field must NOT be
/// treated as backend-owned (CodeRabbit). The provider HTTP emit sites gate on
/// the known `provider` value instead; this helper is for the string-only
/// downstream sites (`expected_error_kind`, `before_send`) that no longer carry
/// the typed provider.
pub fn is_managed_backend_envelope(err: &str) -> bool {
    let label = openhuman_backend_model::PROVIDER_LABEL.to_ascii_lowercase();
    let lower = err.to_ascii_lowercase();
    lower.contains(&format!("{label} api error"))
        || lower.contains(&format!("{label} streaming api error"))
}

/// Managed-backend Sentry-ownership decision for **string-only** call sites:
/// the error must both be a managed-backend envelope AND carry a backend
/// `errorCode` the backend owns. Wraps [`backend_error_code_skips_sentry`] with
/// the [`is_managed_backend_envelope`] gate so a BYO payload that contains an
/// `errorCode` token can't suppress FE Sentry.
pub fn managed_error_skips_sentry(err: &str) -> bool {
    is_managed_backend_envelope(err) && backend_error_code_skips_sentry(err)
}

/// Parse a recognised [`BackendErrorCode`] out of a flattened error string.
pub fn extract_backend_error_code(err: &str) -> Option<BackendErrorCode> {
    extract_backend_error_code_token(err).and_then(|t| BackendErrorCode::from_token(&t))
}

/// Whether the managed backend explicitly flagged this `BAD_REQUEST` as a
/// **malformed** payload (the client built a request the backend couldn't
/// parse), as opposed to a user-parameter rejection (an unsupported model /
/// parameter combination the user can fix).
///
/// Contract consumed from backend PR #870: the malformed variant carries a
/// `"malformed": true` flag alongside `"errorCode":"BAD_REQUEST"`. Only this
/// variant keeps paging the FE Sentry (F8/B8) — every other `errorCode` (the
/// user-param `BAD_REQUEST` included) is owned by the backend and must not
/// double-report.
pub fn body_flags_malformed(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    const KEY: &str = "\"malformed\"";
    let Some(key_idx) = lower.find(KEY) else {
        return false;
    };
    // Whitespace-tolerant: accept `"malformed":true`, `"malformed": true`, and
    // pretty-printed `"malformed" : true` (CodeRabbit) — skip arbitrary
    // whitespace and the colon before matching the boolean literal.
    let after_key = &lower[key_idx + KEY.len()..];
    let after_colon = after_key.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ':');
    after_colon.starts_with("true")
}

/// Whether the error is a backend-flagged malformed `BAD_REQUEST` — the single
/// `errorCode` case the FE *does* page (a client-built payload the backend
/// rejected as unparseable).
pub fn is_backend_malformed_bad_request(err: &str) -> bool {
    matches!(
        extract_backend_error_code(err),
        Some(BackendErrorCode::BadRequest)
    ) && body_flags_malformed(err)
}

/// Whether the `errorCode` names a limit the **client enforces before sending**,
/// so a backend rejection means our pre-send guard leaked — a client-side bug
/// worth paging, not expected user-state.
///
/// - `PAYLOAD_TOO_LARGE`: the client gates attachment size up front
///   (`app/src/lib/attachments.ts` — per-image / per-file byte caps + a
///   `too_large` reject), so an over-limit request reaching the backend means
///   the aggregate slipped past those gates.
/// - `CONTEXT_LENGTH_EXCEEDED`: the client manages context before send (the
///   context stats state's `context_window`, `src/openhuman/agent/context/stats.rs`),
///   so a backend rejection means that fitting / trimming failed.
///
/// The backend does not ops-alert either (they are 4xx, not 500), so if the FE
/// also suppressed them the guard leak would be invisible to everyone. Display
/// classification is unchanged — the user still sees the actionable copy.
pub fn is_backend_client_guard_leak(err: &str) -> bool {
    matches!(
        extract_backend_error_code(err),
        Some(BackendErrorCode::PayloadTooLarge | BackendErrorCode::ContextLengthExceeded)
    )
}

/// Sentry-ownership decision (F2 golden rule): a response carrying any backend
/// `errorCode` must **not** page the FE — the backend owns it (it already
/// paged) or it is expected user-state — *except* errors the **client** caused
/// and so still page:
/// - a backend-flagged malformed `BAD_REQUEST` (unparseable client payload), and
/// - a client-guard-leak code (`PAYLOAD_TOO_LARGE` / `CONTEXT_LENGTH_EXCEEDED`)
///   the client should have caught before sending — see
///   [`is_backend_client_guard_leak`].
///
/// Shared by the provider HTTP layer (`api_error`), the higher-layer re-report
/// classifier (`observability::expected_error_kind`), and the Sentry
/// `before_send` defense-in-depth filter so the three layers can't drift.
pub fn backend_error_code_skips_sentry(err: &str) -> bool {
    extract_backend_error_code_token(err).is_some()
        && !is_backend_malformed_bad_request(err)
        && !is_backend_client_guard_leak(err)
}

#[cfg(test)]
#[path = "error_code_tests.rs"]
mod tests;
