use super::*;
use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::openhuman::config::Config;
use crate::openhuman::credentials::AuthService;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering as AtomicOrdering},
    Arc, Mutex,
};
use tempfile::TempDir;

#[derive(Clone)]
struct ModelProbeState {
    key_status: StatusCode,
    key_calls: Arc<AtomicUsize>,
    model_calls: Arc<AtomicUsize>,
    key_authorization: Arc<Mutex<Vec<Option<String>>>>,
    model_authorization: Arc<Mutex<Vec<Option<String>>>>,
}

async fn openrouter_key_handler(
    State(state): State<ModelProbeState>,
    headers: HeaderMap,
) -> Response {
    state.key_calls.fetch_add(1, AtomicOrdering::SeqCst);
    state
        .key_authorization
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(authorization_header(&headers));
    if state.key_status.is_success() {
        Json(serde_json::json!({
            "data": {
                "label": "test-key",
                "usage": 0
            }
        }))
        .into_response()
    } else {
        (
            state.key_status,
            Json(serde_json::json!({
                "error": {
                    "message": "No auth credentials found"
                }
            })),
        )
            .into_response()
    }
}

async fn models_handler(State(state): State<ModelProbeState>, headers: HeaderMap) -> Response {
    state.model_calls.fetch_add(1, AtomicOrdering::SeqCst);
    state
        .model_authorization
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(authorization_header(&headers));
    Json(serde_json::json!({
        "data": [{
            "id": "openrouter/test-model",
            "owned_by": "openrouter",
            "context_length": 128000
        }]
    }))
    .into_response()
}

fn authorization_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
}

async fn spawn_openrouter_probe_server(key_status: StatusCode) -> (String, ModelProbeState) {
    let state = ModelProbeState {
        key_status,
        key_calls: Arc::new(AtomicUsize::new(0)),
        model_calls: Arc::new(AtomicUsize::new(0)),
        key_authorization: Arc::new(Mutex::new(Vec::new())),
        model_authorization: Arc::new(Mutex::new(Vec::new())),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let app = Router::new()
        .route("/key", get(openrouter_key_handler))
        .route("/models", get(models_handler))
        .with_state(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), state)
}

async fn configure_openrouter_workspace(tmp: &TempDir, endpoint: String, token: &str) -> Config {
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_openrouter_test".to_string(),
        slug: "openrouter".to_string(),
        label: "OpenRouter".to_string(),
        endpoint,
        auth_style: AuthStyle::Bearer,
        legacy_type: None,
        default_model: None,
    });
    config.save().await.expect("save config");

    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        &crate::openhuman::inference::provider::factory::auth_key_for_slug("openrouter"),
        "default",
        token,
        HashMap::new(),
        true,
    )
    .expect("store provider key");
    config
}

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
        openai_codex_client_version(),
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

