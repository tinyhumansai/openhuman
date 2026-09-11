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
    let body =
        r#"custom_openai API error (401 Unauthorized): {"error":{"message":"invalid api key"}}"#;
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
    let body =
        r#"OpenHuman API error (400 Bad Request): {"errorCode":"BAD_REQUEST","malformed" : true}"#;
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
    let managed =
        r#"OpenHuman API error (429 Too Many Requests): {"error":{"errorCode":"RATE_LIMITED"}}"#;
    assert!(is_managed_backend_envelope(managed));
    assert!(managed_error_skips_sentry(managed));

    // The streaming envelope variant is also recognised.
    let managed_stream = r#"OpenHuman streaming API error (500): {"errorCode":"INTERNAL_ERROR"}"#;
    assert!(is_managed_backend_envelope(managed_stream));
    assert!(managed_error_skips_sentry(managed_stream));
}
