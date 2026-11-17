use super::*;
use crate::config::schema::{DiscordConfig, IMessageConfig};
use crate::core::Outcome;
use tempfile::tempdir;
use tinychannels_bus::config::{EmailConfig, YuanbaoConfig};
use tinychannels_bus::controllers::{ChannelAuthMode, ChannelConnectionResult};

fn isolated_test_config() -> (tempfile::TempDir, Config) {
    let tmp = tempdir().expect("failed to create temp dir");
    let mut config = Config::default();
    config.modules.allow_download = false;
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).expect("failed to create workspace dir");
    (tmp, config)
}

/// Read the persisted Discord `allowed_users` array from the saved config.toml.
async fn reload_discord_allowed_users(config: &Config) -> Vec<String> {
    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("saved config should exist");
    let parsed: toml::Value = toml::from_str(&raw).expect("saved config should parse");
    parsed
        .get("channels_config")
        .and_then(|v| v.get("discord"))
        .and_then(|v| v.get("allowed_users"))
        .and_then(toml::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn seed_discord_with_allowlist(config: &mut Config) {
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "discord-token-abc".to_string(),
        guild_id: None,
        channel_id: None,
        allowed_users: vec!["111".to_string(), "222".to_string()],
        listen_to_bots: false,
        mention_only: false,
    });
}

#[path = "ops_connect_status_tests.rs"]
mod connect_status_tests;
#[path = "ops_yuanbao_email_tests.rs"]
mod yuanbao_email_tests;

/// A recorded module result fixture: host tests exercise custody/persistence,
/// while native verification and provider defaults are tested in TinyChannels.
async fn persist_module_fixture(
    config: &Config,
    channel: &str,
    mode: ChannelAuthMode,
    credentials: serde_json::Value,
    update: tinychannels_bus::module_controller::ChannelConfigUpdate,
) -> Result<Outcome<ChannelConnectionResult>, String> {
    connect::persist_prepared_connection(
        config,
        channel,
        mode,
        credentials,
        tinychannels_bus::module_controller::PreparedChannelConnection {
            result: ChannelConnectionResult {
                status: "connected".into(),
                restart_required: true,
                auth_action: None,
                message: Some("verified by module fixture".into()),
            },
            config_update: Some(update),
        },
    )
    .await
}
