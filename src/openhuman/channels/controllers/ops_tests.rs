use super::*;
use tempfile::tempdir;

fn isolated_test_config() -> (tempfile::TempDir, Config) {
    let tmp = tempdir().expect("failed to create temp dir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).expect("failed to create workspace dir");
    (tmp, config)
}

#[tokio::test]
async fn list_channels_returns_definitions() {
    let result = list_channels().await.unwrap();
    assert!(result.value.len() >= 2);
    let ids: Vec<&str> = result.value.iter().map(|d| d.id).collect();
    assert!(ids.contains(&"telegram"));
    assert!(ids.contains(&"discord"));
}

#[tokio::test]
async fn describe_known_channel() {
    let result = describe_channel("telegram").await.unwrap();
    assert_eq!(result.value.id, "telegram");
}

#[tokio::test]
async fn describe_unknown_channel_errors() {
    let err = describe_channel("nonexistent").await.unwrap_err();
    assert!(
        err.contains("unknown channel"),
        "expected 'unknown channel' in error, got: {err}"
    );
}

#[tokio::test]
async fn connect_oauth_returns_pending_auth() {
    let config = Config::default();
    let result = connect_channel(
        &config,
        "discord",
        ChannelAuthMode::OAuth,
        serde_json::json!({}),
    )
    .await
    .unwrap();

    assert_eq!(result.value.status, "pending_auth");
    assert_eq!(result.value.auth_action.as_deref(), Some("discord_oauth"));
}

#[tokio::test]
async fn connect_rejects_unknown_channel() {
    let config = Config::default();
    let result = connect_channel(
        &config,
        "nonexistent",
        ChannelAuthMode::BotToken,
        serde_json::json!({}),
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn connect_rejects_missing_required_fields() {
    let config = Config::default();
    let result = connect_channel(
        &config,
        "telegram",
        ChannelAuthMode::BotToken,
        serde_json::json!({}),
    )
    .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("bot_token"));
}

#[tokio::test]
async fn connect_discord_bot_token_persists_runtime_config() {
    let (_tmp, config) = isolated_test_config();
    let result = connect_channel(
        &config,
        "discord",
        ChannelAuthMode::BotToken,
        serde_json::json!({
            "bot_token": "discord-token-123",
            "guild_id": "guild-1",
            "channel_id": "channel-2"
        }),
    )
    .await
    .expect("discord connect should succeed");

    assert_eq!(result.value.status, "connected");
    assert!(result.value.restart_required);

    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("saved config should exist");
    let parsed: toml::Value = toml::from_str(&raw).expect("saved config should parse");
    let discord = parsed
        .get("channels_config")
        .and_then(|v| v.get("discord"))
        .and_then(toml::Value::as_table)
        .expect("channels_config.discord should be persisted");

    assert_eq!(
        discord.get("bot_token").and_then(toml::Value::as_str),
        Some("discord-token-123")
    );
    assert_eq!(
        discord.get("guild_id").and_then(toml::Value::as_str),
        Some("guild-1")
    );
    assert_eq!(
        discord.get("channel_id").and_then(toml::Value::as_str),
        Some("channel-2")
    );
}

#[tokio::test]
async fn disconnect_discord_bot_token_clears_runtime_config() {
    let (_tmp, mut config) = isolated_test_config();
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "discord-token-abc".to_string(),
        guild_id: Some("guild-1".to_string()),
        channel_id: Some("channel-2".to_string()),
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: false,
    });
    config
        .save()
        .await
        .expect("preloaded config should be persisted");

    disconnect_channel(&config, "discord", ChannelAuthMode::BotToken)
        .await
        .expect("discord disconnect should succeed");

    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("saved config should exist");
    let parsed: toml::Value = toml::from_str(&raw).expect("saved config should parse");
    let discord = parsed.get("channels_config").and_then(|v| v.get("discord"));

    assert!(
        discord.is_none(),
        "channels_config.discord should be removed after disconnect"
    );
}

#[tokio::test]
async fn test_channel_validates_fields() {
    let config = Config::default();

    let ok = test_channel(
        &config,
        "telegram",
        ChannelAuthMode::BotToken,
        serde_json::json!({"bot_token": "123:abc"}),
    )
    .await
    .unwrap();
    assert!(ok.value.success);

    let err = test_channel(
        &config,
        "telegram",
        ChannelAuthMode::BotToken,
        serde_json::json!({}),
    )
    .await;
    assert!(err.is_err());
}

// ── parse_allowed_users / credential_provider ─────────────────

#[test]
fn parse_allowed_users_handles_string_csv() {
    let v = serde_json::json!("alice,bob,@carol");
    let out = parse_allowed_users(Some(&v));
    assert_eq!(out, vec!["alice", "bob", "carol"]);
}

#[test]
fn parse_allowed_users_handles_newline_separated_string() {
    let v = serde_json::json!("alice\nbob\r\ncarol");
    let out = parse_allowed_users(Some(&v));
    assert_eq!(out, vec!["alice", "bob", "carol"]);
}

#[test]
fn parse_allowed_users_dedups_case_insensitively() {
    let v = serde_json::json!("Alice,ALICE,alice,@Alice");
    let out = parse_allowed_users(Some(&v));
    assert_eq!(out, vec!["alice"]);
}

