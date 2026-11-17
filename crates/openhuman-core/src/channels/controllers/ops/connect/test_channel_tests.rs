use super::*;

#[tokio::test]
async fn unavailable_module_cannot_report_locally_valid_credentials_as_connected() {
    let temporary = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.modules.enabled = false;
    config.workspace_dir = temporary.path().join("workspace");
    config.config_path = temporary.path().join("config.toml");
    let error = test_channel(
        &config,
        "telegram",
        ChannelAuthMode::BotToken,
        serde_json::json!({"bot_token":"fixture-token"}),
    )
    .await
    .unwrap_err();
    assert!(error.contains("module unavailable"), "{error}");
    assert!(!config.config_path.exists());
    assert!(!config.workspace_dir.exists());
}
