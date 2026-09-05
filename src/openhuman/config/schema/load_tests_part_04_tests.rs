use super::*;

#[tokio::test]
async fn load_or_init_recovers_from_non_utf8_using_valid_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let config_path = root.join("config.toml");
    let backup_path = root.join("config.toml.bak");

    let binary_bytes: Vec<u8> = vec![0xff, 0xfe, 0x00, 0x01, 0x02];
    write_binary(&config_path, &binary_bytes).await;
    // Write a valid backup.
    write_file(
        &backup_path,
        r#"default_model = "backup-recovery-test"
default_temperature = 0.7
"#,
    )
    .await;

    let config = load_or_init_for_workspace(root).await;

    assert_eq!(
        config.default_model.as_deref(),
        Some("backup-recovery-test"),
        "must recover model from backup when config has non-UTF-8 content"
    );

    // The binary file should have been renamed.
    let dir = std::fs::read_dir(root).unwrap();
    let mut found_corrupted = false;
    for entry in dir {
        let name = entry.unwrap().file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("config.corrupted.") {
            found_corrupted = true;
            break;
        }
    }
    assert!(
        found_corrupted,
        "non-UTF-8 config must be renamed to config.corrupted.<ts>"
    );
}

#[tokio::test]
async fn load_or_init_non_utf8_falls_back_to_defaults_when_backup_also_non_utf8() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let config_path = root.join("config.toml");
    let backup_path = root.join("config.toml.bak");

    let binary_bytes: Vec<u8> = vec![0xff, 0xfe, 0x00, 0x01, 0x02];
    write_binary(&config_path, &binary_bytes).await;
    write_binary(&backup_path, &binary_bytes).await;

    let config = load_or_init_for_workspace(root).await;

    assert_eq!(
        config.default_model.as_deref(),
        Some(crate::openhuman::config::schema::DEFAULT_MODEL),
        "must fall back to defaults when both config and backup have non-UTF-8 content"
    );

    // The primary should be renamed.
    let dir = std::fs::read_dir(root).unwrap();
    let mut found_corrupted = false;
    for entry in dir {
        let name = entry.unwrap().file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("config.corrupted.") {
            found_corrupted = true;
            break;
        }
    }
    assert!(
        found_corrupted,
        "non-UTF-8 config must be renamed to config.corrupted.<ts>"
    );
}

#[tokio::test]
async fn load_or_init_preserves_backup_when_config_is_non_utf8() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let config_path = root.join("config.toml");
    let backup_path = root.join("config.toml.bak");

    let binary_bytes: Vec<u8> = vec![0xff, 0xfe, 0x00, 0x01, 0x02];
    write_binary(&config_path, &binary_bytes).await;
    write_file(
        &backup_path,
        r#"default_model = "preserve-backup-test"
default_temperature = 0.7
"#,
    )
    .await;

    let _config = load_or_init_for_workspace(root).await;

    // The .bak file must NOT be renamed or deleted.
    assert!(
        tokio::fs::try_exists(&backup_path).await.unwrap(),
        ".bak file must be preserved when recovering from non-UTF-8 config"
    );
    let bak_contents = tokio::fs::read_to_string(&backup_path).await.unwrap();
    assert!(
        bak_contents.contains("preserve-backup-test"),
        "backup content must be preserved: {bak_contents}"
    );
}

#[tokio::test]
async fn load_from_config_path_sets_recovery_flag_on_non_utf8() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let workspace = root.join("workspace");

    // The snapshot-reload path (`load_from_config_path`) must set the per-load
    // recovery flag on the returned `Config`. This test asserts only that flag;
    // the surfacing itself is wired one layer up in `app_state_snapshot`, which
    // latches this flag on every poll (see `desktop::app_state::recovery_signal`
    // and its `latch_from_config` call in `snapshot`), so a long-lived runtime
    // that reloads a since-corrupted config does surface the notice (#5167).
    let config_path = root.join("config.toml");
    let binary_bytes: Vec<u8> = vec![0xff, 0xfe, 0x00, 0x01, 0x02];
    write_binary(&config_path, &binary_bytes).await;

    let config = Config::load_from_config_path(&config_path, &workspace)
        .await
        .expect("load_from_config_path must recover, not error");

    assert!(
        config.recovered_from_corruption,
        "load_from_config_path must set the recovery flag on non-UTF-8 content"
    );
}

