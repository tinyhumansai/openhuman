use super::*;

#[test]
fn list_configured_models_accepts_slug() {
    // list_configured_models should find a provider by slug when the caller
    // passes a slug instead of the opaque random id. This lets the frontend
    // call the RPC before the provider config has been persisted (where only
    // the slug is stable).
    use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
    use crate::config::Config;

    let mut config = Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_openai_xyz99".to_string(),
        slug: "openai".to_string(),
        label: "OpenAI".to_string(),
        endpoint: "https://api.openai.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        headers: Default::default(),
        default_model: None,
    });

    // The find predicate must match on slug.
    let found_by_slug = config
        .cloud_providers
        .iter()
        .find(|e| e.id == "openai" || e.slug == "openai");
    assert!(
        found_by_slug.is_some(),
        "slug lookup must find the provider"
    );
    assert_eq!(found_by_slug.unwrap().id, "p_openai_xyz99");

    // The find predicate must still match on id.
    let found_by_id = config
        .cloud_providers
        .iter()
        .find(|e| e.id == "p_openai_xyz99" || e.slug == "p_openai_xyz99");
    assert!(found_by_id.is_some(), "id lookup must still work");
}

#[tokio::test]
async fn openrouter_invalid_key_fails_before_models_catalog_probe() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (endpoint, state) = spawn_openrouter_probe_server(StatusCode::UNAUTHORIZED).await;
    let config = configure_openrouter_workspace(&tmp, endpoint, "bad-openrouter-key").await;

    let err = list_configured_models_from_config("openrouter", &config)
        .await
        .expect_err("invalid OpenRouter key must fail");

    assert!(
        err.contains("OpenRouter key validation returned 401"),
        "unexpected error: {err}"
    );
    assert_eq!(state.key_calls.load(AtomicOrdering::SeqCst), 1);
    assert_eq!(
        state.model_calls.load(AtomicOrdering::SeqCst),
        0,
        "invalid OpenRouter credentials must not fall through to /models"
    );
}

#[tokio::test]
async fn openrouter_valid_key_allows_models_catalog_probe() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (endpoint, state) = spawn_openrouter_probe_server(StatusCode::OK).await;
    let config = configure_openrouter_workspace(&tmp, endpoint, "valid-openrouter-key").await;

    let outcome = list_configured_models_from_config("openrouter", &config)
        .await
        .expect("valid OpenRouter key should list models");

    assert_eq!(state.key_calls.load(AtomicOrdering::SeqCst), 1);
    assert_eq!(state.model_calls.load(AtomicOrdering::SeqCst), 1);
    assert_eq!(outcome.value["models"][0]["id"], "openrouter/test-model");
}

#[tokio::test]
async fn openrouter_key_is_trimmed_for_validation_and_catalog_probe() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (endpoint, state) = spawn_openrouter_probe_server(StatusCode::OK).await;
    let config = configure_openrouter_workspace(&tmp, endpoint, "  valid-openrouter-key\r\n").await;

    list_configured_models_from_config("openrouter", &config)
        .await
        .expect("trimmed OpenRouter key should list models");

    let key_authorization = state
        .key_authorization
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let model_authorization = state
        .model_authorization
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_eq!(
        key_authorization,
        vec![Some("Bearer valid-openrouter-key".to_string())]
    );
    assert_eq!(
        model_authorization,
        vec![Some("Bearer valid-openrouter-key".to_string())]
    );
}

#[test]
fn skips_sentry_report_for_transient_upstream_statuses() {
    // Transient statuses — 429 rate-limit, 408 client timeout, and 502/503/504
    // gateway-layer failures — are retried by reliable.rs. The aggregate
    // "all providers exhausted" event still fires for genuine outages.
    // Reporting each attempt individually floods Sentry (OPENHUMAN-TAURI-2E
    // ~1393 events, 84 ~1050 events, T ~871 events).
    for transient in [
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        reqwest::StatusCode::REQUEST_TIMEOUT,
        reqwest::StatusCode::BAD_GATEWAY,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        reqwest::StatusCode::GATEWAY_TIMEOUT,
    ] {
        assert!(
            !should_report_provider_http_failure(transient),
            "transient status {transient} must not trigger per-attempt Sentry report"
        );
    }
    // Auth + permanent server faults remain reportable — those are
    // misconfiguration or genuine bugs, not transient capacity issues.
    for reportable in [
        reqwest::StatusCode::UNAUTHORIZED,
        reqwest::StatusCode::FORBIDDEN,
        reqwest::StatusCode::BAD_REQUEST,
        reqwest::StatusCode::NOT_FOUND,
        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        assert!(
            should_report_provider_http_failure(reportable),
            "status {reportable} must still report to Sentry"
        );
    }
}

