#[path = "support/env_guard.rs"]
mod env_guard;
use env_guard::EnvVarGuard;
use openhuman_core::config::schema::{Config, StreamMode, TelegramConfig};
use openhuman_core::security::secret_storage;
use std::sync::OnceLock;

async fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

#[tokio::test]
async fn config_secrets_roundtrip_via_keyring_backed_master_key_migration() {
    let _guard = env_lock().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let openhuman_dir = tmp.path().join("user-123");
    let workspace_dir = openhuman_dir.join("workspace");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");

    let _keyring_backend = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let _workspace_override = EnvVarGuard::set("OPENHUMAN_WORKSPACE", &openhuman_dir);
    secret_storage::init_workspace(&workspace_dir);

    let legacy_key_path = openhuman_dir.join(".secret_key");
    std::fs::write(
        &legacy_key_path,
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
    )
    .expect("legacy key file");

    let config_path = openhuman_dir.join("config.toml");
    let config = Config {
        config_path: config_path.clone(),
        workspace_dir: workspace_dir.clone(),
        api_key: Some("sk-direct-secret".into()),
        search: openhuman_core::config::schema::SearchConfig {
            brave: openhuman_core::config::schema::SearchEngineCredentials {
                base_url: None,
                api_key: Some("brave-secret".into()),
            },
            ..Default::default()
        },
        channels_config: openhuman_core::config::schema::ChannelsConfig {
            telegram: Some(TelegramConfig {
                bot_token: "tg-bot-secret".into(),
                chat_id: None,
                allowed_users: vec!["alice".into()],
                stream_mode: StreamMode::default(),
                draft_update_interval_ms: 1000,
                silent_streaming: true,
                mention_only: false,
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    config.save().await.expect("save config");

    assert!(
        !legacy_key_path.exists(),
        "legacy .secret_key should be deleted after verified migration"
    );

    let config_toml = tokio::fs::read_to_string(&config_path)
        .await
        .expect("read config.toml");
    assert!(
        config_toml.contains("enc2:"),
        "config should store ciphertext"
    );
    assert!(
        !config_toml.contains("sk-direct-secret"),
        "config should not contain plaintext api_key"
    );
    assert!(
        !config_toml.contains("brave-secret"),
        "config should not contain plaintext search api key"
    );
    assert!(
        !config_toml.contains("tg-bot-secret"),
        "config should not contain plaintext telegram bot token"
    );

    let keyring_file = workspace_dir.join("dev-keychain.json");
    let keyring_payload = tokio::fs::read_to_string(&keyring_file)
        .await
        .expect("read dev-keychain");
    assert!(
        keyring_payload.contains("user-123:secretstore.master_key"),
        "migrated master key should live in keyring backend payload"
    );

    let loaded = Config::load_or_init().await.expect("reload config");
    assert_eq!(loaded.api_key.as_deref(), Some("sk-direct-secret"));
    assert_eq!(loaded.search.brave.api_key.as_deref(), Some("brave-secret"));
    assert_eq!(
        loaded
            .channels_config
            .telegram
            .as_ref()
            .map(|cfg| cfg.bot_token.as_str()),
        Some("tg-bot-secret")
    );
}