#[test]
fn redact_url_strips_basic_auth_and_query() {
    let out = redact_url_for_log(
        "https://user:token@api.example.com/v1/chat/completions?api_key=sk-x&debug=1",
    );
    assert!(!out.contains("token"), "got: {out}");
    assert!(!out.contains("sk-x"), "got: {out}");
    assert!(out.starts_with("https://api.example.com"), "got: {out}");
}

#[test]
fn redact_url_handles_plain_url() {
    let out = redact_url_for_log("https://api.openai.com/v1/chat/completions");
    assert_eq!(out, "https://api.openai.com/v1/chat/completions");
}

#[test]
fn redact_url_fallback_masks_userinfo_when_unparseable() {
    let out = redact_url_for_log("not-a-scheme://user:secret@host/path?token=1");
    assert!(!out.contains("secret"), "got: {out}");
    assert!(!out.contains("token=1"), "got: {out}");
}

#[test]
fn migrate_legacy_inference_url_moves_external_chat_completions() {
    let mut cfg = Config::default();
    cfg.api_url = Some("https://api.openai.com/v1/chat/completions".to_string());
    cfg.inference_url = None;
    migrate_legacy_inference_url(&mut cfg);
    assert_eq!(cfg.api_url, None);
    assert_eq!(
        cfg.inference_url.as_deref(),
        Some("https://api.openai.com/v1/chat/completions")
    );
}

#[test]
fn migrate_legacy_inference_url_clears_openhuman_backend_form() {
    let mut cfg = Config::default();
    cfg.api_url = Some("https://api.tinyhumans.ai/openai/v1/chat/completions".to_string());
    cfg.inference_url = None;
    migrate_legacy_inference_url(&mut cfg);
    // The OpenHuman host is the default backend — both fields end up None so
    // inference flows through the derived default `{backend}/openai/v1/...`.
    assert_eq!(cfg.api_url, None);
    assert_eq!(cfg.inference_url, None);
}

#[test]
fn migrate_legacy_inference_url_is_noop_when_inference_url_set() {
    let mut cfg = Config::default();
    cfg.api_url = Some("https://api.openai.com/v1/chat/completions".to_string());
    cfg.inference_url = Some("https://existing.example/v1/chat/completions".to_string());
    migrate_legacy_inference_url(&mut cfg);
    // Existing inference_url wins — api_url is left alone.
    assert_eq!(
        cfg.api_url.as_deref(),
        Some("https://api.openai.com/v1/chat/completions")
    );
    assert_eq!(
        cfg.inference_url.as_deref(),
        Some("https://existing.example/v1/chat/completions")
    );
}

#[test]
fn migrate_cloud_provider_slugs_routes_cloud_to_legacy_custom_when_primary_is_openhuman() {
    let mut cfg = Config::default();
    cfg.inference_url = Some("https://api.example.com/v1".into());
    cfg.primary_cloud = Some("p_oh".into());
    cfg.memory_provider = Some("cloud".into());
    cfg.reasoning_provider = Some("openhuman".into());
    cfg.cloud_providers = vec![
        crate::openhuman::config::schema::CloudProviderCreds {
            id: "p_oh".into(),
            slug: "openhuman".into(),
            label: "OpenHuman".into(),
            endpoint: "https://api.openhuman.ai/v1".into(),
            auth_style: crate::openhuman::config::schema::AuthStyle::OpenhumanJwt,
            ..Default::default()
        },
        crate::openhuman::config::schema::CloudProviderCreds {
            id: "p_custom".into(),
            slug: "custom".into(),
            label: "Custom".into(),
            endpoint: "https://api.example.com/v1/".into(),
            auth_style: crate::openhuman::config::schema::AuthStyle::Bearer,
            default_model: Some("gpt-4o-mini".into()),
            ..Default::default()
        },
    ];

    migrate_cloud_provider_slugs(&mut cfg);

    assert_eq!(cfg.memory_provider.as_deref(), Some("custom:"));
    assert_eq!(
        cfg.reasoning_provider.as_deref(),
        Some("openhuman"),
        "explicit OpenHuman routing must stay explicit"
    );
}

