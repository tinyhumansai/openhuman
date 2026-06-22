use openhuman_core::openhuman::config::schema::{Config, StreamMode, TelegramConfig};
use openhuman_core::openhuman::keyring;
use std::sync::{Mutex, OnceLock};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }

    fn set_str(key: &'static str, value: &'static str) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe {
                std::env::set_var(self.key, value);
            },
            None => unsafe {
                std::env::remove_var(self.key);
            },
        }
    }
}

#[tokio::test]
async fn config_secrets_roundtrip_via_keyring_backed_master_key_migration() {
    let _guard = env_lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let openhuman_dir = tmp.path().join("user-123");
    let workspace_dir = openhuman_dir.join("workspace");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");

    let _keyring_backend = EnvGuard::set_str("OPENHUMAN_KEYRING_BACKEND", "file");
    let _workspace_override = EnvGuard::set("OPENHUMAN_WORKSPACE", &openhuman_dir);
    keyring::init_workspace(&workspace_dir);

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
        search: openhuman_core::openhuman::config::schema::SearchConfig {
            parallel: openhuman_core::openhuman::config::schema::SearchEngineCredentials {
                api_key: Some("parallel-secret".into()),
                ..Default::default()
            },
            ..Default::default()
        },
        channels_config: openhuman_core::openhuman::config::schema::ChannelsConfig {
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
        !config_toml.contains("parallel-secret"),
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
    assert_eq!(
        loaded.search.parallel.api_key.as_deref(),
        Some("parallel-secret")
    );
    assert_eq!(
        loaded
            .channels_config
            .telegram
            .as_ref()
            .map(|cfg| cfg.bot_token.as_str()),
        Some("tg-bot-secret")
    );
}
