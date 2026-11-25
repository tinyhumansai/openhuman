use super::*;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use axum::{routing::post, Json, Router};
use tempfile::tempdir;

#[tokio::test]
async fn inference_list_models_uses_the_dispatch_context_config() {
    use crate::core::runtime::{context::CoreContext, DispatchCtx, DomainSet};
    use crate::inference::provider::model_listing_test_support::{
        configure_generic_workspace, spawn_static_models_server,
    };

    let models_a = r#"{"data":[{"id":"tenant-a-model","owned_by":"fixture"}]}"#;
    let models_b = r#"{"data":[{"id":"tenant-b-model","owned_by":"fixture"}]}"#;
    let endpoint_a = spawn_static_models_server(axum::http::StatusCode::OK, models_a).await;
    let endpoint_b = spawn_static_models_server(axum::http::StatusCode::OK, models_b).await;
    let workspace_a = tempdir().expect("tenant A workspace");
    let workspace_b = tempdir().expect("tenant B workspace");
    let config_a = configure_generic_workspace(&workspace_a, endpoint_a).await;
    let config_b = configure_generic_workspace(&workspace_b, endpoint_b).await;
    let ctx_a = DispatchCtx::new(CoreContext::for_test_with_config(
        DomainSet::full(),
        config_a,
    ));
    let ctx_b = DispatchCtx::new(CoreContext::for_test_with_config(
        DomainSet::full(),
        config_b,
    ));

    let result_a = inference_list_models(&ctx_a, "p_generic_test")
        .await
        .expect("tenant A catalog request succeeds");
    let result_b = inference_list_models(&ctx_b, "p_generic_test")
        .await
        .expect("tenant B catalog request succeeds");

    assert_eq!(result_a.value["models"][0]["id"], "tenant-a-model");
    assert_eq!(result_b.value["models"][0]["id"], "tenant-b-model");
}

fn disabled_config() -> (Config, tempfile::TempDir) {
    let tmp = tempdir().expect("tempdir");
    let mut config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    config.local_ai.runtime_enabled = false;
    config.local_ai.opt_in_confirmed = false;
    (config, tmp)
}

async fn spawn_mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{}", addr.port())
}

#[tokio::test]
async fn inference_status_reports_disabled_state_when_runtime_disabled() {
    let (config, _tmp) = disabled_config();
    let outcome = inference_status(&config).await.expect("status");
    assert!(
        matches!(outcome.value.state.as_str(), "idle" | "disabled"),
        "unexpected state: {}",
        outcome.value.state
    );
}

#[tokio::test]
async fn inference_prompt_reuses_local_ai_disabled_error() {
    let (config, _tmp) = disabled_config();
    let err = inference_prompt(&config, "hello", None, Some(true))
        .await
        .expect_err("prompt should fail");
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn inference_summarize_reuses_local_ai_disabled_error() {
    let (config, _tmp) = disabled_config();
    let err = inference_summarize(&config, "hello", None)
        .await
        .expect_err("summarize should fail");
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn inference_test_provider_model_routes_lmstudio_prefix_through_provider_layer() {
    let (config, _tmp) = disabled_config();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|Json(body): Json<serde_json::Value>| async move {
            assert_eq!(body["model"], "test-model");
            Json(serde_json::json!({
                "choices": [{
                    "message": { "role": "assistant", "content": "LMSTUDIO_PROVIDER_OK" }
                }]
            }))
        }),
    );
    let base = spawn_mock(app).await;
    let mut config = config;
    config.local_ai.base_url = Some(format!("{base}/v1"));

    let outcome =
        inference_test_provider_model(&config, "reasoning", "lmstudio:test-model", "Hello")
            .await
            .expect("lmstudio provider probe");
    assert_eq!(outcome.value.reply, "LMSTUDIO_PROVIDER_OK");
}

#[tokio::test]
async fn inference_test_provider_model_routes_ollama_prefix_through_provider_layer() {
    let (config, _tmp) = disabled_config();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|Json(body): Json<serde_json::Value>| async move {
            assert_eq!(body["model"], "test-model");
            Json(serde_json::json!({
                "choices": [{
                    "message": { "role": "assistant", "content": "OLLAMA_PROVIDER_OK" }
                }]
            }))
        }),
    );
    let base = spawn_mock(app).await;
    let mut config = config;
    config.local_ai.base_url = Some(base);

    let outcome = inference_test_provider_model(&config, "reasoning", "ollama:test-model", "Hello")
        .await
        .expect("ollama provider probe");
    assert_eq!(outcome.value.reply, "OLLAMA_PROVIDER_OK");
}

