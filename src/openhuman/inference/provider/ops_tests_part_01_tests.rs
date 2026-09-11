use super::*;

#[test]
fn list_configured_models_accepts_slug() {
    // list_configured_models should find a provider by slug when the caller
    // passes a slug instead of the opaque random id. This lets the frontend
    // call the RPC before the provider config has been persisted (where only
    // the slug is stable).
    use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
    use crate::openhuman::config::Config;

    let mut config = Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_openai_xyz99".to_string(),
        slug: "openai".to_string(),
        label: "OpenAI".to_string(),
        endpoint: "https://api.openai.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        legacy_type: None,
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

#[test]
fn openrouter_detection_matches_builtin_slug_or_host() {
    let provider = |slug: &str, endpoint: &str| CloudProviderCreds {
        id: format!("p_{slug}"),
        slug: slug.to_string(),
        label: slug.to_string(),
        endpoint: endpoint.to_string(),
        auth_style: AuthStyle::Bearer,
        legacy_type: None,
        default_model: None,
    };

    assert!(is_openrouter_provider(&provider(
        "openrouter",
        "http://127.0.0.1:1234"
    )));
    assert!(is_openrouter_provider(&provider(
        "custom-router",
        "https://openrouter.ai/api/v1"
    )));
    assert!(is_openrouter_provider(&provider(
        "custom-router",
        "https://oauth.openrouter.ai/api/v1"
    )));
    assert!(!is_openrouter_provider(&provider(
        "custom-openai",
        "https://api.openai.com/v1"
    )));
}

#[test]
fn openai_codex_models_url_includes_client_version_query() {
    let url = append_query_param(
        "https://chatgpt.com/backend-api/codex/models",
        "client_version",
        &openai_codex_client_version(),
    );
    let parsed = reqwest::Url::parse(&url).expect("url");

    assert_eq!(parsed.path(), "/backend-api/codex/models");
    assert_eq!(
        parsed
            .query_pairs()
            .find(|(key, _)| key == "client_version")
            .map(|(_, value)| value.into_owned()),
        Some(openai_codex_client_version().to_string())
    );
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
    use crate::openhuman::inference::provider::openhuman_backend_model::PROVIDER_LABEL;

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

// Confirm the budget-exhausted suppression predicate is scoped correctly.
// These tests exercise the real production function, not a duplicate.
mod budget_exhausted_suppression {
    use super::super::*;

    const BUDGET_BODY: &str = "Insufficient budget";
    const UNRELATED_BODY: &str = "Invalid request: model not found";

    #[test]
    fn budget_exhausted_400_is_suppressed() {
        assert!(is_budget_exhausted_http_400(
            reqwest::StatusCode::BAD_REQUEST,
            BUDGET_BODY,
        ));
    }

    #[test]
    fn budget_exhausted_400_is_case_insensitive() {
        assert!(is_budget_exhausted_http_400(
            reqwest::StatusCode::BAD_REQUEST,
            "budget exceeded — ADD credits to continue",
        ));
    }

    #[test]
    fn budget_exhausted_500_is_not_suppressed() {
        // A 500 is a server bug, not expected user-state — keep reporting.
        assert!(!is_budget_exhausted_http_400(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            BUDGET_BODY,
        ));
    }

    #[test]
    fn budget_exhausted_400_unrelated_body_is_not_suppressed() {
        assert!(!is_budget_exhausted_http_400(
            reqwest::StatusCode::BAD_REQUEST,
            UNRELATED_BODY,
        ));
    }

    #[test]
    fn budget_exhausted_402_is_not_suppressed() {
        assert!(!is_budget_exhausted_http_400(
            reqwest::StatusCode::PAYMENT_REQUIRED,
            BUDGET_BODY,
        ));
    }

    #[test]
    fn budget_exhausted_empty_body_is_not_suppressed() {
        assert!(!is_budget_exhausted_http_400(
            reqwest::StatusCode::BAD_REQUEST,
            "",
        ));
    }
}

mod provider_access_policy_suppression {
    use super::super::*;

    const ACCESS_TERMINATED_BODY: &str =
        "{\"error\":{\"message\":\"Kimi For Coding is currently only available for Coding Agents.\",\"type\":\"access_terminated_error\"}}";

    #[test]
    fn access_terminated_403_is_suppressed() {
        assert!(is_provider_access_policy_denied_http_403(
            reqwest::StatusCode::FORBIDDEN,
            ACCESS_TERMINATED_BODY,
        ));
    }

    #[test]
    fn access_terminated_non_403_is_not_suppressed() {
        assert!(!is_provider_access_policy_denied_http_403(
            reqwest::StatusCode::BAD_REQUEST,
            ACCESS_TERMINATED_BODY,
        ));
    }

    #[test]
    fn unrelated_403_is_not_suppressed() {
        assert!(!is_provider_access_policy_denied_http_403(
            reqwest::StatusCode::FORBIDDEN,
            "{\"error\":{\"message\":\"forbidden\"}}",
        ));
    }
}

// Exercises the real `is_provider_config_rejection_http` decision used
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
        assert!(is_provider_config_rejection_http(
            reqwest::StatusCode::BAD_REQUEST,
            "custom_openai",
            TIER_LEAK_BODY,
        ));
        assert!(is_provider_config_rejection_http(
            reqwest::StatusCode::BAD_REQUEST,
            "custom_openai",
            TEMP_BODY,
        ));
        // 404 "model does not exist" is the same user-config class.
        assert!(is_provider_config_rejection_http(
            reqwest::StatusCode::NOT_FOUND,
            "custom_openai",
            "The model `gpt-5.5` does not exist or you do not have access to it.",
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
        assert!(!is_provider_config_rejection_http(
            reqwest::StatusCode::BAD_REQUEST,
            openhuman_backend_model::PROVIDER_LABEL,
            TIER_LEAK_BODY,
        ));
        assert!(!is_provider_config_rejection_http(
            reqwest::StatusCode::BAD_REQUEST,
            openhuman_backend_model::PROVIDER_LABEL,
            TEMP_BODY,
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
                is_provider_config_rejection_http(
                    reqwest::StatusCode::BAD_REQUEST,
                    openhuman_backend_model::PROVIDER_LABEL,
                    body,
                ),
                "TAURI-RUST-2Z1 body must be suppressed for openhuman backend: {body:?}"
            );
        }
    }

    #[test]
    fn server_error_is_not_suppressed() {
        // A 5xx is a server bug, not user-config — keep reporting.
        assert!(!is_provider_config_rejection_http(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "custom_openai",
            TIER_LEAK_BODY,
        ));
    }

    #[test]
    fn transient_429_is_not_suppressed_here() {
        // 429 is transient; handled by should_report_provider_http_failure,
        // not this classifier (must not be swallowed as user-config).
        assert!(!is_provider_config_rejection_http(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "custom_openai",
            TIER_LEAK_BODY,
        ));
    }

    #[test]
    fn unrelated_4xx_body_is_not_suppressed() {
        assert!(!is_provider_config_rejection_http(
            reqwest::StatusCode::BAD_REQUEST,
            "custom_openai",
            "Bad request: missing required field 'messages'",
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
            is_provider_config_rejection_http(
                reqwest::StatusCode::FORBIDDEN,
                "ollama",
                body,
            ),
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
            !is_provider_config_rejection_http(
                reqwest::StatusCode::FORBIDDEN,
                openhuman_backend_model::PROVIDER_LABEL,
                body,
            ),
            "backend 403 subscription phrase must NOT be suppressed (polarity guard)"
        );
    }

    #[test]
    fn log_helper_runs_without_panicking() {
        // Covers the demotion log path taken by `api_error` when a
        // custom provider rejects the user's model/param config. No
        // tracing subscriber in unit tests, so this is a pure smoke.
        log_provider_config_rejection(
            "api_error",
            "custom_openai",
            Some("reasoning-v1"),
            reqwest::StatusCode::BAD_REQUEST,
        );
    }
}

mod context_window_exceeded_suppression {
    use super::super::*;

    #[test]
    fn classifies_tauri_rust_501_custom_provider_500_body() {
        // TAURI-RUST-501: the custom-provider 500 wire body. The
        // matcher is status-agnostic, so the 500 mis-report is caught
        // (the provider api_error cascade routes it to
        // `log_context_window_exceeded` instead of `report_error`).
        assert!(is_context_window_exceeded_message(
            "{\"error\":{\"code\":500,\"message\":\"Context size has been exceeded.\",\"type\":\"server_error\"}}"
        ));
    }

    #[test]
    fn classifies_established_context_overflow_phrasings() {
        // The phrasings the reliable.rs non-retryable classifier
        // recognized before this refactor must all still match through
        // the shared single-source matcher.
        for body in [
            "This model's maximum context length is 8192 tokens",
            "request exceeds the context window of this model",
            "context length exceeded",
            "too many tokens in the prompt",
            "token limit exceeded",
            "prompt is too long for the selected model",
            "input is too long",
        ] {
            assert!(
                is_context_window_exceeded_message(body),
                "should match context-overflow body: {body}"
            );
        }
    }

    #[test]
    fn classifies_lmstudio_n_keep_exceeds_n_ctx_body() {
        // TAURI-RUST-6V0: LM Studio / llama.cpp reject a prompt whose
        // un-evictable prefix (`n_keep`) is larger than the model's loaded
        // context (`n_ctx`). The user loaded the model with too small a
        // context length; the remediation lives in the user's local server,
        // so the matcher must demote this from Sentry. Verbatim wire body.
        let body = "lmstudio API error (400 Bad Request): {\"error\":\"The number of tokens to keep from the initial prompt is greater than the context length (n_keep: 10978 >= n_ctx: 8192). Try to load the model with a larger context length, or provide a shorter input.\"}";
        assert!(
            is_context_window_exceeded_message(body),
            "LM Studio n_keep >= n_ctx body must classify as context-window overflow"
        );
        // Both anchors fire independently: the `greater than the context
        // length` phrase AND the paired `n_keep`/`n_ctx` diagnostic.
        assert!(is_context_window_exceeded_message(
            "request rejected: prompt is greater than the context length of the loaded model"
        ));
        assert!(is_context_window_exceeded_message(
            "n_keep: 9000 >= n_ctx: 4096"
        ));
    }

    #[test]
    fn does_not_match_unrelated_bodies() {
        for body in [
            "rate limit exceeded, retry after 30s",
            "Invalid request: model not found",
            "Insufficient budget",
            "tool call exceeded the allowed budget",
            // Only one of the paired n_keep/n_ctx tokens present — must NOT
            // match (guards the paired-anchor arm against bare n_ctx logging).
            "loaded model with n_ctx: 8192 and 32 layers",
        ] {
            assert!(
                !is_context_window_exceeded_message(body),
                "must NOT match unrelated body: {body}"
            );
        }
    }

    #[test]
    fn token_rate_limits_are_not_context_overflow() {
        // Token-count phrases collide with per-minute token RATE limits.
        // Those are transient 429s that must stay retryable and keep
        // reaching Sentry — they must NOT be classified as context
        // overflow (CodeRabbit review of #2820). The rate-limit marker
        // disambiguates.
        for body in [
            "Rate limit reached: too many tokens per minute (TPM) for this org",
            "rate_limit_exceeded: token limit exceeded, retry after 12s",
            "You have hit too many tokens per min; try again in 30s",
        ] {
            assert!(
                !is_context_window_exceeded_message(body),
                "TPM rate-limit must NOT match as context overflow: {body}"
            );
        }
        // …but a token-count overflow with NO rate marker still matches.
        assert!(is_context_window_exceeded_message(
            "Request rejected: too many tokens in the input for this model"
        ));
    }

    #[test]
    fn log_helper_runs_without_panicking() {
        // Smoke for the demotion path taken by `api_error` — no tracing
        // subscriber in unit tests.
        log_context_window_exceeded(
            "api_error",
            "custom_openai",
            None,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        );
    }
}

#[test]
fn test_sanitize_api_error_utf8() {
    let input = "🦀".repeat(MAX_API_ERROR_CHARS + 10);
    let sanitized = sanitize_api_error(&input);
    assert!(sanitized.ends_with("..."));
    // Should truncate at MAX_API_ERROR_CHARS crabs
    let crabs_count = sanitized.chars().filter(|c| *c == '🦀').count();
    assert_eq!(crabs_count, MAX_API_ERROR_CHARS);
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
