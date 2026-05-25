use super::*;
use crate::openhuman::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
use crate::openhuman::inference::openai_oauth::{OPENAI_OAUTH_PROFILE_NAME, OPENAI_PROVIDER_KEY};
use axum::{routing::post, Json, Router};
use chrono::{Duration, Utc};
use tempfile::tempdir;

fn disabled_config() -> (Config, tempfile::TempDir) {
    let tmp = tempdir().expect("tempdir");
    let mut config = Config {
        workspace_dir: tmp.path().join("workspace"),
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
async fn inference_embed_reuses_local_ai_disabled_error() {
    let (config, _tmp) = disabled_config();
    let err = inference_embed(&config, &["hello".to_string()])
        .await
        .expect_err("embed should fail");
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

#[tokio::test]
async fn inference_should_react_short_circuits_for_empty_message() {
    let (config, _tmp) = disabled_config();
    let outcome = inference_should_react(&config, "   ", "web")
        .await
        .expect("reaction decision");
    assert!(!outcome.value.should_react);
    assert!(outcome.value.emoji.is_none());
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

    let outcome = inference_get_client_config()
        .await
        .expect("client config snapshot");
    assert!(outcome.value.get("cloud_providers").is_some());
    assert!(outcome.value.get("api_key_set").is_some());
}

#[tokio::test]
async fn inference_apply_preset_rejects_invalid_tier() {
    let (config, _tmp) = disabled_config();
    config.save().await.expect("save config");

    let err = inference_apply_preset("ram_bogus")
        .await
        .expect_err("invalid tier should fail");
    assert!(err.contains("invalid tier"));
}

#[tokio::test]
async fn inference_presets_returns_recommended_tier() {
    let (config, _tmp) = disabled_config();
    config.save().await.expect("save config");

    let outcome = inference_presets().await.expect("presets");
    assert!(outcome.value.get("recommended_tier").is_some());
    assert!(outcome.value.get("presets").is_some());
}

#[tokio::test]
async fn inference_openai_oauth_start_returns_authorize_payload() {
    let (config, _tmp) = disabled_config();

    let outcome = inference_openai_oauth_start(&config)
        .await
        .expect("oauth start");

    assert!(outcome.value["authUrl"]
        .as_str()
        .unwrap()
        .contains("auth.openai.com"));
    assert_eq!(
        outcome.value["redirectUri"].as_str(),
        Some("http://127.0.0.1:1455/auth/callback")
    );
    assert_eq!(outcome.logs, vec!["openai oauth authorize url ready"]);
}

#[tokio::test]
async fn inference_openai_oauth_complete_surfaces_state_errors() {
    let (config, _tmp) = disabled_config();
    let start = inference_openai_oauth_start(&config)
        .await
        .expect("oauth start");
    let state = start.value["state"].as_str().unwrap();
    let callback = format!("http://127.0.0.1:1455/auth/callback?code=fake&state=wrong-{state}");

    let err = inference_openai_oauth_complete(&config, &callback)
        .await
        .expect_err("state mismatch should fail");

    assert!(err.contains("state mismatch"));
}

#[tokio::test]
async fn inference_openai_oauth_status_returns_connected_payload() {
    let (config, tmp) = disabled_config();
    let store = AuthProfilesStore::new(tmp.path(), false);
    store
        .upsert_profile(
            AuthProfile::new_oauth(
                OPENAI_PROVIDER_KEY,
                OPENAI_OAUTH_PROFILE_NAME,
                TokenSet {
                    access_token: "oauth-access".into(),
                    refresh_token: None,
                    id_token: None,
                    expires_at: Some(Utc::now() + Duration::hours(1)),
                    token_type: Some("Bearer".into()),
                    scope: None,
                },
            ),
            true,
        )
        .unwrap();

    let outcome = inference_openai_oauth_status(&config)
        .await
        .expect("oauth status");

    assert_eq!(outcome.value["connected"], true);
    assert_eq!(outcome.value["authMethod"], "oauth");
    assert_eq!(outcome.logs, vec!["openai oauth status"]);
}

#[tokio::test]
async fn inference_openai_oauth_disconnect_returns_removed_flag() {
    let (config, tmp) = disabled_config();
    let store = AuthProfilesStore::new(tmp.path(), false);
    store
        .upsert_profile(
            AuthProfile::new_oauth(
                OPENAI_PROVIDER_KEY,
                OPENAI_OAUTH_PROFILE_NAME,
                TokenSet {
                    access_token: "oauth-access".into(),
                    refresh_token: None,
                    id_token: None,
                    expires_at: None,
                    token_type: Some("Bearer".into()),
                    scope: None,
                },
            ),
            true,
        )
        .unwrap();

    let outcome = inference_openai_oauth_disconnect(&config)
        .await
        .expect("oauth disconnect");

    assert_eq!(outcome.value["disconnected"], true);
    assert_eq!(outcome.logs, vec!["openai oauth disconnected"]);
}