#[test]
fn migrate_cloud_provider_slugs_keeps_cloud_on_openhuman_without_legacy_custom() {
    let mut cfg = Config::default();
    cfg.primary_cloud = Some("p_oh".into());
    cfg.memory_provider = Some("cloud".into());
    cfg.cloud_providers = vec![crate::openhuman::config::schema::CloudProviderCreds {
        id: "p_oh".into(),
        slug: "openhuman".into(),
        label: "OpenHuman".into(),
        endpoint: "https://api.tinyhumans.ai/v1".into(),
        auth_style: crate::openhuman::config::schema::AuthStyle::OpenhumanJwt,
        ..Default::default()
    }];

    migrate_cloud_provider_slugs(&mut cfg);

    assert_eq!(cfg.memory_provider.as_deref(), Some("openhuman"));
}

#[test]
fn migrate_cloud_provider_slugs_does_not_pick_unmatched_custom_provider() {
    let mut cfg = Config::default();
    cfg.inference_url = Some("https://api.example.com/v1".into());
    cfg.primary_cloud = Some("p_oh".into());
    cfg.memory_provider = Some("cloud".into());
    cfg.cloud_providers = vec![
        crate::openhuman::config::schema::CloudProviderCreds {
            id: "p_oh".into(),
            slug: "openhuman".into(),
            label: "OpenHuman".into(),
            endpoint: "https://api.openhuman.ai/v1".into(),
            auth_style: crate::openhuman::config::schema::AuthStyle::OpenhumanJwt,
            ..Default::default()
        },
        crate::openhuman::config::schema::CloudProviderCreds {
            id: "p_other".into(),
            slug: "other".into(),
            label: "Other".into(),
            endpoint: "https://other.example.com/v1".into(),
            auth_style: crate::openhuman::config::schema::AuthStyle::Bearer,
            ..Default::default()
        },
    ];

    migrate_cloud_provider_slugs(&mut cfg);

    assert_eq!(cfg.memory_provider.as_deref(), Some("openhuman"));
}