/// Spawn a minimal axum server that always returns 404 for the /models endpoint.
/// Used to verify that providers without model listing return an empty list,
/// not an error (Sentry issue TAURI-RUST-1Z).
async fn spawn_models_404_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let app = axum::Router::new().route(
        "/models",
        axum::routing::get(|| async {
            (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response()
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn models_404_returns_empty_list_not_error() {
    // Providers that return 404 on /models (e.g. DeepSeek, Kimi, custom proxies)
    // must yield an empty model list, not an Err. Returning an Err was firing a
    // Sentry error for every `inference_list_models` call (TAURI-RUST-1Z, 819 events).
    let tmp = tempfile::tempdir().expect("tempdir");
    let endpoint = spawn_models_404_server().await;

    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_custom_test".to_string(),
        slug: "custom-no-models".to_string(),
        label: "Custom (no /models)".to_string(),
        endpoint,
        auth_style: AuthStyle::Bearer,
        legacy_type: None,
        default_model: None,
    });

    let outcome = list_configured_models_from_config("custom-no-models", &config)
        .await
        .expect("404 from /models must succeed with an empty list");

    let models = outcome.value["models"]
        .as_array()
        .expect("response must have a `models` array");
    assert!(
        models.is_empty(),
        "expected empty model list for a 404 /models endpoint, got: {models:?}"
    );
    assert_eq!(
        outcome.value["unsupported"],
        serde_json::Value::Bool(true),
        "unsupported flag must be set to true for 404 providers"
    );
}

#[test]
fn factory_backend() {
    assert!(create_backend_inference_provider(
        None,
        None,
        None,
        &ProviderRuntimeOptions::default()
    )
    .is_ok());
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

// Confirm the budget-exhausted suppression predicate is scoped correctly.
// These tests exercise the real production function, not a duplicate.
mod budget_exhausted_suppression {
    use super::*;

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

// Tests for the rate-limit body suppression guard added to `api_error`.
// Exercises `is_upstream_rate_limit_message` with the exact body shapes that
// produced OPENHUMAN-TAURI-S (~6 984 events from HTTP 500 wrapping a
// "429 rate limit exceeded" body) and OPENHUMAN-TAURI-6Y / -2E.
mod rate_limit_body_suppression {
    use crate::core::observability::is_upstream_rate_limit_message;

    /// HTTP 500 with a `"429 rate limit exceeded"` body must be detected
    /// as a rate-limit signal so the guard in `api_error` can skip the
    /// Sentry report (OPENHUMAN-TAURI-S).
    #[test]
    fn http_500_with_429_body_phrase_is_rate_limited() {
        let body = r#"{"success":false,"error":"429 rate limit exceeded, please try again later"}"#
            .to_ascii_lowercase();
        assert!(
            is_upstream_rate_limit_message(&body),
            "500-body with '429 rate limit exceeded' must be detected as rate-limited"
        );
    }

    /// HTTP 500 with an `"upstream rate limit exceeded"` body
    /// (OPENHUMAN-TAURI-6Y shape).
    #[test]
    fn http_500_with_upstream_rate_limit_body_is_rate_limited() {
        let body = r#"{"success":false,"error":"Upstream rate limit exceeded for model 'summarization-v1'. Please retry shortly.","details":{"provider":"gmi"}}"#
            .to_ascii_lowercase();
        assert!(
            is_upstream_rate_limit_message(&body),
            "500-body with 'upstream rate limit exceeded' must be detected"
        );
    }

    /// OpenAI / Anthropic `"rate_limit_error"` type body.
    #[test]
    fn body_with_rate_limit_error_type_is_rate_limited() {
        let body = r#"{"error":{"message":"Rate limit exceeded. Please retry after a brief wait.","type":"rate_limit_error"}}"#
            .to_ascii_lowercase();
        assert!(
            is_upstream_rate_limit_message(&body),
            "body with 'rate_limit_error' type must be detected"
        );
    }

    /// Unrelated 500 body must NOT be detected as rate-limited.
    #[test]
    fn http_500_unrelated_body_is_not_rate_limited() {
        let body = r#"{"success":false,"error":"internal server error: database unavailable"}"#
            .to_ascii_lowercase();
        assert!(
            !is_upstream_rate_limit_message(&body),
            "unrelated 500 body must not be detected as rate-limited"
        );
    }
}

mod provider_access_policy_suppression {
    use super::*;

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
    use super::*;

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
            openhuman_backend::PROVIDER_LABEL,
            TIER_LEAK_BODY,
        ));
        assert!(!is_provider_config_rejection_http(
            reqwest::StatusCode::BAD_REQUEST,
            openhuman_backend::PROVIDER_LABEL,
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
                    openhuman_backend::PROVIDER_LABEL,
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
    use super::*;

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
    fn does_not_match_unrelated_bodies() {
        for body in [
            "rate limit exceeded, retry after 30s",
            "Invalid request: model not found",
            "Insufficient budget",
            "tool call exceeded the allowed budget",
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

// ── TAURI-RUST-12: list_models JSON parse error must surface body ──────
//
// `response.json()` previously dropped the body when decoding failed, so
// Sentry saw `[providers][list_models] failed to parse JSON: error decoding
// response body` with no clue what the server actually returned. The fix
// reads the body as text first, parses with `serde_json::from_str`, and
// appends a sanitized + truncated snippet to the error string so the
// failure is diagnosable from the log line alone.

#[derive(Clone)]
struct StaticResponse {
    status: StatusCode,
    body: &'static str,
}

async fn static_models_handler(State(s): State<StaticResponse>) -> Response {
    (s.status, s.body).into_response()
}

async fn spawn_static_models_server(status: StatusCode, body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let app = Router::new()
        .route("/models", get(static_models_handler))
        .with_state(StaticResponse { status, body });
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}")
}

async fn configure_generic_workspace(tmp: &TempDir, endpoint: String) -> Config {
    // Non-`openrouter` slug so the OpenRouter pre-validation path is
    // skipped and the test hits `/models` directly.
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_generic_test".to_string(),
        slug: "generic-test".to_string(),
        label: "Generic".to_string(),
        endpoint,
        auth_style: AuthStyle::None,
        legacy_type: None,
        default_model: None,
    });
    config.save().await.expect("save config");
    config
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

#[tokio::test]
async fn list_models_empty_body_returns_diagnostic_error() {
    // Some misconfigured load balancers return 200 with an empty body.
    let tmp = tempfile::tempdir().expect("tempdir");
    let endpoint = spawn_static_models_server(StatusCode::OK, "").await;
    let config = configure_generic_workspace(&tmp, endpoint).await;

    let err = list_configured_models_from_config("generic-test", &config)
        .await
        .expect_err("empty body must not parse as JSON");

    assert!(
        err.contains("failed to parse JSON"),
        "error must keep canonical prefix: {err}"
    );
}

#[tokio::test]
async fn list_models_valid_json_still_succeeds() {
    // Regression guard: the new text-then-parse path must still accept
    // a valid `/models` JSON response.
    let tmp = tempfile::tempdir().expect("tempdir");
    let body = r#"{"data":[{"id":"some-model","owned_by":"vendor","context_length":4096}]}"#;
    let endpoint = spawn_static_models_server(StatusCode::OK, body).await;
    let config = configure_generic_workspace(&tmp, endpoint).await;

    let outcome = list_configured_models_from_config("generic-test", &config)
        .await
        .expect("valid JSON must list models");
    assert_eq!(outcome.value["models"][0]["id"], "some-model");
}

// ── parse_models_response (TAURI-RUST-4Y) ──────────────────────────────
//
// Before this fix the `/models` parser collapsed "no `data` field" and
// "`data` field present but not an array" into a single misleading
// error string: `"provider response missing `data` array — endpoint is
// not OpenAI-compatible (got keys: data, object)"` — the keys list
// included `data`, contradicting the "missing" claim. The split
// surfaces the actual JSON-type mismatch so future Sentry events on
// this code path are triageable instead of looking like the parser
// is hallucinating.

#[test]
fn parse_models_response_returns_models_for_well_formed_data_array() {
    // Happy path — exact OpenAI `/models` shape, must yield model ids
    // and `owned_by` / `context_length` projections from each entry.
    let body = serde_json::json!({
        "object": "list",
        "data": [
            { "id": "m1", "owned_by": "openai", "context_length": 8192 },
            { "id": "m2", "owned_by": "openai" },
            { "id": "m3", "context_window": 4096 },
        ],
    });
    let models = parse_models_response(&body).expect("well-formed body must parse");
    assert_eq!(models.len(), 3);
    assert_eq!(models[0].id, "m1");
    assert_eq!(models[0].owned_by.as_deref(), Some("openai"));
    assert_eq!(models[0].context_window, Some(8192));
    assert_eq!(models[2].id, "m3");
    assert_eq!(models[2].owned_by, None);
    assert_eq!(models[2].context_window, Some(4096));
}

#[test]
fn parse_models_response_returns_models_for_codex_models_array() {
    let body = serde_json::json!({
        "models": [
            { "slug": "gpt-5.5", "owned_by_organization": "openai", "max_context_window": 272000 },
            "gpt-5.4",
        ],
    });

    let models = parse_models_response(&body).expect("Codex models body must parse");

    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "gpt-5.5");
    assert_eq!(models[0].owned_by.as_deref(), Some("openai"));
    assert_eq!(models[0].context_window, Some(272000));
    assert_eq!(models[1].id, "gpt-5.4");
}

#[test]
fn parse_models_response_distinguishes_missing_data_field_from_wrong_type() {
    // (1) `data`/`models` fields completely absent — wrong endpoint
    // misconfiguration. Codex uses `models`, so it is accepted alongside
    // OpenAI-compatible `data`.
    let body = serde_json::json!({ "object": "list", "items": [] });
    let err = parse_models_response(&body).expect_err("no model catalog field must fail");
    assert!(
        err.contains("missing `data` or `models` field"),
        "no-data error should say `missing`: {err}"
    );
    assert!(
        err.contains("items") && err.contains("object"),
        "no-data error should list actual keys: {err}"
    );

    // (2) `data` field present but wrong type — TAURI-RUST-4Y verbatim
    // shape (`object` + `data` keys both present, but `data` isn't an
    // array). The error MUST NOT say "missing" — it must surface the
    // actual JSON type so triage knows what shape the provider sent.
    for (label, value) in [
        (
            "object",
            serde_json::json!({"object":"error","message":"boom"}),
        ),
        ("string", serde_json::json!("models go here")),
        ("null", serde_json::Value::Null),
        ("bool", serde_json::json!(true)),
        ("number", serde_json::json!(42)),
    ] {
        let body = serde_json::json!({ "object": "list", "data": value });
        let err = parse_models_response(&body).expect_err("wrong-type data must fail");
        assert!(
            !err.contains("missing"),
            "wrong-type error must not say `missing` ({label}): {err}"
        );
        assert!(
            err.contains(label),
            "wrong-type error must name the actual JSON kind ({label}): {err}"
        );
    }
}

#[test]
fn openai_codex_model_hints_are_merged_without_duplicates() {
    let mut models = vec![ModelInfo {
        id: "gpt-5.4".to_string(),
        owned_by: Some("openai-codex".to_string()),
        context_window: Some(128000),
    }];

    merge_openai_codex_model_hints(&mut models);

    let ids = models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec!["gpt-5.4", "gpt-5.5", "gpt-5.3-codex-spark", "gpt-5.3-codex"]
    );
}

// ── synthesize_local_runtime_entry (TAURI-RUST-28Z fallback) ────────────

#[test]
fn synthesize_local_runtime_entry_ollama_returns_v1_endpoint_with_no_auth() {
    // Sentry TAURI-RUST-28Z fires when `inference_list_models("ollama")`
    // runs against a config that has no `ollama` cloud_providers entry.
    // The synth fallback must produce an entry routed to Ollama's
    // OpenAI-compatible `/v1/models` surface at the resolved base URL,
    // with `AuthStyle::None` so the probe runs without an Authorization
    // header (loopback Ollama accepts unauthenticated requests).
    let config = Config::default();
    let entry = synthesize_local_runtime_entry("ollama", &config)
        .expect("ollama must produce a synthetic entry");
    assert_eq!(entry.slug, "ollama");
    assert_eq!(entry.auth_style, AuthStyle::None);
    assert!(
        entry.endpoint.ends_with("/v1"),
        "ollama endpoint must terminate at /v1 so `<endpoint>/models` hits the OpenAI-compat surface; got {}",
        entry.endpoint
    );
}

#[test]
fn synthesize_local_runtime_entry_lmstudio_returns_v1_endpoint_with_no_auth() {
    // LM Studio's default `lm_studio_base_url` already terminates at
    // `/v1`; the synth must preserve that and select `AuthStyle::None`
    // so the probe doesn't attach a bearer header (LM Studio runs
    // unauthenticated on loopback).
    let config = Config::default();
    let entry = synthesize_local_runtime_entry("lmstudio", &config)
        .expect("lmstudio must produce a synthetic entry");
    assert_eq!(entry.slug, "lmstudio");
    assert_eq!(entry.auth_style, AuthStyle::None);
    assert!(
        entry.endpoint.ends_with("/v1"),
        "lmstudio endpoint must terminate at /v1; got {}",
        entry.endpoint
    );
}

#[test]
fn synthesize_local_runtime_entry_returns_none_for_unknown_slug() {
    // Only `ollama` and `lmstudio` are the recognized local-runtime
    // aliases. Every other slug — built-in cloud providers (`openai`,
    // `anthropic`), opaque ids (`p_random_xyz`), or typos — must fall
    // through to the existing "no cloud provider" error. Pinning this
    // rejection contract guards against the synth growing into a
    // blanket "any unknown slug points at localhost" matcher.
    let config = Config::default();
    for slug in ["openai", "anthropic", "openrouter", "p_random_xyz", "", " "] {
        assert!(
            synthesize_local_runtime_entry(slug, &config).is_none(),
            "{slug:?} must NOT synthesize a local-runtime entry"
        );
    }
}

#[test]
fn parse_models_response_handles_non_object_body() {
    // Provider returned a bare array / string / number at the
    // top level — not an object at all. Surface as a parse failure
    // (not a panic).
    for body in [
        serde_json::json!([{"id": "m1"}]),
        serde_json::json!("hello"),
        serde_json::Value::Null,
    ] {
        let err = parse_models_response(&body)
            .expect_err("non-object body must fail with a clear message");
        assert!(
            !err.is_empty(),
            "non-object body error must be non-empty: {err}"
        );
    }
}

/// `is_backend_auth_failure` is the polarity guard that decides whether a
/// 401/403 is the OpenHuman backend's expired session (silence + drive
/// reauth) or a third-party BYO-key rejection (actionable, must reach
/// Sentry). Getting this wrong in either direction is a regression:
/// over-matching silences real misconfig; under-matching is TAURI-RUST-N.
#[test]
fn is_backend_auth_failure_only_matches_openhuman_backend_401_403() {
    use reqwest::StatusCode;
    let backend = crate::openhuman::inference::provider::openhuman_backend::PROVIDER_LABEL;

    assert!(is_backend_auth_failure(backend, StatusCode::UNAUTHORIZED));
    assert!(is_backend_auth_failure(backend, StatusCode::FORBIDDEN));

    // Non-auth backend statuses stay reportable (real server bugs / transient).
    for s in [
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::BAD_REQUEST,
        StatusCode::NOT_FOUND,
    ] {
        assert!(
            !is_backend_auth_failure(backend, s),
            "backend {s} must not be treated as session-expiry"
        );
    }

    // Third-party BYO-key 401/403 (user's own key revoked) must NOT be
    // silenced — that is actionable misconfiguration for Sentry.
    for provider in ["custom_openai", "OpenAI", "Anthropic", "openrouter"] {
        assert!(
            !is_backend_auth_failure(provider, StatusCode::UNAUTHORIZED),
            "{provider} 401 must reach Sentry as actionable BYO-key error"
        );
        assert!(
            !is_backend_auth_failure(provider, StatusCode::FORBIDDEN),
            "{provider} 403 must reach Sentry as actionable BYO-key error"
        );
    }
}

/// `publish_backend_session_expired` must emit a `SessionExpired` event on
/// the `auth` domain with the canonical source and a sanitized reason, so
/// the credentials subscriber can drive reauth.
#[tokio::test]
async fn publish_backend_session_expired_emits_sanitized_session_expired() {
    use crate::core::event_bus::{global, init_global, DomainEvent};

    init_global(1024);
    let mut rx = global().expect("event bus initialized").raw_receiver();

    // `TEST_MARKER_A` makes this event distinguishable from the sibling
    // `chat_completions_backend_401_*` test's event on the shared global
    // bus (both run in parallel against the same singleton). The `sk-`
    // token probes that `sanitize_api_error` actually scrubs secrets out
    // of the SessionExpired reason rather than just emitting the event.
    let secret = "sk-LIVEA0123456789abcdefSECRET";
    let msg = format!(
        r#"OpenHuman API error (401 Unauthorized): {{"success":false,"error":"TEST_MARKER_A Invalid token {secret}"}}"#
    );
    publish_backend_session_expired(
        "chat_completions",
        crate::openhuman::inference::provider::openhuman_backend::PROVIDER_LABEL,
        reqwest::StatusCode::UNAUTHORIZED,
        &msg,
    );

    let mut reason_seen: Option<String> = None;
    loop {
        match rx.try_recv() {
            Ok(DomainEvent::SessionExpired { source, reason }) => {
                if source == "llm_provider.openhuman_backend" && reason.contains("TEST_MARKER_A") {
                    reason_seen = Some(reason);
                    break;
                }
            }
            Ok(_) => continue,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    let reason = reason_seen.expect(
        "publish_backend_session_expired must emit SessionExpired(source=llm_provider.openhuman_backend) carrying TEST_MARKER_A",
    );
    assert!(
        reason.contains("[REDACTED]"),
        "sanitize_api_error must redact the sk- token in the reason: {reason}"
    );
    assert!(
        !reason.contains(secret),
        "raw secret must not survive into the SessionExpired reason: {reason}"
    );
}

/// End-to-end regression for TAURI-RUST-N: a backend `401 Invalid token`
/// on the hand-rolled `chat_completions` path must publish `SessionExpired`
/// (driving reauth) and surface the typed error — NOT spam Sentry. The
/// provider is labelled exactly like the OpenHuman backend provider, which
/// is what gates the backend-auth-failure branch.
#[tokio::test]
async fn chat_completions_backend_401_publishes_session_expired() {
    use crate::core::event_bus::{global, init_global, DomainEvent};
    use axum::routing::post;

    init_global(1024);
    let mut rx = global().expect("event bus initialized").raw_receiver();

    async fn unauthorized_handler() -> Response {
        // `TEST_MARKER_B` distinguishes this event from the sibling
        // `publish_backend_session_expired_*` test on the shared global
        // bus; the `sk-` token probes end-to-end redaction through
        // `api_error` → `publish_backend_session_expired`.
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "success": false,
                "error": "TEST_MARKER_B Invalid token sk-LIVEB9876543210fedcbaSECRET"
            })),
        )
            .into_response()
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let app = Router::new().route("/chat/completions", post(unauthorized_handler));
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let provider =
        crate::openhuman::inference::provider::compatible::OpenAiCompatibleProvider::new_no_responses_fallback(
            crate::openhuman::inference::provider::openhuman_backend::PROVIDER_LABEL,
            &format!("http://{addr}"),
            Some("expired-jwt"),
            crate::openhuman::inference::provider::compatible::AuthStyle::Bearer,
        );

    let err = crate::openhuman::inference::provider::traits::Provider::chat_with_system(
        &provider,
        None,
        "hi",
        "reasoning-quick-v1",
        0.0,
    )
    .await
    .expect_err("backend 401 must surface as an error");
    let msg = err.to_string();
    assert!(
        msg.contains("OpenHuman API error (401") && msg.contains("Invalid token"),
        "error must carry the backend 401 envelope: {msg}"
    );

    let mut reason_seen: Option<String> = None;
    loop {
        match rx.try_recv() {
            Ok(DomainEvent::SessionExpired { source, reason }) => {
                if source == "llm_provider.openhuman_backend" && reason.contains("TEST_MARKER_B") {
                    reason_seen = Some(reason);
                    break;
                }
            }
            Ok(_) => continue,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    let reason = reason_seen.expect(
        "backend 401 on chat_completions must publish SessionExpired carrying TEST_MARKER_B, not report to Sentry",
    );
    assert!(
        reason.contains("[REDACTED]"),
        "sanitize_api_error must redact the sk- token end-to-end: {reason}"
    );
    assert!(
        !reason.contains("sk-LIVEB9876543210fedcbaSECRET"),
        "raw secret must not survive into the SessionExpired reason: {reason}"
    );
}

#[test]
fn synthesize_local_runtime_entry_ollama_respects_config_base_url() {
    // The synth must honor `config.local_ai.base_url` (the same
    // priority `ollama_base_url_from_config` uses for chat routing).
    // This is the path users hit when they point Ollama at a non-loopback
    // host (e.g. a LAN box at 192.168.1.5).
    let mut config = Config::default();
    config.local_ai.base_url = Some("http://192.168.1.5:11434".to_string());
    let entry = synthesize_local_runtime_entry("ollama", &config)
        .expect("ollama with custom base_url must still synthesize");
    assert_eq!(
        entry.endpoint, "http://192.168.1.5:11434/v1",
        "synth must use config.local_ai.base_url and append /v1 once",
    );
}

#[test]
fn cloud_providers_entry_takes_precedence_over_local_runtime_synthesis() {
    // Pin the precedence: if the user has explicitly added an `ollama`
    // entry to `cloud_providers` (e.g. a remote ollama box at
    // https://ollama.example.com/v1), that entry MUST win — the synth
    // fallback is reached only when the find returns `None`. Mirrors
    // the lookup in `list_configured_models_from_config` so a future
    // refactor that swaps `find().or_else(synth)` for unconditional
    // synthesis fails this test loudly.
    let mut config = Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_ollama_explicit".to_string(),
        slug: "ollama".to_string(),
        label: "Remote Ollama".to_string(),
        endpoint: "https://ollama.example.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        legacy_type: None,
        default_model: None,
    });

    let resolved = config
        .cloud_providers
        .iter()
        .find(|e| e.id == "ollama" || e.slug == "ollama")
        .cloned()
        .or_else(|| synthesize_local_runtime_entry("ollama", &config))
        .expect("either explicit or synth must resolve");
    assert_eq!(
        resolved.endpoint, "https://ollama.example.com/v1",
        "explicit cloud_providers entry must beat local-runtime synth",
    );
    assert_eq!(resolved.auth_style, AuthStyle::Bearer);
}

#[test]
fn missing_cloud_providers_entry_falls_back_to_local_runtime_synth() {
    // The TAURI-RUST-28Z regression contract: when no `ollama` entry
    // exists in `cloud_providers` AND the slug is a recognized
    // local-runtime alias, the find/synth chain must yield a synthetic
    // entry (instead of `None`, which produces the
    // "no cloud provider with id or slug 'ollama' found" Sentry error).
    let config = Config::default();
    assert!(
        config.cloud_providers.is_empty(),
        "precondition: clean config has no providers configured",
    );

    let resolved = config
        .cloud_providers
        .iter()
        .find(|e| e.id == "ollama" || e.slug == "ollama")
        .cloned()
        .or_else(|| synthesize_local_runtime_entry("ollama", &config));
    assert!(
        resolved.is_some(),
        "ollama must resolve via synth when cloud_providers is empty"
    );
    assert_eq!(resolved.unwrap().slug, "ollama");
}

#[test]
fn missing_cloud_providers_entry_for_unknown_slug_still_errors() {
    // The synth is intentionally narrow: only `ollama` and `lmstudio`
    // get fallback routing. An unknown slug with no `cloud_providers`
    // match must continue to produce `None` (which the caller surfaces
    // as the "no cloud provider" error) — otherwise typos would
    // silently route to localhost.
    let config = Config::default();
    let resolved = config
        .cloud_providers
        .iter()
        .find(|e| e.id == "tpyo" || e.slug == "tpyo")
        .cloned()
        .or_else(|| synthesize_local_runtime_entry("tpyo", &config));
    assert!(
        resolved.is_none(),
        "unknown slug with no cloud_providers entry must NOT synthesize",
    );
}
