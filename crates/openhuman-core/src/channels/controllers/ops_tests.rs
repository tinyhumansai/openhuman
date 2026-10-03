use super::*;
use crate::channels::email_channel::EmailConfig;
use crate::channels::providers::yuanbao::YuanbaoConfig;
use crate::config::schema::{DiscordConfig, IMessageConfig};
use tempfile::tempdir;

fn isolated_test_config() -> (tempfile::TempDir, Config) {
    let tmp = tempdir().expect("failed to create temp dir");
    let mut config = Config::default();
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

// ── Yuanbao channel credential verification ────────────────────
// Issue: connect_channel for yuanbao previously stored creds and returned
// "connected" without ever calling the upstream sign-token endpoint, so
// random input (e.g. app_key=12) showed as Connected in the UI. The fix
// calls `/api/v5/robotLogic/sign-token` and propagates the API error.

/// Build a Config pre-pointed at a mock `api_domain` so the verification
/// step hits the wiremock server instead of the live prod URL.
fn yuanbao_test_config(mock_uri: &str) -> (tempfile::TempDir, Config) {
    let (tmp, mut config) = isolated_test_config();
    config.channels_config.yuanbao = Some(YuanbaoConfig {
        api_domain: mock_uri.to_string(),
        ..Default::default()
    });
    (tmp, config)
}

#[path = "ops_connect_status_tests.rs"]
mod connect_status_tests;
#[path = "ops_yuanbao_email_tests.rs"]
mod yuanbao_email_tests;
