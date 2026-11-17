#[path = "support/env_guard.rs"]
mod env_guard;
use env_guard::EnvVarGuard;
use openhuman_core::config::schema::{Config, StreamMode, TelegramConfig};
use openhuman_core::security::secret_storage;
use std::sync::OnceLock;

async fn env_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

fn parse_keyring_payload(json: &str) -> serde_json::Value {
    serde_json::from_str(json).expect("parse keyring json")
}

#[tokio::test]
async fn config_secrets_create_master_key_in_keyring_on_fresh_install() {
    let _guard = env_lock_async().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let openhuman_dir = tmp.path().join("fresh-user");
    let workspace_dir = openhuman_dir.join("workspace");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");

    let _keyring_backend = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let _workspace_override = EnvVarGuard::set("OPENHUMAN_WORKSPACE", &openhuman_dir);
    secret_storage::init_workspace(&workspace_dir);

    let legacy_key_path = openhuman_dir.join(".secret_key");
    assert!(
        !legacy_key_path.exists(),
        "fresh install should not start with legacy key file"
    );

    let config_path = openhuman_dir.join("config.toml");
    let backup_path = openhuman_dir.join("config.toml.bak");
    let config = Config {
        config_path: config_path.clone(),
        workspace_dir: workspace_dir.clone(),
        api_key: Some("sk-fresh-secret".into()),
        channels_config: openhuman_core::config::schema::ChannelsConfig {
            telegram: Some(TelegramConfig {
                bot_token: "fresh-tg-secret".into(),
                chat_id: None,
                allowed_users: vec!["bob".into()],
                stream_mode: StreamMode::default(),
                draft_update_interval_ms: 1000,
                silent_streaming: true,
                mention_only: false,
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    config.save().await.expect("first save");
    config.save().await.expect("second save");

    assert!(
        !legacy_key_path.exists(),
        "fresh install should not create a legacy key file"
    );

    let keyring_file = workspace_dir.join("dev-keychain.json");
    let keyring_payload = tokio::fs::read_to_string(&keyring_file)
        .await
        .expect("read dev-keychain");
    let parsed = parse_keyring_payload(&keyring_payload);
    let key_entry = parsed
        .get("fresh-user:secretstore.master_key")
        .and_then(|v| v.as_str())
        .expect("master key entry");
    assert_eq!(key_entry.len(), 64, "master key should be 32 bytes as hex");

    let config_toml = tokio::fs::read_to_string(&config_path)
        .await
        .expect("read config.toml");
    let backup_toml = tokio::fs::read_to_string(&backup_path)
        .await
        .expect("read config.toml.bak");

    for payload in [&config_toml, &backup_toml] {
        assert!(
            payload.contains("enc2:"),
            "persisted config should be encrypted"
        );
        assert!(
            !payload.contains("sk-fresh-secret"),
            "persisted config should not leak plaintext api_key"
        );
        assert!(
            !payload.contains("fresh-tg-secret"),
            "persisted config should not leak plaintext bot token"
        );
    }

    let loaded = Config::load_or_init().await.expect("reload config");
    assert_eq!(loaded.api_key.as_deref(), Some("sk-fresh-secret"));
    assert_eq!(
        loaded
            .channels_config
            .telegram
            .as_ref()
            .map(|cfg| cfg.bot_token.as_str()),
        Some("fresh-tg-secret")
    );
}
