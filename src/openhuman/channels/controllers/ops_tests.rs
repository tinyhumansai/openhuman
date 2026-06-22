use super::*;
use crate::openhuman::channels::email_channel::EmailConfig;
use crate::openhuman::channels::providers::yuanbao::YuanbaoConfig;
use crate::openhuman::config::schema::{
    ChannelsConfig, DingTalkConfig, DiscordConfig, IMessageConfig, IrcConfig, LarkConfig,
    MatrixConfig, MattermostConfig, QQConfig, SignalConfig, SlackConfig, TelegramConfig,
    WhatsAppConfig,
};
use crate::openhuman::memory_store::chunks::store as memory_tree_store;
use crate::openhuman::memory_store::chunks::types::{
    chunk_id, Chunk, Metadata, SourceKind, SourceRef,
};
use chrono::{TimeZone, Utc};
use tempfile::tempdir;

fn isolated_test_config() -> (tempfile::TempDir, Config) {
    let tmp = tempdir().expect("failed to create temp dir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).expect("failed to create workspace dir");
    (tmp, config)
}

fn sample_chat_chunk(source_id: &str, seq: u32) -> Chunk {
    let ts = Utc
        .timestamp_millis_opt(1_700_000_000_000 + i64::from(seq))
        .unwrap();
    Chunk {
        id: chunk_id(SourceKind::Chat, source_id, seq, "channel memory"),
        content: format!("channel memory {source_id} {seq}"),
        metadata: Metadata {
            source_kind: SourceKind::Chat,
            source_id: source_id.to_string(),
            owner: "alice@example.com".to_string(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: vec!["channel".to_string()],
            source_ref: Some(SourceRef::new(format!("discord://{source_id}/{seq}"))),
            path_scope: None,
        },
        token_count: 12,
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    }
}

#[test]
fn channel_config_connected_covers_config_backed_modes() {
    let mut config = Config::default();

    config.channels_config.telegram = Some(TelegramConfig {
        bot_token: "telegram-token".into(),
        chat_id: None,
        allowed_users: vec![],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "discord-token".into(),
        guild_id: None,
        channel_id: None,
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: false,
    });
    config.channels_config.slack = Some(SlackConfig {
        bot_token: "slack-token".into(),
        app_token: None,
        channel_id: None,
        allowed_users: vec![],
    });
    config.channels_config.mattermost = Some(MattermostConfig {
        url: "https://mattermost.example".into(),
        bot_token: "mattermost-token".into(),
        channel_id: None,
        allowed_users: vec![],
        thread_replies: None,
        mention_only: None,
    });
    config.channels_config.imessage = Some(IMessageConfig {
        allowed_contacts: vec![],
    });
    config.channels_config.matrix = Some(MatrixConfig {
        homeserver: "https://matrix.example".into(),
        access_token: "matrix-token".into(),
        user_id: None,
        device_id: None,
        room_id: "!room:matrix.example".into(),
        allowed_users: vec![],
    });
    config.channels_config.signal = Some(SignalConfig {
        http_url: "http://127.0.0.1:8080".into(),
        account: "+15550100".into(),
        group_id: None,
        allowed_from: vec![],
        ignore_attachments: false,
        ignore_stories: false,
    });
    config.channels_config.whatsapp = Some(WhatsAppConfig {
        access_token: Some("whatsapp-token".into()),
        phone_number_id: Some("phone-id".into()),
        verify_token: Some("verify".into()),
        app_secret: None,
        session_path: None,
        pair_phone: None,
        pair_code: None,
        allowed_numbers: vec![],
    });
    let parsed_linq: ChannelsConfig = toml::from_str(
        r#"
[linq]
api_token = "linq-token"
from_phone = "+15550101"
"#,
    )
    .expect("linq channel config should parse");
    config.channels_config.linq = parsed_linq.linq;
    config.channels_config.email = Some(EmailConfig {
        imap_host: "imap.example".into(),
        imap_port: 993,
        imap_folder: "INBOX".into(),
        smtp_host: "smtp.example".into(),
        smtp_port: 465,
        smtp_tls: true,
        username: "bot@example.com".into(),
        password: "email-password".into(),
        from_address: "bot@example.com".into(),
        idle_timeout_secs: 1740,
        allowed_senders: vec![],
    });
    config.channels_config.irc = Some(IrcConfig {
        server: "irc.example".into(),
        port: 6697,
        nickname: "openhuman".into(),
        username: None,
        channels: vec!["#ops".into()],
        allowed_users: vec![],
        server_password: None,
        nickserv_password: None,
        sasl_password: None,
        verify_tls: None,
    });
    config.channels_config.lark = Some(LarkConfig {
        app_id: "lark-app".into(),
        app_secret: "lark-secret".into(),
        encrypt_key: None,
        verification_token: None,
        allowed_users: vec![],
        use_feishu: false,
        receive_mode: Default::default(),
        port: None,
    });
    config.channels_config.dingtalk = Some(DingTalkConfig {
        client_id: "dingtalk-client".into(),
        client_secret: "dingtalk-secret".into(),
        allowed_users: vec![],
    });
    config.channels_config.qq = Some(QQConfig {
        app_id: "qq-app".into(),
        app_secret: "qq-secret".into(),
        allowed_users: vec![],
    });
    config.channels_config.yuanbao = Some(YuanbaoConfig::default());

    for (channel, mode) in [
        ("telegram", ChannelAuthMode::BotToken),
        ("discord", ChannelAuthMode::BotToken),
        ("slack", ChannelAuthMode::BotToken),
        ("mattermost", ChannelAuthMode::BotToken),
        ("imessage", ChannelAuthMode::ManagedDm),
        ("matrix", ChannelAuthMode::ApiKey),
        ("signal", ChannelAuthMode::ApiKey),
        ("whatsapp", ChannelAuthMode::ApiKey),
        ("linq", ChannelAuthMode::ApiKey),
        ("email", ChannelAuthMode::ApiKey),
        ("irc", ChannelAuthMode::ApiKey),
        ("lark", ChannelAuthMode::ApiKey),
        ("dingtalk", ChannelAuthMode::ApiKey),
        ("qq", ChannelAuthMode::ApiKey),
        ("yuanbao", ChannelAuthMode::ApiKey),
    ] {
        assert!(
            channel_config_connected(&config, channel, mode),
            "{channel}/{mode:?} should be connected from config"
        );
    }

    assert!(!channel_config_connected(
        &config,
        "unknown",
        ChannelAuthMode::ApiKey
    ));
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

    // bot_token is encrypted on disk (issue #1900)
    let token = discord.get("bot_token").and_then(toml::Value::as_str);
    assert!(
        token.is_some_and(|t| t.starts_with("enc:") || t.starts_with("enc2:")),
        "bot_token should be encrypted on disk, got: {token:?}"
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
async fn connect_telegram_bot_token_persists_chat_id() {
    let (_tmp, config) = isolated_test_config();
    let result = connect_channel(
        &config,
        "telegram",
        ChannelAuthMode::BotToken,
        serde_json::json!({
            "bot_token": "telegram-token-123",
            "chat_id": "  987654  "
        }),
    )
    .await
    .expect("telegram connect should succeed");

    assert_eq!(result.value.status, "connected");
    assert!(result.value.restart_required);

    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("saved config should exist");
    let parsed: toml::Value = toml::from_str(&raw).expect("saved config should parse");
    let telegram = parsed
        .get("channels_config")
        .and_then(|v| v.get("telegram"))
        .and_then(toml::Value::as_table)
        .expect("channels_config.telegram should be persisted");

    // chat_id is trimmed before persistence (mirrors Discord channel_id).
    assert_eq!(
        telegram.get("chat_id").and_then(toml::Value::as_str),
        Some("987654")
    );
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

#[tokio::test]
async fn connect_discord_omitted_allowlist_reuses_existing() {
    // Reconnecting without resending `allowed_users` keeps the saved list — the
    // reconnect-convenience path (#3794 review — Codex P2).
    let (_tmp, mut config) = isolated_test_config();
    seed_discord_with_allowlist(&mut config);
    config.save().await.expect("seed should persist");

    connect_channel(
        &config,
        "discord",
        ChannelAuthMode::BotToken,
        serde_json::json!({ "bot_token": "discord-token-abc" }),
    )
    .await
    .expect("reconnect should succeed");

    assert_eq!(
        reload_discord_allowed_users(&config).await,
        vec!["111".to_string(), "222".to_string()],
        "omitted allowed_users must reuse the previously-saved list"
    );
}

#[tokio::test]
async fn connect_discord_cleared_allowlist_allows_everyone() {
    // Clearing the allowlist in the UI submits an explicit empty value; the
    // backend must honor it (empty ⇒ allow-all) instead of reusing the old list
    // (#3794 review — Codex P2).
    let (_tmp, mut config) = isolated_test_config();
    seed_discord_with_allowlist(&mut config);
    config.save().await.expect("seed should persist");

    connect_channel(
        &config,
        "discord",
        ChannelAuthMode::BotToken,
        serde_json::json!({ "bot_token": "discord-token-abc", "allowed_users": "" }),
    )
    .await
    .expect("reconnect should succeed");

    assert!(
        reload_discord_allowed_users(&config).await.is_empty(),
        "an explicit empty allowed_users must clear the list (allow-all), not reuse it"
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

    disconnect_channel(&config, "discord", ChannelAuthMode::BotToken, false)
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
async fn disconnect_channel_clear_memory_deletes_matching_chat_sources() {
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

    let target_a = sample_chat_chunk("discord:guild-1", 0);
    let target_b = sample_chat_chunk("discord:guild-1:channel-2", 1);
    let unrelated = sample_chat_chunk("telegram:chat-1", 0);
    memory_tree_store::upsert_chunks(&config, &[target_a, target_b, unrelated])
        .expect("chunks should seed");

    let result = disconnect_channel(&config, "discord", ChannelAuthMode::BotToken, true)
        .await
        .expect("discord disconnect should succeed");

    assert_eq!(
        result.value["memory_chunks_deleted"].as_u64(),
        Some(2),
        "disconnect should report deleted memory chunks"
    );
    let remaining = memory_tree_store::list_chunks(
        &config,
        &memory_tree_store::ListChunksQuery {
            source_kind: Some(SourceKind::Chat),
            ..Default::default()
        },
    )
    .expect("chunks should list");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].metadata.source_id, "telegram:chat-1");
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

    disconnect_channel(&config, "imessage", ChannelAuthMode::ManagedDm, false)
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

// ---------------------------------------------------------------------------
// Issue #1149: managed-DM / OAuth channels are stored only in the credential
// layer (`channel:<slug>:<mode>`), not in `channels_config.<slug>`. Both
// `channel_status` and `connected_channel_slugs` must surface them so the
// chat agent stops reporting "Telegram not connected" right after a
// managed-DM link succeeds.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn channel_status_reports_managed_dm_credential_as_connected() {
    let (_tmp, config) = isolated_test_config();

    // Simulate the post-link state: `telegram_login_check` stored a
    // credential marker under `channel:telegram:managed_dm` with no
    // corresponding `channels_config.telegram` block.
    crate::openhuman::credentials::ops::store_provider_credentials(
        &config,
        "channel:telegram:managed_dm",
        None,
        Some("managed".to_string()),
        Some(serde_json::json!({ "linked": true })),
        Some(true),
    )
    .await
    .expect("seed managed-DM credential");

    let result = channel_status(&config, Some("telegram"))
        .await
        .expect("channel_status should succeed");

    let managed_dm = result
        .value
        .iter()
        .find(|e| e.auth_mode == ChannelAuthMode::ManagedDm)
        .expect("managed_dm entry");
    assert!(
        managed_dm.connected,
        "managed-DM credential should report connected: {:?}",
        result.value
    );
    assert!(managed_dm.has_credentials);
}

// ---------------------------------------------------------------------------
// Issue #3712: `channel_status` must reflect the *live* supervised-listener
// health, not just credential/config presence, so the Messaging tab never
// shows a false "Connected" while the listener is actually failing.
// ---------------------------------------------------------------------------

#[test]
fn merge_listener_health_ignores_modes_without_a_listener() {
    // managed-DM and other listener-less modes have no `channel:<id>` health
    // component — presence must pass through untouched and never set an error.
    assert_eq!(
        merge_listener_health(true, false, Some("error"), Some("boom")),
        (true, None)
    );
    assert_eq!(
        merge_listener_health(false, false, None, None),
        (false, None)
    );
}

#[test]
fn merge_listener_health_error_overrides_presence_and_surfaces_reason() {
    // Configured (presence == connected) but the live listener is failing →
    // report disconnected and carry the reason to the UI.
    assert_eq!(
        merge_listener_health(true, true, Some("error"), Some("gateway 4004")),
        (false, Some("gateway 4004".to_string()))
    );
}

#[test]
fn merge_listener_health_ok_confirms_connected() {
    assert_eq!(
        merge_listener_health(true, true, Some("ok"), None),
        (true, None)
    );
}

#[test]
fn merge_listener_health_starting_keeps_presence() {
    // Before the first connect attempt the component is "starting" (or absent):
    // keep the presence-based value so a freshly-configured channel isn't shown
    // as broken prematurely.
    assert_eq!(
        merge_listener_health(true, true, Some("starting"), None),
        (true, None)
    );
    assert_eq!(merge_listener_health(true, true, None, None), (true, None));
}

#[tokio::test]
async fn channel_status_surfaces_live_listener_error() {
    let (_tmp, mut config) = isolated_test_config();

    // Configure a bot_token Discord channel (materialises a runtime listener).
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "tok".to_string(),
        guild_id: None,
        channel_id: None,
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: false,
    });

    // Simulate the supervisor reporting the listener as failed.
    crate::openhuman::health::mark_component_error("channel:discord", "gateway closed (4004)");

    let result = channel_status(&config, Some("discord"))
        .await
        .expect("channel_status should succeed");

    let bot_token = result
        .value
        .iter()
        .find(|e| e.auth_mode == ChannelAuthMode::BotToken)
        .expect("bot_token entry");
    assert!(
        !bot_token.connected,
        "a failing listener must report not-connected: {:?}",
        result.value
    );
    assert_eq!(
        bot_token.error.as_deref(),
        Some("gateway closed (4004)"),
        "the disconnect reason must be surfaced: {:?}",
        result.value
    );

    // Recovery: once the supervisor marks the listener healthy, status flips
    // back to connected with the error cleared.
    crate::openhuman::health::mark_component_ok("channel:discord");
    let recovered = channel_status(&config, Some("discord"))
        .await
        .expect("channel_status should succeed");
    let bot_token = recovered
        .value
        .iter()
        .find(|e| e.auth_mode == ChannelAuthMode::BotToken)
        .expect("bot_token entry");
    assert!(
        bot_token.connected,
        "healthy listener should report connected"
    );
    assert!(bot_token.error.is_none(), "error should clear on recovery");
}

// ---------------------------------------------------------------------------
// Issue #3712: default messaging channel switch (Telegram↔Discord). Setting the
// default must persist to `channels_config.active_channel`; an unknown channel
// must be rejected without clobbering the current value.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn set_default_channel_persists_known_channels() {
    let (_tmp, mut config) = isolated_test_config();
    assert!(config.channels_config.active_channel.is_none());

    set_default_channel(&mut config, "Discord")
        .await
        .expect("set discord");
    assert_eq!(
        config.channels_config.active_channel.as_deref(),
        Some("discord"),
        "channel must be canonicalised to lowercase and persisted"
    );

    set_default_channel(&mut config, "telegram")
        .await
        .expect("set telegram");
    assert_eq!(
        config.channels_config.active_channel.as_deref(),
        Some("telegram")
    );
}

#[tokio::test]
async fn set_default_channel_rejects_unknown_and_empty() {
    let (_tmp, mut config) = isolated_test_config();
    set_default_channel(&mut config, "discord")
        .await
        .expect("seed discord");

    assert!(set_default_channel(&mut config, "myspace")
        .await
        .unwrap_err()
        .contains("unknown channel"),);
    assert!(set_default_channel(&mut config, "   ").await.is_err());

    // A rejected set must not clobber the previously persisted value.
    assert_eq!(
        config.channels_config.active_channel.as_deref(),
        Some("discord")
    );
}

#[test]
fn get_default_channel_defaults_to_web_when_unset() {
    let (_tmp, config) = isolated_test_config();
    let out = get_default_channel(&config).expect("get default");
    assert_eq!(out.value["active_channel"], "web");
}

#[tokio::test]
async fn connected_channel_slugs_merges_credentials_and_config() {
    let (_tmp, mut config) = isolated_test_config();

    // Layer 1: TOML-resident channel (e.g. discord bot_token).
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "tok".to_string(),
        guild_id: None,
        channel_id: None,
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: false,
    });

    // Layer 2: credential-only channel (telegram managed_dm).
    crate::openhuman::credentials::ops::store_provider_credentials(
        &config,
        "channel:telegram:managed_dm",
        None,
        Some("managed".to_string()),
        Some(serde_json::json!({ "linked": true })),
        Some(true),
    )
    .await
    .expect("seed managed-DM credential");

    let slugs = connected_channel_slugs(&config)
        .await
        .expect("connected_channel_slugs should succeed");

    assert!(slugs.contains(&"discord".to_string()), "got {slugs:?}");
    assert!(slugs.contains(&"telegram".to_string()), "got {slugs:?}");
}