#[test]
fn inference_test_provider_model_demotes_expected_config_errors() {
    let kind = expected_test_provider_model_error_kind(
        "OpenAI API error (401 Unauthorized): missing API key",
    );

    assert_eq!(
        kind,
        Some(crate::core::observability::ExpectedErrorKind::ApiKeyMissing)
    );
}

#[test]
fn inference_test_provider_model_keeps_unexpected_errors_reportable() {
    assert_eq!(
        expected_test_provider_model_error_kind("provider task panicked while parsing response"),
        None
    );
}

#[tokio::test]
async fn inference_analyze_sentiment_handles_empty_message() {
    let (config, _tmp) = disabled_config();
    let outcome = inference_analyze_sentiment(&config, "   ")
        .await
        .expect("sentiment");
    assert_eq!(outcome.value.valence, "neutral");
}

#[tokio::test]
async fn inference_get_client_config_returns_safe_snapshot() {
    let (config, _tmp) = disabled_config();
    config.save().await.expect("save config");

    let outcome = inference_get_client_config(&DispatchCtx::process_default())
        .await
        .expect("client config snapshot");
    assert!(outcome.value.get("cloud_providers").is_some());
    assert!(outcome.value.get("api_key_set").is_some());
    // #3767: authoritative per-tier credits-gate bypass map is present and, with
    // no BYO provider configured, every tier defaults to false (inference still
    // bills managed credits).
    let credits_bypass = outcome
        .value
        .get("credits_bypass")
        .expect("credits_bypass present");
    assert_eq!(
        credits_bypass.get("chat"),
        Some(&serde_json::Value::Bool(false))
    );
    assert_eq!(
        credits_bypass.get("reasoning"),
        Some(&serde_json::Value::Bool(false))
    );
}

// ── is_unknown_provider_user_config (TAURI-RUST-X) ───────────────────────
//
// `inference_list_models` calls `provider::models::catalog_listing::list_configured_models`,
// which surfaces a `String` error when the user-selected provider id isn't
// registered in the cloud-provider list (e.g. picking "ollama" — a local
// runtime — as a cloud provider). The error string is emitted at
// `crates/openhuman-core/src/inference/provider/models/catalog_listing.rs:54`. Before this fix the emit
// site at `inference/ops.rs:248` escalated every such error to `error!`,
// which sentry-tracing ships to Sentry as `"[inference::ops]
// list_models:error"` — 5740+ events with the underlying error hidden in
// a tracing field where no Sentry classifier can reach it. The helper
// gate keeps the demote anchored so unrelated failures (HTTP / JSON / IO)
// still escalate.

#[test]
fn is_unknown_provider_user_config_matches_canonical_emit_site_string() {
    // Verbatim shape from `provider/models/catalog_listing.rs:54`:
    //   format!("no cloud provider with id or slug '{}' found", provider_id)
    // Latest TAURI-RUST-X event (Sentry id 95) carried provider_id="ollama";
    // every well-formed provider id slug must trigger the demote.
    assert!(is_unknown_provider_user_config(
        "no cloud provider with id or slug 'ollama' found"
    ));
    assert!(is_unknown_provider_user_config(
        "no cloud provider with id or slug 'made-up-custom-id' found"
    ));
    assert!(is_unknown_provider_user_config(
        "no cloud provider with id or slug '' found"
    ));
}

#[test]
fn is_unknown_provider_user_config_rejects_other_list_models_failures() {
    // Defense in depth: the sibling list_models Sentry issues
    // (TAURI-RUST-12 JSON parse, TAURI-RUST-2W HTTP builder, etc.) are
    // real bugs that MUST still escalate to Sentry. The matcher must stay
    // strictly anchored on the "no cloud provider with id or slug" phrase
    // so it can't accidentally silence them.
    for raw in [
        // TAURI-RUST-12 (362 events) — provider/models/catalog_listing.rs JSON decode failure
        "[providers][list_models] failed to parse JSON: error decoding response",
        // TAURI-RUST-2W (100 events) — provider/models/catalog_listing.rs reqwest builder failure
        "[providers][list_models] HTTP request failed: builder error",
        // TAURI-RUST-JP (8 events) — local_ai ollama_admin transport failure
        "[local_ai:ollama_admin] list_models: request send failed",
        // Generic shapes from elsewhere in the call chain
        "request timed out after 30s",
        "permission denied accessing config",
        "no cloud provider configured for slug 'openai' (role 'chat')",
        "",
    ] {
        assert!(
            !is_unknown_provider_user_config(raw),
            "must NOT demote real error: {raw:?}"
        );
    }
}
