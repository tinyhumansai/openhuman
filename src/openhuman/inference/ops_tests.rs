use super::*;
use tempfile::tempdir;

fn disabled_config() -> (Config, tempfile::TempDir) {
    let tmp = tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config.local_ai.runtime_enabled = false;
    config.local_ai.opt_in_confirmed = false;
    (config, tmp)
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
async fn inference_chat_rejects_empty_messages() {
    let (config, _tmp) = disabled_config();
    let err = inference_chat(&config, vec![], None)
        .await
        .expect_err("chat should fail");
    assert!(err.contains("must not be empty"));
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
