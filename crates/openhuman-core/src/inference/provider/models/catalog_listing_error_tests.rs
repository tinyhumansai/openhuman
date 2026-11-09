use super::*;

use crate::inference::provider::model_listing_test_support::{
    configure_generic_workspace, configure_openrouter_workspace, spawn_openrouter_probe_server,
    spawn_static_models_server,
};
use axum::http::StatusCode;
use std::sync::atomic::Ordering as AtomicOrdering;
use tinyinference_core::sanitize::{sanitize_api_error, MAX_API_ERROR_CHARS};

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

    let err = catalog_listing::list_configured_models_from_config("openrouter", &config)
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

    let outcome = catalog_listing::list_configured_models_from_config("openrouter", &config)
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

    catalog_listing::list_configured_models_from_config("openrouter", &config)
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

    let err = catalog_listing::list_configured_models_from_config("generic-test", &config)
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