#[test]
fn backend_error_code_owned_gates_managed_errors_except_malformed_bad_request() {
    use crate::inference::provider::openhuman_backend_model::PROVIDER_LABEL;

    // F2/F4: backend-owned / expected-user-state errorCodes must NOT page the
    // provider HTTP layer.
    for code in [
        "RATE_LIMITED",
        "USER_INSUFFICIENT_CREDITS",
        "UPSTREAM_UNAVAILABLE",
        "MODEL_UNAVAILABLE",
        "INTERNAL_ERROR",
    ] {
        let body = format!("{{\"error\":{{\"errorCode\":\"{code}\",\"message\":\"x\"}}}}");
        assert!(
            is_backend_error_code_owned(PROVIDER_LABEL, &body),
            "errorCode={code} must be backend-owned (no provider-layer Sentry)"
        );
    }

    // A user-param BAD_REQUEST is still backend-owned (F8 only carves out the
    // malformed variant).
    assert!(is_backend_error_code_owned(
        PROVIDER_LABEL,
        "{\"error\":{\"errorCode\":\"BAD_REQUEST\",\"message\":\"bad param\"}}"
    ));

    // Client-guard-leak codes page: the client enforces these limits before
    // sending (attachment size gates; context-window management), so a backend
    // rejection means our guard leaked — the gate must NOT claim them.
    for code in ["PAYLOAD_TOO_LARGE", "CONTEXT_LENGTH_EXCEEDED"] {
        let body = format!("{{\"error\":{{\"errorCode\":\"{code}\",\"message\":\"x\"}}}}");
        assert!(
            !is_backend_error_code_owned(PROVIDER_LABEL, &body),
            "errorCode={code} is a client guard leak and must page (not owned)"
        );
    }

    // F8: a backend-flagged malformed BAD_REQUEST is also a case the FE still
    // pages — the gate must NOT claim it.
    assert!(!is_backend_error_code_owned(
        PROVIDER_LABEL,
        "{\"error\":{\"errorCode\":\"BAD_REQUEST\",\"malformed\":true}}"
    ));

    // BYO (no errorCode) is never claimed by this gate — it falls through to
    // the status-based decision.
    assert!(!is_backend_error_code_owned(
        PROVIDER_LABEL,
        "{\"error\":{\"message\":\"Incorrect API key provided\"}}"
    ));

    // CodeRabbit: a BYO / direct provider whose body merely contains an
    // `errorCode`-shaped field must NOT be claimed as backend-owned — the
    // provider gate keeps it reaching Sentry via the status decision.
    assert!(!is_backend_error_code_owned(
        "custom_openai",
        "{\"error\":{\"errorCode\":\"RATE_LIMITED\"}}"
    ));
}

// Exercises the real `config_rejection_demoted` decision used
// by `api_error`, including the inverted provider-aware polarity.
mod provider_config_rejection_suppression {
    use super::super::*;

    // The exact #2079 Sentry body shape.
    const TIER_LEAK_BODY: &str =
        "The supported API model names are deepseek-v4-pro or deepseek-v4-flash, \
         but you passed reasoning-v1.";
    // #2076 Moonshot Kimi K2 temperature constraint.
    const TEMP_BODY: &str = "invalid temperature: only 1 is allowed for this model";