#[tokio::test]
async fn connected_channel_slugs_dedupes_when_both_layers_present() {
    let (_tmp, mut config) = isolated_test_config();

    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "tok".to_string(),
        guild_id: None,
        channel_id: None,
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: false,
    });

    // Same slug appears in both layers — should collapse to one entry.
    crate::openhuman::credentials::ops::store_provider_credentials(
        &config,
        "channel:discord:managed_dm",
        None,
        Some("managed".to_string()),
        Some(serde_json::json!({ "linked": true })),
        Some(true),
    )
    .await
    .expect("seed managed-DM credential");

    let slugs = connected_channel_slugs(&config)
        .await
        .expect("connected_channel_slugs should succeed");

    let discord_count = slugs.iter().filter(|s| *s == "discord").count();
    assert_eq!(discord_count, 1, "discord should appear once: {slugs:?}");
}

#[tokio::test]
async fn connected_channel_slugs_empty_when_nothing_configured() {
    let (_tmp, config) = isolated_test_config();
    let slugs = connected_channel_slugs(&config).await.unwrap();
    assert!(
        slugs.is_empty(),
        "fresh config should yield no channels: {slugs:?}"
    );
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

#[tokio::test]
async fn connect_yuanbao_rejects_invalid_credentials() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v5/robotLogic/sign-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 40001,
            "msg": "invalid signature",
        })))
        .mount(&server)
        .await;

    let (_tmp, config) = yuanbao_test_config(&server.uri());
    let err = connect_channel(
        &config,
        "yuanbao",
        ChannelAuthMode::ApiKey,
        serde_json::json!({ "app_key": "12", "app_secret": "12" }),
    )
    .await
    .expect_err("invalid yuanbao credentials should fail");

    assert!(
        err.contains("yuanbao credential verification failed") && err.contains("invalid signature"),
        "expected upstream API msg in error, got: {err}"
    );

    // Nothing should be persisted on failure: no TOML write, no credential row.
    let raw = tokio::fs::read_to_string(&config.config_path).await.ok();
    if let Some(text) = raw {
        let parsed: toml::Value = toml::from_str(&text).expect("config parses");
        // The mock api_domain we pre-loaded is allowed to be present, but
        // app_key / app_secret must NOT have been written.
        if let Some(yb) = parsed
            .get("channels_config")
            .and_then(|v| v.get("yuanbao"))
            .and_then(toml::Value::as_table)
        {
            assert_ne!(
                yb.get("app_key").and_then(toml::Value::as_str),
                Some("12"),
                "app_key must not be persisted when verification fails"
            );
        }
    }
}

