use super::*;

fn extract_backend_error_code(err: &str) -> Option<BackendErrorCode> {
    extract_backend_error_code_token(err).and_then(|token| BackendErrorCode::from_token(&token))
}

fn is_backend_malformed_bad_request(err: &str) -> bool {
    let classification = classify(Some(PROVIDER_LABEL), err);
    classification.kind == Some(BackendErrorKind::BadRequest)
        && classification.malformed_bad_request
}

fn is_backend_client_guard_leak(err: &str) -> bool {
    matches!(
        classify(Some(PROVIDER_LABEL), err).kind,
        Some(BackendErrorKind::PayloadTooLarge | BackendErrorKind::ContextLengthExceeded)
    )
}

fn backend_error_code_skips_sentry(err: &str) -> bool {
    let classification = classify(None, err);
    classification.backend_owned && !classification.reportable
}

fn managed_error_skips_sentry(err: &str) -> bool {
    backend_error_code_skips_sentry(err)
}

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
fn every_known_wire_token_maps_to_its_provider_neutral_kind() {
    for (token, kind) in [
        ("RATE_LIMITED", BackendErrorKind::RateLimited),
        (
            "USER_INSUFFICIENT_CREDITS",
            BackendErrorKind::InsufficientCredits,
        ),
        (
            "UPSTREAM_UNAVAILABLE",
            BackendErrorKind::UpstreamUnavailable,
        ),
        ("MODEL_UNAVAILABLE", BackendErrorKind::ModelUnavailable),
        ("PAYLOAD_TOO_LARGE", BackendErrorKind::PayloadTooLarge),
        (
            "CONTEXT_LENGTH_EXCEEDED",
            BackendErrorKind::ContextLengthExceeded,
        ),
        ("BAD_REQUEST", BackendErrorKind::BadRequest),
        ("INTERNAL_ERROR", BackendErrorKind::Internal),
    ] {
        let wire = format!(r#"OpenHuman API error: {{"errorCode":"{token}"}}"#);
        let classification = classify(Some(PROVIDER_LABEL), &wire);
        assert_eq!(classification.token.as_deref(), Some(token), "{token}");
        assert_eq!(classification.kind, Some(kind), "{token}");
    }
}

#[test]
fn owner_and_legacy_managed_provider_labels_keep_error_codes_provider_owned() {
    let error = r#"OpenHuman API error: {"errorCode":"RATE_LIMITED"}"#;
    assert_eq!(
        classify(Some("tinyhumans"), error).kind,
        Some(BackendErrorKind::RateLimited)
    );
    assert_eq!(
        classify(Some("OpenHuman"), error).kind,
        Some(BackendErrorKind::RateLimited)
    );
    assert_eq!(
        classify(Some("custom_openai"), error).kind,
        None,
        "an arbitrary provider must not inherit managed-backend ownership"
    );
}

#[test]
fn flattened_owner_errors_keep_backend_code_ownership() {
    let error = r#"tinyhumans API error: {"errorCode":"RATE_LIMITED"}"#;
    assert!(is_managed_backend_envelope(error));
    assert_eq!(
        classify(None, error).kind,
        Some(BackendErrorKind::RateLimited)
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
    let body = r#"OpenHuman API error: {"errorCode":"SOME_FUTURE_CODE"}"#;
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
    let body = r#"OpenHuman API error: {"errorCode":"INTERNAL_ERROR","malformed":true}"#;
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

    let embedded = r#"custom_openai API error (429): {"error":{"message":"OpenHuman API error from upstream","errorCode":"RATE_LIMITED"}}"#;
    assert!(!is_managed_backend_envelope(embedded));
    assert!(!managed_error_skips_sentry(embedded));

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

#[test]
fn hosted_classifier_drives_core_observability_without_core_token_ownership() {
    use openhuman_embed::__host::core::observability::{expected_error_kind, ExpectedErrorKind};
    use openhuman_embed::__host::inference_registry::register_backend_error_classifier;
    use std::sync::Arc;

    register_backend_error_classifier(Arc::new(super::classify));

    let owned =
        r#"OpenHuman API error (429): {"error":{"errorCode":"RATE_LIMITED","message":"x"}}"#;
    assert_eq!(
        expected_error_kind(owned),
        Some(ExpectedErrorKind::BackendErrorCodeOwned)
    );

    let unknown =
        r#"OpenHuman API error (500): {"error":{"errorCode":"FUTURE_CODE","message":"x"}}"#;
    assert_eq!(
        expected_error_kind(unknown),
        Some(ExpectedErrorKind::BackendErrorCodeOwned)
    );

    let byo = r#"custom_openai API error (429): {"error":{"errorCode":"RATE_LIMITED"}}"#;
    assert_ne!(
        expected_error_kind(byo),
        Some(ExpectedErrorKind::BackendErrorCodeOwned)
    );

    let malformed =
        r#"OpenHuman API error (400): {"error":{"errorCode":"BAD_REQUEST","malformed":true}}"#;
    assert_eq!(expected_error_kind(malformed), None);

    for code in ["PAYLOAD_TOO_LARGE", "CONTEXT_LENGTH_EXCEEDED"] {
        let guard_leak = format!(
            "OpenHuman API error (413): {{\"error\":{{\"errorCode\":\"{code}\",\"message\":\"x\"}}}}"
        );
        assert_eq!(
            expected_error_kind(&guard_leak),
            None,
            "{code} must remain reportable as a client guard leak"
        );
    }
}

#[test]
fn classification_keeps_provider_ownership_separate_from_raw_token_and_reportability() {
    let unknown = classify(
        Some(PROVIDER_LABEL),
        r#"{"error":{"errorCode":"FUTURE_BACKEND_CODE"}}"#,
    );
    assert!(unknown.provider_owned);
    assert!(unknown.backend_owned);
    assert!(!unknown.reportable);
    assert_eq!(unknown.token.as_deref(), Some("FUTURE_BACKEND_CODE"));
    assert_eq!(unknown.kind, None);

    let null_code = classify(Some(PROVIDER_LABEL), r#"{"errorCode":null}"#);
    assert!(null_code.provider_owned);
    assert!(!null_code.backend_owned);
    assert!(null_code.reportable);
    assert_eq!(null_code.token, None);

    let malformed = classify(
        Some(PROVIDER_LABEL),
        r#"{"error":{"errorCode":"BAD_REQUEST","malformed":true}}"#,
    );
    assert!(malformed.provider_owned);
    assert!(malformed.backend_owned);
    assert!(malformed.reportable);
    assert!(malformed.malformed_bad_request);
}

#[test]
fn hosted_voice_unavailable_copy_is_backend_owned_and_not_reported_again() {
    // `authed_json` already reported (or demoted as transient) the failure the
    // neutral copy replaces; the RPC boundary must not report it a second time.
    let message = super::HOSTED_VOICE_UNAVAILABLE_USER_MESSAGE;
    for err in [
        message.to_string(),
        format!("voice.reply_synthesize: {message}"),
    ] {
        let classification = classify(None, &err);
        assert!(
            classification.backend_owned && !classification.reportable,
            "{err}"
        );
        assert_eq!(classification.kind, None, "{err}");
    }
    // Another provider's error that merely quotes the sentence is not exempt.
    for err in [
        format!("custom provider error 500: {message}"),
        format!("{message} (upstream said so)"),
    ] {
        let classification = classify(None, &err);
        assert!(!classification.backend_owned, "{err}");
        assert!(classification.reportable, "{err}");
    }
}