/// Regression test for #1900: secrets are encrypted on save and decrypted on load.
///
/// Verifies that:
/// 1. Channel tokens are NOT stored in plaintext on disk
/// 2. The backup file (.bak) is encrypted even when overwriting a plaintext config
/// 3. Loading the config back decrypts secrets correctly
#[tokio::test]
async fn config_secrets_encrypted_on_save_decrypted_on_load() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let known_secret = "my-telegram-bot-token-abc123";

    // ── Phase 1: Simulate a pre-upgrade plaintext config on disk ──────
    // Write a raw TOML file containing the secret in plaintext, just like
    // a user who upgraded from a build before encryption was wired in.
    // save() requires the workspace dir to exist, so create it first.
    let workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).unwrap();

    let plaintext_toml = format!(
        r#"[channels_config.telegram]
bot_token = "{known_secret}"
allowed_users = ["@admin"]
"#
    );
    std::fs::write(&config_path, plaintext_toml.as_bytes()).unwrap();

    // Build a Config pointing at the existing plaintext file.
    // We set a fresh secret to force a changed value — the save path
    // will encrypt this new value and write it to disk.
    let mut cfg = Config {
        config_path: config_path.clone(),
        workspace_dir,
        ..Default::default()
    };
    cfg.channels_config.telegram = Some(TelegramConfig {
        bot_token: known_secret.to_string(),
        chat_id: None,
        allowed_users: vec!["@admin".to_string()],
        stream_mode: StreamMode::Off,
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });

    // ── Phase 2: Save (encrypts + creates backup from old file) ──────
    cfg.save().await.unwrap();

    // The primary config must NOT contain the plaintext secret.
    let raw_contents = std::fs::read_to_string(&config_path).expect("config.toml should exist");
    assert!(
        !raw_contents.contains(known_secret),
        "SECURITY BUG: secret '{known_secret}' found in plaintext in config.toml!"
    );

    // The backup file is created by copying the old on-disk file BEFORE
    // the atomic replace. Our fix ensures the backup comes from the
    // encrypted bytes, NOT the plaintext original.
    let backup_path = config_path.with_extension("toml.bak");
    assert!(
        backup_path.exists(),
        "config.toml.bak should exist after overwriting an existing config"
    );
    let backup_contents = std::fs::read_to_string(&backup_path).unwrap();
    assert!(
        !backup_contents.contains(known_secret),
        "SECURITY BUG: secret found in plaintext in config.toml.bak!\n\
         Backup contents:\n{backup_contents}"
    );

    // ── Phase 3: Reload — secrets must decrypt back correctly ────────
    let reloaded = load_or_init_for_workspace(tmp.path()).await;
    let reloaded_token = reloaded
        .channels_config
        .telegram
        .as_ref()
        .map(|t| t.bot_token.as_str());
    assert_eq!(
        reloaded_token,
        Some(known_secret),
        "decrypt path broken: reloaded bot_token '{reloaded_token:?}' \
         does not match original '{known_secret}'"
    );
}

#[tokio::test]
async fn youpet_service_token_encrypted_on_save_decrypted_on_load() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).unwrap();

    let known_secret = "youpet-service-token-sensitive";
    let cfg = Config {
        config_path: config_path.clone(),
        workspace_dir,
        secrets: crate::openhuman::config::schema::SecretsConfig { encrypt: true },
        youpet: crate::openhuman::config::schema::YouPetConfig {
            service_token: Some(known_secret.to_string()),
            ..Default::default()
        },
        ..Default::default()
    };

    cfg.save().await.unwrap();

    let raw_contents = std::fs::read_to_string(&config_path).expect("config.toml should exist");
    assert!(!raw_contents.contains(known_secret));
    assert!(raw_contents.contains("service_token = \"enc"));

    let reloaded = load_or_init_for_workspace(tmp.path()).await;
    assert_eq!(reloaded.youpet.service_token.as_deref(), Some(known_secret));
}

/// Regression for keyring-loss scenario: if a channel token was encrypted with
/// a key that is no longer accessible (e.g. keyring reset, machine migration),
/// config load must NOT fail hard. The field should be cleared and a warning
/// logged, so the rest of the app continues to work.
#[tokio::test]
async fn config_load_succeeds_when_decryption_key_inaccessible() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).unwrap();

    // Write a config whose discord.bot_token is encrypted with a key from a
    // *different* workspace so the current SecretStore (keyed to `tmp`) cannot
    // decrypt it. The `enc2:` prefix makes `is_encrypted()` return true.
    // The hex blob is garbage — intentionally undecryptable.
    let stale_ciphertext =
        "enc2:deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let toml_content = format!(
        r#"[secrets]
encrypt = true

[channels_config.discord]
bot_token = "{stale_ciphertext}"
"#
    );
    std::fs::write(&config_path, toml_content.as_bytes()).unwrap();

    // Config load must succeed even though the token cannot be decrypted.
    let reloaded = load_or_init_for_workspace(tmp.path()).await;

    // Discord config should be cleared (None bot_token → channel won't start)
    // rather than crashing the entire config load.
    let discord_token = reloaded
        .channels_config
        .discord
        .as_ref()
        .map(|d| d.bot_token.as_str());
    assert!(
        discord_token.map_or(true, |t| t.is_empty()),
        "Expected discord.bot_token to be cleared after decryption failure, got: {discord_token:?}"
    );
}

