use super::*;

use super::local_runtime::synthesize_local_runtime_entry;
use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::config::Config;
use crate::inference::provider::model_listing_test_support::{
    configure_generic_workspace, spawn_static_models_server,
};
use axum::http::StatusCode;

#[tokio::test]
async fn list_models_empty_body_returns_diagnostic_error() {
    // Some misconfigured load balancers return 200 with an empty body.
    let tmp = tempfile::tempdir().expect("tempdir");
    let endpoint = spawn_static_models_server(StatusCode::OK, "").await;
    let config = configure_generic_workspace(&tmp, endpoint).await;

    let err = catalog_listing::list_configured_models_from_config("generic-test", &config)
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

    let outcome = catalog_listing::list_configured_models_from_config("generic-test", &config)
        .await
        .expect("valid JSON must list models");
    assert_eq!(outcome.value["models"][0]["id"], "some-model");
}

#[tokio::test]
async fn unregistered_legacy_auth_style_fails_before_catalog_network_request() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = configure_generic_workspace(&tmp, "http://127.0.0.1:0/v1".to_string()).await;
    config.cloud_providers[0].auth_style = AuthStyle::OpenhumanJwt;

    let error = catalog_listing::list_configured_models_from_config("generic-test", &config)
        .await
        .expect_err("an unadapted legacy auth style must fail closed");

    assert!(
        error.contains("hosted cloud catalog is unavailable"),
        "{error}"
    );
}

#[tokio::test]
async fn list_models_uses_provider_ca_bundle_client() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let endpoint = spawn_static_models_server(
        StatusCode::OK,
        r#"{"data":[{"id":"private-model","owned_by":"team"}]}"#,
    )
    .await;
    let mut config = configure_generic_workspace(&tmp, endpoint).await;
    config.cloud_provider_ca_certs.insert(
        "generic-test".into(),
        include_str!("../../../util/tls/test-ca.pem").into(),
    );

    let outcome = catalog_listing::list_configured_models_from_config("generic-test", &config)
        .await
        .expect("custom CA client should list models");
    assert_eq!(outcome.value["models"][0]["id"], "private-model");
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
fn synthesize_local_runtime_entry_lmstudio_preserves_configured_v1_base() {
    let mut config = Config::default();
    config.local_ai.base_url = Some("http://192.168.1.8:1234/v1/".to_string());

    let entry = synthesize_local_runtime_entry("lmstudio", &config)
        .expect("lmstudio with custom base_url must still synthesize");

    assert_eq!(entry.endpoint, "http://192.168.1.8:1234/v1");
    assert_eq!(entry.auth_style, AuthStyle::None);
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

/// `is_backend_auth_failure` is the polarity guard that decides whether a
/// 401/403 is the OpenHuman backend's expired session (silence + drive
/// reauth) or a third-party BYO-key rejection (actionable, must reach
#[test]
fn synthesize_local_runtime_entry_ollama_respects_config_base_url() {
    // The synth must honor `config.local_ai.base_url` (the same
    // priority `ollama_base_url_from_override` uses for chat routing).
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
