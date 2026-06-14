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
//! 1. **Classification** ([`super::super::super::channels::providers::web_errors::classify_inference_error`]):
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

use super::openhuman_backend;

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
    let label = openhuman_backend::PROVIDER_LABEL.to_ascii_lowercase();
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
///   context pipeline's `context_window`, `src/openhuman/context/pipeline.rs`),
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
mod tests {
    use super::*;

    #[test]
    fn extracts_known_tokens() {
        let body = r#"OpenHuman API error (429 Too Many Requests): {"error":{"message":"slow down","errorCode":"RATE_LIMITED","retryAfter":30}}"#;
        assert_eq!(
            extract_backend_error_code_token(body).as_deref(),
            Some("RATE_LIMITED")
        );
        assert_eq!(
            extract_backend_error_code(body),
            Some(BackendErrorCode::RateLimited)
        );
    }

    #[test]
    fn extraction_is_case_insensitive_on_key_and_normalises_value() {
        let body = r#"{"ErrorCode":"rate_limited"}"#;
        assert_eq!(
            extract_backend_error_code_token(body).as_deref(),
            Some("RATE_LIMITED")
        );
    }

    #[test]
    fn unknown_token_is_present_but_not_recognised() {
        let body = r#"{"errorCode":"SOME_FUTURE_CODE"}"#;
        assert_eq!(
            extract_backend_error_code_token(body).as_deref(),
            Some("SOME_FUTURE_CODE")
        );
        assert_eq!(extract_backend_error_code(body), None);
        // Golden rule still applies: an unknown code is a managed error.
        assert!(backend_error_code_skips_sentry(body));
    }

    #[test]
    fn no_error_code_means_byo_path() {
        let body = r#"custom_openai API error (401 Unauthorized): {"error":{"message":"invalid api key"}}"#;
        assert_eq!(extract_backend_error_code_token(body), None);
        assert!(!backend_error_code_skips_sentry(body));
    }

    #[test]
    fn malformed_bad_request_is_the_one_paging_exception() {
        let malformed = r#"OpenHuman API error (400 Bad Request): {"error":{"errorCode":"BAD_REQUEST","malformed":true}}"#;
        assert!(is_backend_malformed_bad_request(malformed));
        assert!(!backend_error_code_skips_sentry(malformed));

        let malformed_spaced = r#"{"errorCode":"BAD_REQUEST","malformed": true,"message":"bad"}"#;
        assert!(is_backend_malformed_bad_request(malformed_spaced));
    }

    #[test]
    fn user_param_bad_request_does_not_page() {
        let user_param = r#"OpenHuman API error (400 Bad Request): {"error":{"errorCode":"BAD_REQUEST","message":"unsupported parameter"}}"#;
        assert!(!is_backend_malformed_bad_request(user_param));
        assert!(backend_error_code_skips_sentry(user_param));
    }

    #[test]
    fn client_guard_leak_codes_page_but_other_state_codes_do_not() {
        // PAYLOAD_TOO_LARGE / CONTEXT_LENGTH_EXCEEDED are limits the client
        // enforces before sending, so a backend rejection is a guard leak that
        // must page the FE — unlike genuinely backend-owned / user-state codes.
        let payload = r#"OpenHuman API error (413 Payload Too Large): {"error":{"errorCode":"PAYLOAD_TOO_LARGE","message":"too big"}}"#;
        assert!(is_backend_client_guard_leak(payload));
        assert!(!backend_error_code_skips_sentry(payload));
        assert!(!managed_error_skips_sentry(payload));

        let context = r#"OpenHuman API error (400 Bad Request): {"error":{"errorCode":"CONTEXT_LENGTH_EXCEEDED","message":"start a new chat"}}"#;
        assert!(is_backend_client_guard_leak(context));
        assert!(!backend_error_code_skips_sentry(context));

        // Contrast: these remain backend-owned / expected user-state -> suppress.
        let rate = r#"OpenHuman API error (429): {"error":{"errorCode":"RATE_LIMITED"}}"#;
        let credits =
            r#"OpenHuman API error (402): {"error":{"errorCode":"USER_INSUFFICIENT_CREDITS"}}"#;
        assert!(!is_backend_client_guard_leak(rate));
        assert!(backend_error_code_skips_sentry(rate));
        assert!(backend_error_code_skips_sentry(credits));
    }

    #[test]
    fn malformed_flag_without_bad_request_is_ignored() {
        // A stray `malformed` flag on a non-BAD_REQUEST code must not turn a
        // backend-owned error into a paging one.
        let body = r#"{"errorCode":"INTERNAL_ERROR","malformed":true}"#;
        assert!(!is_backend_malformed_bad_request(body));
        assert!(backend_error_code_skips_sentry(body));
    }

    #[test]
    fn non_string_error_code_is_not_treated_as_present_code() {
        // `"errorCode":null` (or a numeric value) must NOT latch onto the next
        // quoted key and return a bogus token (CodeRabbit).
        let body = r#"{"error":{"errorCode":null,"message":"x"}}"#;
        assert_eq!(extract_backend_error_code_token(body), None);
        assert!(!backend_error_code_skips_sentry(body));
    }

    #[test]
    fn malformed_flag_with_spaced_colon_is_detected() {
        // Pretty-printed JSON `"malformed" : true` must still flag malformed.
        let body = r#"OpenHuman API error (400 Bad Request): {"errorCode":"BAD_REQUEST","malformed" : true}"#;
        assert!(is_backend_malformed_bad_request(body));
        assert!(!managed_error_skips_sentry(body));
    }

    #[test]
    fn managed_envelope_gate_rejects_byo_payload_carrying_error_code() {
        // A BYO / direct-provider envelope that merely contains an
        // `errorCode`-shaped field must NOT be treated as backend-owned —
        // otherwise it would wrongly suppress FE Sentry.
        let byo = r#"custom_openai API error (429 Too Many Requests): {"error":{"errorCode":"RATE_LIMITED"}}"#;
        assert!(!is_managed_backend_envelope(byo));
        assert!(!managed_error_skips_sentry(byo));

        // The same body under the managed envelope IS backend-owned.
        let managed = r#"OpenHuman API error (429 Too Many Requests): {"error":{"errorCode":"RATE_LIMITED"}}"#;
        assert!(is_managed_backend_envelope(managed));
        assert!(managed_error_skips_sentry(managed));

        // The streaming envelope variant is also recognised.
        let managed_stream =
            r#"OpenHuman streaming API error (500): {"errorCode":"INTERNAL_ERROR"}"#;
        assert!(is_managed_backend_envelope(managed_stream));
        assert!(managed_error_skips_sentry(managed_stream));
    }
}
