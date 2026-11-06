use super::*;

/// Pairing a Telegram identity must persist into the runtime's resolved
/// config file, not a hardcoded legacy `~/.openhuman/config.toml` (#8141).
#[cfg(feature = "channels")]
#[tokio::test]
async fn persist_allowed_identity_writes_the_resolved_config_path() {
    let _env = crate::config::test_env::lock_env_async().await;
    let tmp = tempfile::tempdir().unwrap();

    // Point HOME somewhere else with a stale legacy config that has a
    // telegram section but no paired user: this is the file the old code
    // wrote to, so it must stay untouched.
    let fake_home = tmp.path().join("home");
    let legacy_dir = fake_home.join(".openhuman");
    tokio::fs::create_dir_all(&legacy_dir).await.unwrap();
    let legacy_config_path = legacy_dir.join("config.toml");
    let legacy_workspace_dir = legacy_dir.join("workspace");
    let mut legacy = Config {
        config_path: legacy_config_path.clone(),
        workspace_dir: legacy_workspace_dir.clone(),
        action_dir: legacy_workspace_dir.clone(),
        ..Config::default()
    };
    legacy.channels_config.telegram = Some(crate::config::TelegramConfig {
        bot_token: String::new(),
        chat_id: None,
        allowed_users: Vec::new(),
        stream_mode: crate::config::StreamMode::Off,
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    legacy.save().await.unwrap();
    let _home = crate::config::test_env::EnvVarGuard::set("HOME", &fake_home);

    // The runtime's resolved config lives at an unrelated path.
    let resolved_dir = tmp.path().join("resolved");
    tokio::fs::create_dir_all(&resolved_dir).await.unwrap();
    let config_path = resolved_dir.join("config.toml");
    let workspace_dir = resolved_dir.join("workspace");
    let mut seed = Config {
        config_path: config_path.clone(),
        workspace_dir: workspace_dir.clone(),
        action_dir: workspace_dir.clone(),
        ..Config::default()
    };
    seed.channels_config.telegram = Some(crate::config::TelegramConfig {
        bot_token: String::new(),
        chat_id: None,
        allowed_users: Vec::new(),
        stream_mode: crate::config::StreamMode::Off,
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    seed.save().await.unwrap();

    ConfigAllowlistStore {
        config: Arc::new(seed),
    }
    .persist_allowed_identity("telegram", "@operator")
    .await
    .unwrap();

    let saved = Config::load_from_config_path(&config_path, &workspace_dir)
        .await
        .unwrap();
    assert_eq!(
        saved
            .channels_config
            .telegram
            .as_ref()
            .unwrap()
            .allowed_users,
        ["operator"]
    );

    let legacy_after = Config::load_from_config_path(&legacy_config_path, &legacy_workspace_dir)
        .await
        .unwrap();
    assert!(
        legacy_after
            .channels_config
            .telegram
            .as_ref()
            .unwrap()
            .allowed_users
            .is_empty(),
        "legacy config must not gain the paired identity"
    );
}