#[tokio::test]
async fn connect_yuanbao_persists_when_credentials_valid() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v5/robotLogic/sign-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "token": "tok-abc",
                "bot_id": "bot-123",
                "product": "yuanbao",
                "source": "openhuman",
                "duration": 3600,
            }
        })))
        .mount(&server)
        .await;

    let (_tmp, config) = yuanbao_test_config(&server.uri());
    let result = connect_channel(
        &config,
        "yuanbao",
        ChannelAuthMode::ApiKey,
        serde_json::json!({ "app_key": "real-key", "app_secret": "real-secret" }),
    )
    .await
    .expect("valid yuanbao credentials should succeed");

    assert_eq!(result.value.status, "connected");
    assert!(result.value.restart_required);

    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("config should be persisted");
    let parsed: toml::Value = toml::from_str(&raw).expect("config parses");
    let yb = parsed
        .get("channels_config")
        .and_then(|v| v.get("yuanbao"))
        .and_then(toml::Value::as_table)
        .expect("channels_config.yuanbao persisted");
    assert_eq!(
        yb.get("app_key").and_then(toml::Value::as_str),
        Some("real-key")
    );
    // The plaintext `app_secret` must NOT be persisted in TOML — the
    // runtime loads it from the encrypted credentials store instead.
    let toml_secret = yb.get("app_secret").and_then(toml::Value::as_str);
    assert!(
        toml_secret.is_none() || toml_secret == Some(""),
        "app_secret must not be persisted in plaintext TOML, got {toml_secret:?}"
    );

    // The credentials store should contain the secret so startup can recover it.
    let auth = crate::openhuman::credentials::AuthService::from_config(&config);
    let profile = auth
        .get_profile("channel:yuanbao:api_key", None)
        .expect("credentials lookup succeeds")
        .expect("yuanbao credentials stored");
    assert_eq!(
        profile.metadata.get("app_secret").map(String::as_str),
        Some("real-secret")
    );
    assert_eq!(
        profile.metadata.get("app_key").map(String::as_str),
        Some("real-key")
    );
}

