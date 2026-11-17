use super::*;

fn isolated_config() -> (tempfile::TempDir, Config) {
    let directory = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.modules.enabled = false;
    config.workspace_dir = directory.path().join("workspace");
    config.config_path = directory.path().join("config.toml");
    (directory, config)
}

fn connected(update: ChannelConfigUpdate) -> PreparedChannelConnection {
    PreparedChannelConnection {
        result: ChannelConnectionResult {
            status: "connected".into(),
            restart_required: true,
            auth_action: None,
            message: Some("fixture verified".into()),
        },
        config_update: Some(update),
    }
}

#[tokio::test]
async fn unavailable_module_cannot_store_valid_channel_credentials() {
    let (_directory, config) = isolated_config();
    let error = connect_channel(
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

#[tokio::test]
async fn prepared_update_cannot_write_another_provider_or_credentials() {
    let (_directory, config) = isolated_config();
    let error = persist_prepared_connection(
        &config,
        "telegram",
        ChannelAuthMode::BotToken,
        serde_json::json!({"bot_token":"fixture-token"}),
        connected(ChannelConfigUpdate::IMessage(
            tinychannels_bus::config::IMessageConfig {
                allowed_contacts: vec!["different-provider".into()],
            },
        )),
    )
    .await
    .unwrap_err();
    assert!(error.contains("module execution failed"), "{error}");
    assert!(!config.config_path.exists());
    assert!(!config.workspace_dir.exists());
}

#[tokio::test]
async fn pending_module_auth_action_does_not_touch_credentials_or_configuration() {
    let (_directory, config) = isolated_config();
    let result = persist_prepared_connection(
        &config,
        "discord",
        ChannelAuthMode::OAuth,
        Value::Null,
        PreparedChannelConnection {
            result: ChannelConnectionResult {
                status: "pending_auth".into(),
                restart_required: false,
                auth_action: Some("discord_oauth".into()),
                message: None,
            },
            config_update: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(result.value.auth_action.as_deref(), Some("discord_oauth"));
    assert!(!config.config_path.exists());
    assert!(!config.workspace_dir.exists());
}

#[tokio::test]
async fn prepared_local_imessage_configuration_is_persisted_without_credentials() {
    let (_directory, config) = isolated_config();
    persist_prepared_connection(
        &config,
        "imessage",
        ChannelAuthMode::ManagedDm,
        serde_json::json!({}),
        connected(ChannelConfigUpdate::IMessage(
            tinychannels_bus::config::IMessageConfig {
                allowed_contacts: vec!["alice".into()],
            },
        )),
    )
    .await
    .unwrap();
    let persisted: toml::Value = toml::from_str(
        &tokio::fs::read_to_string(&config.config_path)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        persisted["channels_config"]["imessage"]["allowed_contacts"][0].as_str(),
        Some("alice")
    );
}