#[test]
fn parse_allowed_users_normalises_at_prefix_and_whitespace() {
    let v = serde_json::json!("  @Alice  ");
    let out = parse_allowed_users(Some(&v));
    assert_eq!(out, vec!["alice"]);
}

#[test]
fn parse_allowed_users_rejects_empty_and_at_only() {
    let v = serde_json::json!(",  ,@,@ ,@@@, ,");
    let out = parse_allowed_users(Some(&v));
    // Normalisation: split on `,` / `\n` / `\r`, trim whitespace, strip
    // *all* leading '@' via `trim_start_matches('@')`, then trim again.
    // Every token here reduces to "" at some step, so the whole input
    // produces an empty result.
    let expected: Vec<String> = Vec::new();
    assert_eq!(out, expected);
}

#[test]
fn parse_allowed_users_accepts_array_of_strings() {
    let v = serde_json::json!(["a", "b,c", "@d\ne"]);
    let out = parse_allowed_users(Some(&v));
    for expected in ["a", "b", "c", "d", "e"] {
        assert!(
            out.contains(&expected.to_string()),
            "missing `{expected}` in {out:?}"
        );
    }
}

#[test]
fn parse_allowed_users_returns_empty_for_none_or_non_string_value() {
    assert!(parse_allowed_users(None).is_empty());
    assert!(parse_allowed_users(Some(&serde_json::json!(42))).is_empty());
    assert!(parse_allowed_users(Some(&serde_json::json!({}))).is_empty());
    assert!(parse_allowed_users(Some(&serde_json::Value::Null)).is_empty());
}

#[test]
fn credential_provider_combines_channel_id_and_mode() {
    // Format: `channel:{channel_id}:{mode}` with mode rendered via
    // `ChannelAuthMode`'s Display impl (`bot_token` / `oauth`).
    assert_eq!(
        credential_provider("telegram", ChannelAuthMode::BotToken),
        "channel:telegram:bot_token"
    );
    assert_eq!(
        credential_provider("discord", ChannelAuthMode::OAuth),
        "channel:discord:oauth"
    );
}

// ── connect_channel validation ─────────────────────────────────
// (list_channels / describe_channel catalog coverage lives in the
// earlier `list_channels_returns_definitions`, `describe_known_channel`,
// and `describe_unknown_channel_errors` tests.)

#[tokio::test]
async fn connect_channel_errors_for_unknown_channel() {
    let config = Config::default();
    let err = connect_channel(
        &config,
        "__unknown__",
        ChannelAuthMode::BotToken,
        serde_json::json!({}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("unknown channel"));
}

#[tokio::test]
async fn connect_channel_rejects_non_object_credentials_for_credential_modes() {
    let config = Config::default();
    let err = connect_channel(
        &config,
        "telegram",
        ChannelAuthMode::BotToken,
        serde_json::json!("not an object"),
    )
    .await
    .unwrap_err();
    assert!(err.contains("credentials must be a JSON object"));
}

// ── iMessage channel ───────────────────────────────────────────
#[tokio::test]
async fn connect_imessage_persists_allowed_contacts() {
    let (_tmp, config) = isolated_test_config();
    let result = connect_channel(
        &config,
        "imessage",
        ChannelAuthMode::ManagedDm,
        serde_json::json!({
            "allowed_contacts": "+15551234567, user@icloud.com"
        }),
    )
    .await
    .expect("imessage connect should succeed");
    assert_eq!(result.value.status, "connected");
    assert!(result.value.restart_required);

    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("saved config should exist");
    let parsed: toml::Value = toml::from_str(&raw).expect("saved config should parse");
    let im = parsed
        .get("channels_config")
        .and_then(|v| v.get("imessage"))
        .and_then(toml::Value::as_table)
        .expect("channels_config.imessage should be persisted");
    let contacts: Vec<&str> = im
        .get("allowed_contacts")
        .and_then(toml::Value::as_array)
        .expect("allowed_contacts array")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect();
    assert!(contacts.iter().any(|c| *c == "+15551234567"));
    assert!(contacts.iter().any(|c| *c == "user@icloud.com"));
}

#[tokio::test]
async fn connect_imessage_allows_empty_contacts() {
    let (_tmp, config) = isolated_test_config();
    let result = connect_channel(
        &config,
        "imessage",
        ChannelAuthMode::ManagedDm,
        serde_json::json!({}),
    )
    .await
    .expect("imessage connect with no contacts should succeed");
    assert_eq!(result.value.status, "connected");
}

#[tokio::test]
async fn disconnect_imessage_clears_runtime_config() {
    let (_tmp, mut config) = isolated_test_config();
    config.channels_config.imessage = Some(IMessageConfig {
        allowed_contacts: vec!["+15551234567".to_string()],
    });
    config
        .save()
        .await
        .expect("preloaded config should be persisted");

    disconnect_channel(&config, "imessage", ChannelAuthMode::ManagedDm)
        .await
        .expect("imessage disconnect should succeed");

    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("saved config should exist");
    let parsed: toml::Value = toml::from_str(&raw).expect("saved config should parse");
    let im_entry = parsed
        .get("channels_config")
        .and_then(|v| v.get("imessage"));
    assert!(im_entry.is_none(), "imessage config should be cleared");
}