#[tokio::test]
async fn connect_yuanbao_verifies_against_overridden_api_domain() {
    // Regression: previously, `verify_yuanbao_credentials` rebuilt the
    // YuanbaoConfig from `config.channels_config.yuanbao` alone and
    // ignored the `api_domain` / `env` / `route_env` overrides on the
    // connect-channel payload. A user submitting `env = "pre"` could
    // pass verification against PROD and then fail after restart when
    // the persisted override took effect.
    //
    // Here the base TOML's `api_domain` deliberately points at an
    // unreachable URL — verification only succeeds if the override
    // supplied in `creds_map` is what actually gets used.
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v5/robotLogic/sign-token"))
        .and(header("X-Route-Env", "canary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "token": "tok-override",
                "bot_id": "bot-1",
                "product": "yuanbao",
                "source": "openhuman",
                "duration": 3600,
            }
        })))
        .mount(&server)
        .await;

    let (_tmp, mut config) = isolated_test_config();
    // Base TOML points to a black hole so the test fails immediately if
    // the verifier ignores the override.
    config.channels_config.yuanbao = Some(YuanbaoConfig {
        api_domain: "http://127.0.0.1:1".to_string(),
        ..Default::default()
    });

    let mock_uri = server.uri();
    let result = connect_channel(
        &config,
        "yuanbao",
        ChannelAuthMode::ApiKey,
        serde_json::json!({
            "app_key": "k",
            "app_secret": "s",
            "api_domain": mock_uri.clone(),
            "route_env": "canary",
        }),
    )
    .await
    .expect("override should be applied before verify");

    assert_eq!(result.value.status, "connected");

    // The override should also have been persisted (single source of
    // truth between verify and persist).
    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("config should be persisted");
    let parsed: toml::Value = toml::from_str(&raw).expect("config parses");
    let yb = parsed
        .get("channels_config")
        .and_then(|v| v.get("yuanbao"))
        .and_then(toml::Value::as_table)
        .expect("channels_config.yuanbao persisted");
    assert_eq!(
        yb.get("api_domain").and_then(toml::Value::as_str),
        Some(mock_uri.as_str()),
    );
    assert_eq!(
        yb.get("route_env").and_then(toml::Value::as_str),
        Some("canary"),
    );
}

#[tokio::test]
async fn connect_yuanbao_persists_env_override() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v5/robotLogic/sign-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "token": "tok-pre",
                "bot_id": "bot-456",
                "product": "yuanbao",
                "source": "openhuman",
                "duration": 3600,
            }
        })))
        .mount(&server)
        .await;

    let (_tmp, config) = yuanbao_test_config(&server.uri());
    connect_channel(
        &config,
        "yuanbao",
        ChannelAuthMode::ApiKey,
        serde_json::json!({
            "app_key": "k",
            "app_secret": "s",
            "env": "pre",
            "route_env": "canary",
        }),
    )
    .await
    .expect("valid yuanbao credentials should succeed");

    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("config should be persisted");
    let parsed: toml::Value = toml::from_str(&raw).expect("config parses");
    let yb = parsed
        .get("channels_config")
        .and_then(|v| v.get("yuanbao"))
        .and_then(toml::Value::as_table)
        .expect("channels_config.yuanbao persisted");
    assert_eq!(yb.get("env").and_then(toml::Value::as_str), Some("pre"));
    assert_eq!(
        yb.get("route_env").and_then(toml::Value::as_str),
        Some("canary")
    );
}
