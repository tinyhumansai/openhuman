//! Hosted backend error-token parsing and provider-neutral classification.
//!
//! The OpenHuman **managed backend** (PR #870 / backend `tinyhumansai/backend#870`)
//! stamps every inference error response with a stable machine-readable
//! `errorCode` field in the JSON body, e.g.
//!
//! ```json
//! {"error":{"message":"Rate limited","errorCode":"RATE_LIMITED","retryAfter":30}}
//! ```
//!
//! The provider adapter owns parsing and emits a generic
//! [`BackendErrorClassification`] through the core registry. That value keeps
//! provider ownership, backend ownership, reportability, optional token, and
//! display kind separate. Core chat and observability code consumes those
//! fields without learning hosted token vocabulary or parsing provider
//! envelopes.

use super::managed_model::PROVIDER_LABEL;
use openhuman_embed::__host::inference_registry::{BackendErrorClassification, BackendErrorKind};

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
/// Used when callers have only a flattened string and no provider identity to
/// supply to the classifier.
pub fn is_managed_backend_envelope(err: &str) -> bool {
    let label = PROVIDER_LABEL.to_ascii_lowercase();
    let lower = err.to_ascii_lowercase();
    lower.starts_with(&format!("{label} api error"))
        || lower.starts_with(&format!("{label} streaming api error"))
        // Existing diagnostics may still carry the former host-adapter label.
        || lower.starts_with("openhuman api error")
        || lower.starts_with("openhuman streaming api error")
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

/// User-facing message for a hosted voice request the backend's own upstream
/// provider refused. Nothing the user can change, so no instruction. The
/// failure behind it was already reported (or demoted as transient) by
/// `authed_json`, so the backend-error classifier marks this message
/// backend-owned and not reportable (no second event at the RPC boundary).
pub const HOSTED_VOICE_UNAVAILABLE_USER_MESSAGE: &str =
    "The voice service is temporarily unavailable. Please try again later.";

/// Whether `err` is exactly the neutral hosted-voice copy, bare or behind an
/// RPC method name (`voice.reply_synthesize: <copy>`). Matching the sentence
/// anywhere would also exempt another provider's error that happens to quote
/// it, which `authed_json` never reported.
fn is_hosted_voice_unavailable_copy(err: &str) -> bool {
    let message = HOSTED_VOICE_UNAVAILABLE_USER_MESSAGE;
    let err = err.trim();
    if err == message {
        return true;
    }
    err.strip_suffix(message)
        .and_then(|rest| rest.strip_suffix(": "))
        .is_some_and(|method| {
            !method.is_empty()
                && method
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        })
}

/// Classify an error for the core's generic observability and chat surfaces.
pub(crate) fn classify(provider: Option<&str>, err: &str) -> BackendErrorClassification {
    // The neutral hosted-voice copy replaces a failure `authed_json` already
    // reported or demoted: backend-owned, so the RPC boundary does not report
    // it again. No token or kind, so chat display is unaffected.
    if is_hosted_voice_unavailable_copy(err) {
        return BackendErrorClassification {
            provider_owned: false,
            backend_owned: true,
            reportable: false,
            token: None,
            kind: None,
            malformed_bad_request: false,
        };
    }
    let provider_owned = provider.map_or_else(
        || is_managed_backend_envelope(err),
        |provider| matches!(provider, PROVIDER_LABEL | "OpenHuman"),
    );
    let token = provider_owned
        .then(|| extract_backend_error_code_token(err))
        .flatten();
    let kind = token
        .as_deref()
        .and_then(BackendErrorCode::from_token)
        .map(|code| match code {
            BackendErrorCode::RateLimited => BackendErrorKind::RateLimited,
            BackendErrorCode::UserInsufficientCredits => BackendErrorKind::InsufficientCredits,
            BackendErrorCode::UpstreamUnavailable => BackendErrorKind::UpstreamUnavailable,
            BackendErrorCode::ModelUnavailable => BackendErrorKind::ModelUnavailable,
            BackendErrorCode::PayloadTooLarge => BackendErrorKind::PayloadTooLarge,
            BackendErrorCode::ContextLengthExceeded => BackendErrorKind::ContextLengthExceeded,
            BackendErrorCode::BadRequest => BackendErrorKind::BadRequest,
            BackendErrorCode::InternalError => BackendErrorKind::Internal,
        });
    let malformed_bad_request =
        matches!(kind, Some(BackendErrorKind::BadRequest)) && body_flags_malformed(err);
    let client_guard_leak = matches!(
        kind,
        Some(BackendErrorKind::PayloadTooLarge | BackendErrorKind::ContextLengthExceeded)
    );
    let backend_owned = provider_owned && token.is_some();
    BackendErrorClassification {
        provider_owned,
        backend_owned,
        reportable: !backend_owned || malformed_bad_request || client_guard_leak,
        token,
        kind,
        malformed_bad_request,
    }
}

#[cfg(test)]
#[path = "backend_error_code_tests.rs"]
mod tests;