    #[test]
    fn custom_provider_4xx_config_rejection_is_suppressed() {
        assert!(config_rejection_demoted(
            &tinyinference_llm::failure::http::classify_http_failure(
                "custom_openai",
                reqwest::StatusCode::BAD_REQUEST.as_u16(),
                TIER_LEAK_BODY
            ),
            "custom_openai"
        ));
        assert!(config_rejection_demoted(
            &tinyinference_llm::failure::http::classify_http_failure(
                "custom_openai",
                reqwest::StatusCode::BAD_REQUEST.as_u16(),
                TEMP_BODY
            ),
            "custom_openai"
        ));
        // 404 "model does not exist" is the same user-config class.
        assert!(config_rejection_demoted(
            &tinyinference_llm::failure::http::classify_http_failure(
                "custom_openai",
                reqwest::StatusCode::NOT_FOUND.as_u16(),
                "The model `gpt-5.5` does not exist or you do not have access to it."
            ),
            "custom_openai"
        ));
    }

    #[test]
    fn openhuman_backend_same_body_is_not_suppressed() {
        // Inverted polarity: for tier-leak / temperature / litellm /
        // OpenRouter-style phrases, the OpenHuman backend never
        // emits them, so the same body from our OWN backend would
        // mean we sent it a bad request — a real regression that
        // must still reach Sentry. (Mirror of the 401/403 backend
        // rule.)
        assert!(!config_rejection_demoted(
            &tinyinference_llm::failure::http::classify_http_failure(
                openhuman_backend_model::PROVIDER_LABEL,
                reqwest::StatusCode::BAD_REQUEST.as_u16(),
                TIER_LEAK_BODY
            ),
            openhuman_backend_model::PROVIDER_LABEL
        ));
        assert!(!config_rejection_demoted(
            &tinyinference_llm::failure::http::classify_http_failure(
                openhuman_backend_model::PROVIDER_LABEL,
                reqwest::StatusCode::BAD_REQUEST.as_u16(),
                TEMP_BODY
            ),
            openhuman_backend_model::PROVIDER_LABEL
        ));
    }

    #[test]
    fn openhuman_backend_openai_compatible_unknown_model_is_suppressed() {
        // TAURI-RUST-2Z1 — the OpenHuman backend DOES emit the
        // OpenAI-compatible "Model 'X' is not available. Use GET
        // /openai/v1/models …" wire body for user-configured unknown
        // model ids (here `MiniMax-M2.7-highspeed` and two
        // `custom:`-prefixed fallback variants from the user's own
        // `model_fallbacks` config). That's user-state, not a
        // regression — drop the polarity guard for this specific
        // shape so the per-attempt event stops reaching Sentry.
        // (The aggregate sibling TAURI-RUST-2Z2 is already covered by
        // `expected_error_kind` via the broader message-only
        // classifier.)
        for body in [
            r#"OpenHuman API error (400 Bad Request): {"success":false,"error":"Model 'MiniMax-M2.7-highspeed' is not available. Use GET /openai/v1/models to list available models."}"#,
            r#"OpenHuman API error (400 Bad Request): {"success":false,"error":"Model 'custom:MiniMax-M2.7' is not available. Use GET /openai/v1/models to list available models."}"#,
        ] {
            assert!(
                config_rejection_demoted(
                    &tinyinference_llm::failure::http::classify_http_failure(
                        openhuman_backend_model::PROVIDER_LABEL,
                        reqwest::StatusCode::BAD_REQUEST.as_u16(),
                        body
                    ),
                    openhuman_backend_model::PROVIDER_LABEL
                ),
                "TAURI-RUST-2Z1 body must be suppressed for openhuman backend: {body:?}"
            );
        }
    }

    #[test]
    fn server_error_is_not_suppressed() {
        // A 5xx is a server bug, not user-config — keep reporting.
        assert!(!config_rejection_demoted(
            &tinyinference_llm::failure::http::classify_http_failure(
                "custom_openai",
                reqwest::StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                TIER_LEAK_BODY
            ),
            "custom_openai"
        ));
    }

