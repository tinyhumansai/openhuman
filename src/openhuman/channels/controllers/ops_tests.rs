use super::*;
use crate::openhuman::channels::providers::yuanbao::YuanbaoConfig;
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
        },
        token_count: 12,
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    }
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
