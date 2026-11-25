//! Wire metadata assertions for the shared failure-copy table.

use super::*;
use crate::web_chat::WebChannelEvent;

const EXPECTED_COPY_KEYS: &[(&str, &str, &str)] = &[
    (
        "codex_expired",
        "chat_error.codex_session_expired",
        r#"{"provider":"openai_codex"}"#,
    ),
    ("session_expired", "chat_error.session_expired", "null"),
    (
        "local_session_managed",
        "chat_error.local_session_managed_unavailable",
        "null",
    ),
    ("action_budget", "chat_error.action_budget", "null"),
    ("max_iterations", "chat_error.max_iterations", "null"),
    ("turn_timeout_marker", "chat_error.turn_timeout", "null"),
    ("turn_timeout_harness", "chat_error.turn_timeout", "null"),
    ("empty_response", "chat_error.empty_response", "null"),
    (
        "chat_template",
        "chat_error.chat_template_rejected",
        r#"{"provider":"openai"}"#,
    ),
    (
        "rate_limited_transient",
        "chat_error.rate_limited",
        r#"{"provider":"openrouter","retry_after_secs":30}"#,
    ),
    (
        "rate_limited_business",
        "chat_error.rate_limited_billing",
        r#"{"provider":"openai"}"#,
    ),
    ("timeout", "chat_error.timeout", "null"),
    (
        "auth_error",
        "chat_error.auth_error",
        r#"{"provider":"openai"}"#,
    ),
    (
        "budget_402",
        "chat_error.budget_exhausted",
        r#"{"provider":"openai"}"#,
    ),
    ("budget_phrase", "chat_error.budget_exhausted", "null"),
    (
        "budget_phrase_provider_source",
        "chat_error.budget_exhausted",
        r#"{"provider":"openrouter"}"#,
    ),
    (
        "provider_5xx",
        "chat_error.provider_unavailable",
        r#"{"provider":"anthropic"}"#,
    ),
    ("context_overflow", "chat_error.context_overflow", "null"),
    (
        "config_rejection",
        "chat_error.model_unavailable",
        r#"{"provider":"openai"}"#,
    ),
    ("model_unavailable", "chat_error.model_unavailable", "null"),
    ("model_transient", "chat_error.provider_unavailable", "null"),
    ("vision", "chat_error.capability_unsupported", "null"),
    (
        "malformed_history_byo",
        "chat_error.malformed_history",
        r#"{"provider":"openai"}"#,
    ),
    (
        "request_rejected_4xx",
        "chat_error.request_rejected",
        r#"{"provider":"someprovider"}"#,
    ),
    ("network", "chat_error.network", "null"),
    (
        "transient_529",
        "chat_error.provider_unavailable",
        r#"{"provider":"anthropic"}"#,
    ),
    ("generic", "chat_error.inference", "null"),
    ("fallback_chain", "chat_error.inference", "null"),
    (
        "be_rate_limited",
        "chat_error.managed_rate_limited",
        r#"{"provider":"openhuman","retry_after_secs":30}"#,
    ),
    (
        "be_credits",
        "chat_error.managed_budget_exhausted",
        r#"{"provider":"openhuman"}"#,
    ),
    (
        "be_upstream",
        "chat_error.managed_unavailable",
        r#"{"provider":"openhuman"}"#,
    ),
    (
        "be_model_unavailable",
        "chat_error.managed_unavailable",
        r#"{"provider":"openhuman"}"#,
    ),
    (
        "be_payload",
        "chat_error.payload_too_large",
        r#"{"provider":"openhuman"}"#,
    ),
    (
        "be_context",
        "chat_error.context_overflow",
        r#"{"provider":"openhuman"}"#,
    ),
    (
        "be_bad_request",
        "chat_error.managed_request_rejected",
        r#"{"provider":"openhuman"}"#,
    ),
    (
        "be_bad_request_malformed",
        "chat_error.managed_malformed_request",
        r#"{"provider":"openhuman"}"#,
    ),
    (
        "be_bad_request_history",
        "chat_error.malformed_history",
        r#"{"provider":"openhuman"}"#,
    ),
    (
        "be_internal",
        "chat_error.managed_internal",
        r#"{"provider":"openhuman"}"#,
    ),
];

#[test]
fn every_failure_class_carries_its_i18n_key_and_params() {
    assert_eq!(EXPECTED_COPY_KEYS.len(), EXPECTED.len());
    for ((case, input, ..), (key_case, copy_key, params)) in
        EXPECTED.iter().zip(EXPECTED_COPY_KEYS.iter())
    {
        assert_eq!(case, key_case, "fixture rows out of order");
        let got = classify_failure_fixture(case, input);
        assert_eq!(got.copy_key, *copy_key, "{case}: copy_key");
        let got_params = got
            .copy_params
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        assert_eq!(got_params, *params, "{case}: copy_params");
    }
}

#[test]
fn provider_detail_travels_as_a_param_and_the_message_still_quotes_it() {
    let got = classify_inference_error(
        r#"openai API error (400): {"error":{"message":"bad temperature for this model"}}"#,
    );
    let params = got.copy_params.expect("provider + detail params");
    let detail = params["detail"].as_str().expect("detail param");
    assert!(detail.contains("bad temperature"), "{detail}");
    assert!(
        got.message.ends_with(&format!("\n\n> {detail}")),
        "message must keep quoting the detail: {}",
        got.message
    );
}

#[test]
fn chat_error_event_serializes_copy_key_and_params_to_literal_json() {
    let classified =
        classify_inference_error("openrouter API error (429 Too Many Requests): Retry-After: 30");
    let event = WebChannelEvent::ChatError {
        context: crate::web_chat::WebEventContext::default(),
        message: Some(classified.message.clone()),
        error_type: Some(classified.error_type.to_string()),
        error_source: None,
        error_retryable: None,
        error_retry_after_ms: None,
        error_provider: None,
        error_fallback_available: None,
        copy_key: Some(classified.copy_key.to_string()),
        copy_params: classified.copy_params,
        guardrail: None,
    };
    let json = serde_json::to_value(&event).expect("serializes");
    assert_eq!(json["copy_key"], "chat_error.rate_limited");
    assert_eq!(
        json["copy_params"],
        serde_json::json!({"provider": "openrouter", "retry_after_secs": 30})
    );
    assert_eq!(json["error_type"], "rate_limited");
    assert_eq!(json["message"], classified.message);

    // Older emitters (cancellation, guardrail) carry neither key.
    let bare = serde_json::to_value(WebChannelEvent::ChatError {
        context: crate::web_chat::WebEventContext::default(),
        message: None,
        error_type: None,
        error_source: None,
        error_retryable: None,
        error_retry_after_ms: None,
        error_provider: None,
        error_fallback_available: None,
        copy_key: None,
        copy_params: None,
        guardrail: None,
    })
    .expect("serializes");
    assert!(bare.get("copy_key").is_none() && bare.get("copy_params").is_none());
}