/// Backwards-compatibility regression for #1900: a pre-upgrade `config.toml`
/// that contains plaintext secrets (written by a build from before encryption
/// was wired in) must continue to load with `secrets.encrypt = true`. The
/// load path should hand the raw plaintext to channel code rather than
/// erroring or returning a ciphertext placeholder. The next `save()` is what
/// migrates the values to `enc2:` on disk.
#[tokio::test]
async fn plaintext_legacy_config_still_loads_with_encryption_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let known_secret = "legacy-plaintext-bot-token-xyz789";

    let plaintext_toml = format!(
        r#"[secrets]
encrypt = true

[channels_config.telegram]
bot_token = "{known_secret}"
allowed_users = ["@admin"]
"#
    );
    std::fs::write(&config_path, plaintext_toml.as_bytes()).unwrap();

    let reloaded = load_or_init_for_workspace(tmp.path()).await;
    let reloaded_token = reloaded
        .channels_config
        .telegram
        .as_ref()
        .map(|t| t.bot_token.as_str());
    assert_eq!(
        reloaded_token,
        Some(known_secret),
        "backwards-compat broken: legacy plaintext bot_token did not load as cleartext \
         (got {reloaded_token:?})"
    );
}

// ── resolve_action_dir precedence (env > override > default), issue #3240 ──────

#[test]
fn resolve_action_dir_env_beats_override_and_default() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var(ACTION_DIR_ENV_VAR, "/tmp/env-action-dir");
    }
    let over = Some(PathBuf::from("/tmp/override-action-dir"));
    assert_eq!(
        resolve_action_dir(&over),
        PathBuf::from("/tmp/env-action-dir"),
        "env var must win over a persisted override"
    );
    unsafe {
        std::env::remove_var(ACTION_DIR_ENV_VAR);
    }
}

#[test]
fn resolve_action_dir_override_beats_default_when_no_env() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var(ACTION_DIR_ENV_VAR);
    }
    let over = Some(PathBuf::from("/tmp/override-action-dir"));
    assert_eq!(
        resolve_action_dir(&over),
        PathBuf::from("/tmp/override-action-dir"),
        "override must be used when no env var is set"
    );
}

#[test]
fn resolve_action_dir_falls_back_to_default_when_none() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var(ACTION_DIR_ENV_VAR);
    }
    assert_eq!(
        resolve_action_dir(&None),
        default_projects_dir(),
        "no env + no override must fall back to the default projects dir"
    );
}

#[test]
fn resolve_action_dir_blank_env_does_not_pin() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var(ACTION_DIR_ENV_VAR, "   ");
    }
    let over = Some(PathBuf::from("/tmp/override-action-dir"));
    assert_eq!(
        resolve_action_dir(&over),
        PathBuf::from("/tmp/override-action-dir"),
        "blank env var must be ignored so the override still applies"
    );
    unsafe {
        std::env::remove_var(ACTION_DIR_ENV_VAR);
    }
}

#[test]
fn resolve_action_dir_rejects_relative_override() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var(ACTION_DIR_ENV_VAR);
    }
    let over = Some(PathBuf::from("relative/projects"));
    assert_eq!(
        resolve_action_dir(&over),
        default_projects_dir(),
        "relative override must be ignored, falling back to default"
    );
}

#[test]
fn resolve_action_dir_rejects_empty_override() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var(ACTION_DIR_ENV_VAR);
    }
    let over = Some(PathBuf::from(""));
    assert_eq!(
        resolve_action_dir(&over),
        default_projects_dir(),
        "empty override must be ignored, falling back to default"
    );
}