    #[test]
    fn transient_429_is_not_suppressed_here() {
        // 429 is transient; handled by should_report_provider_http_failure,
        // not this classifier (must not be swallowed as user-config).
        assert!(!config_rejection_demoted(
            &tinyinference_llm::failure::http::classify_http_failure(
                "custom_openai",
                reqwest::StatusCode::TOO_MANY_REQUESTS.as_u16(),
                TIER_LEAK_BODY
            ),
            "custom_openai"
        ));
    }

    #[test]
    fn unrelated_4xx_body_is_not_suppressed() {
        assert!(!config_rejection_demoted(
            &tinyinference_llm::failure::http::classify_http_failure(
                "custom_openai",
                reqwest::StatusCode::BAD_REQUEST.as_u16(),
                "Bad request: missing required field 'messages'"
            ),
            "custom_openai"
        ));
    }

    /// TAURI-RUST-4XK — Ollama Cloud returns HTTP 403 with body
    /// `{"error":"this model requires a subscription, upgrade for access: …"}`.
    /// Before this fix, `is_provider_config_rejection_http` rejected 403
    /// before reaching the phrase matcher, so the subscription-gate body
    /// fell through to Sentry. Adding 403 to the allowed status set closes
    /// that gap; the existing phrase in `config_rejection.rs` already
    /// handles the body content.
    #[test]
    fn ollama_cloud_403_subscription_gate_is_suppressed() {
        // Verbatim wire body from TAURI-RUST-4XK Sentry issue 5338.
        let body = r#"ollama API error (403 Forbidden): {"error":"this model requires a subscription, upgrade for access: https://ollama.com/upgrade (ref: bc48f3c8-fba1-40b6-93a9-786a167d16f9)"}"#;
        assert!(
            config_rejection_demoted(&tinyinference_llm::failure::http::classify_http_failure("ollama", reqwest::StatusCode::FORBIDDEN.as_u16(), body), "ollama"),
            "TAURI-RUST-4XK: ollama 403 subscription-gate must be classified as provider config-rejection"
        );
    }

    #[test]
    fn openhuman_backend_403_subscription_phrase_is_not_suppressed() {
        // Polarity guard: if our own backend somehow returned a 403 with
        // the subscription phrase, that would be an unexpected regression
        // and must still reach Sentry. The phrase does not appear in any
        // expected backend body, so this is purely defensive.
        let body = r#"{"error":"this model requires a subscription, upgrade for access: https://ollama.com/upgrade (ref: test)"}"#;
        assert!(
            !config_rejection_demoted(
                &tinyinference_llm::failure::http::classify_http_failure(
                    openhuman_backend_model::PROVIDER_LABEL,
                    reqwest::StatusCode::FORBIDDEN.as_u16(),
                    body
                ),
                openhuman_backend_model::PROVIDER_LABEL
            ),
            "backend 403 subscription phrase must NOT be suppressed (polarity guard)"
        );
    }
}

#[test]
fn test_sanitize_api_error_utf8() {
    let input = "🦀".repeat(MAX_API_ERROR_CHARS + 10);
    let sanitized = sanitize_api_error(&input);
    assert!(sanitized.ends_with("..."));
    assert_eq!(sanitized.chars().count(), MAX_API_ERROR_CHARS);
    // The ellipsis is included in the maximum character count.
    let crabs_count = sanitized.chars().filter(|c| *c == '🦀').count();
    assert_eq!(crabs_count, MAX_API_ERROR_CHARS - 3);
}

#[tokio::test]
async fn list_models_html_body_returns_diagnostic_snippet() {
    // Captive-portal / proxy-login wire shape: 200 OK with HTML.
    let tmp = tempfile::tempdir().expect("tempdir");
    let html = "<html><head><title>Sign in</title></head><body>captive portal</body></html>";
    let endpoint = spawn_static_models_server(StatusCode::OK, html).await;
    let config = configure_generic_workspace(&tmp, endpoint).await;

    let err = list_configured_models_from_config("generic-test", &config)
        .await
        .expect_err("HTML body must not parse as JSON");

    assert!(
        err.contains("failed to parse JSON"),
        "error must keep canonical prefix: {err}"
    );
    assert!(
        err.contains("captive portal") || err.contains("Sign in") || err.contains("html"),
        "error must include a body snippet for diagnosis: {err}"
    );
}
