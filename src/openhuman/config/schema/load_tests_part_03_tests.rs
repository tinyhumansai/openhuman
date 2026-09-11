use super::*;

#[test]
fn env_overlay_auto_update_rpc_mutations_enabled_parses_bool() {
    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED", "false"),
    );
    assert!(!cfg.update.rpc_mutations_enabled);
}

#[test]
fn env_overlay_empty_lookup_leaves_defaults_intact() {
    // The seam with no env entries should be a no-op on a fresh Config.
    let mut cfg = Config::default();
    let before = (
        cfg.default_model.clone(),
        cfg.default_temperature,
        cfg.runtime.reasoning_enabled,
        cfg.update.enabled,
        cfg.dictation.enabled,
    );
    cfg.apply_env_overlay_with(&HashMapEnv::new());
    let after = (
        cfg.default_model.clone(),
        cfg.default_temperature,
        cfg.runtime.reasoning_enabled,
        cfg.update.enabled,
        cfg.dictation.enabled,
    );
    assert_eq!(before, after);
}

#[test]
fn env_lookup_get_any_preserves_precedence() {
    let env = HashMapEnv::new()
        .with("KEY_A", "first-wins")
        .with("KEY_B", "second")
        .with("KEY_C", "third");
    // Ordered lookup: first hit wins.
    assert_eq!(env.get_any(&["KEY_A", "KEY_B"]), Some("first-wins".into()));
    // Missing first → falls through.
    assert_eq!(
        env.get_any(&["KEY_MISSING", "KEY_B"]),
        Some("second".into())
    );
    // All missing → None.
    assert_eq!(env.get_any(&["KEY_X", "KEY_Y"]), None);
}

// ── resolve_runtime_config_dirs_with ──────────────────────────────────────

#[tokio::test]
async fn resolve_runtime_config_dirs_with_env_workspace_override() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let default_workspace = root.join("workspace");

    // Point OPENHUMAN_WORKSPACE at a custom path via HashMapEnv — no
    // process-env mutation needed.
    let custom_ws = tmp.path().join("custom_ws");
    let env = HashMapEnv::new().with("OPENHUMAN_WORKSPACE", custom_ws.to_str().unwrap());

    let (oh_dir, ws_dir, source) = resolve_runtime_config_dirs_with(root, &default_workspace, &env)
        .await
        .unwrap();

    assert_eq!(source, ConfigResolutionSource::EnvWorkspace);
    // resolve_config_dir_for_workspace: no config.toml and basename ≠
    // "workspace" → oh_dir == custom_ws, ws_dir == custom_ws/workspace.
    assert_eq!(oh_dir, custom_ws);
    assert_eq!(ws_dir, custom_ws.join("workspace"));
}

#[tokio::test]
async fn resolve_runtime_config_dirs_with_empty_env_falls_back_to_default() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let default_workspace = root.join("workspace");

    // Empty env: no OPENHUMAN_WORKSPACE → falls through to the pre-login
    // user directory path (no active_user.toml, no workspace marker).
    let env = HashMapEnv::new();
    let (oh_dir, _ws_dir, source) =
        resolve_runtime_config_dirs_with(root, &default_workspace, &env)
            .await
            .unwrap();

    assert_eq!(source, ConfigResolutionSource::DefaultConfigDir);
    // Should be under the users/pre-login tree, not the bare root.
    assert!(
        oh_dir.starts_with(root.join("users")),
        "expected oh_dir under users/, got {oh_dir:?}"
    );
}

// ── parse_config_with_recovery ─────────────────────────────────

#[tokio::test]
async fn test_corrupt_config_no_backup_falls_back_to_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");

    // Write invalid TOML — no .bak present.
    std::fs::write(&config_path, b"this is [not valid toml !!!").unwrap();

    let (result, was_corrupted) =
        parse_config_with_recovery(&config_path, "this is [not valid toml !!!").await;

    // Must return default config values.
    assert!(
        (result.default_temperature - 0.7).abs() < f64::EPSILON,
        "expected default temperature 0.7, got {}",
        result.default_temperature
    );
    assert!(was_corrupted, "parse failure must report corruption");
}

#[tokio::test]
async fn test_corrupt_config_valid_backup_recovers() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let backup_path = config_path.with_extension("toml.bak");

    // Write invalid primary TOML.
    std::fs::write(&config_path, b"not [ valid toml").unwrap();

    // Write a valid backup with a distinguishable field value.
    let bak_toml = "default_temperature = 1.5\n";
    std::fs::write(&backup_path, bak_toml).unwrap();

    let (result, was_corrupted) =
        parse_config_with_recovery(&config_path, "not [ valid toml").await;

    assert!(
        (result.default_temperature - 1.5).abs() < f64::EPSILON,
        "expected backup temperature 1.5, got {}",
        result.default_temperature
    );
    assert!(was_corrupted, "backup recovery must report corruption");
}

#[tokio::test]
async fn test_corrupt_config_corrupt_backup_falls_back_to_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let backup_path = config_path.with_extension("toml.bak");

    // Both files contain invalid TOML.
    std::fs::write(&config_path, b"invalid primary").unwrap();
    std::fs::write(&backup_path, b"invalid backup").unwrap();

    let (result, was_corrupted) = parse_config_with_recovery(&config_path, "invalid primary").await;

    assert!(
        (result.default_temperature - 0.7).abs() < f64::EPSILON,
        "expected default temperature 0.7 after double-corrupt, got {}",
        result.default_temperature
    );
    assert!(
        was_corrupted,
        "double-corrupt fallback must report corruption"
    );
}

#[test]
fn test_missing_default_temperature_uses_correct_default() {
    // TOML with no `default_temperature` field — serde should apply the
    // `default_temperature_value()` fn (0.7), not the bare Default (0.0).
    let toml_without_temperature = "api_url = \"https://example.com\"\n";
    let config: Config = toml::from_str(toml_without_temperature).unwrap();
    assert!(
        (config.default_temperature - 0.7).abs() < f64::EPSILON,
        "expected serde default 0.7 when field is absent, got {}",
        config.default_temperature
    );
}

#[tokio::test]
async fn test_save_preserves_backup_file() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let backup_path = tmp.path().join("config.toml.bak");

    let config = Config {
        config_path: config_path.clone(),
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        ..Default::default()
    };

    // First save — creates config.toml (no prior file, so no .bak yet).
    config.save().await.unwrap();
    assert!(
        config_path.exists(),
        "config.toml must exist after first save"
    );

    // Second save — had_existing_config=true → .bak is written.
    config.save().await.unwrap();
    assert!(
        backup_path.exists(),
        "config.toml.bak must exist after second save"
    );
}

#[tokio::test]
async fn test_save_then_corrupt_then_recover() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");

    let config = Config {
        config_path: config_path.clone(),
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        default_temperature: 1.3,
        ..Default::default()
    };

    // First save writes config.toml.
    config.save().await.unwrap();
    // Second save copies to .bak and writes new primary.
    config.save().await.unwrap();

    // Verify .bak exists.
    let backup_path = config_path.with_extension("toml.bak");
    assert!(backup_path.exists(), ".bak must exist after save");

    // Now corrupt the primary.
    tokio::fs::write(&config_path, b"totally broken toml [[[")
        .await
        .unwrap();

    // Recovery should use .bak and return the saved temperature.
    let (recovered, was_corrupted) =
        parse_config_with_recovery(&config_path, "totally broken toml [[[").await;
    assert!(
        (recovered.default_temperature - 1.3).abs() < f64::EPSILON,
        "expected recovered temperature 1.3, got {}",
        recovered.default_temperature
    );
    assert!(was_corrupted, "recovery from .bak must report corruption");
}

#[test]
fn apply_env_overrides_commits_side_effects_to_runtime_proxy() {
    use crate::openhuman::config::schema::proxy::{runtime_proxy_config, set_runtime_proxy_config};

    // Hold the env lock so no other test races on proxy-related env vars.
    let _g = env_lock();
    clear_env(&[
        "OPENHUMAN_PROXY_ENABLED",
        "OPENHUMAN_HTTP_PROXY",
        "HTTP_PROXY",
        "OPENHUMAN_HTTPS_PROXY",
        "HTTPS_PROXY",
        "OPENHUMAN_ALL_PROXY",
        "ALL_PROXY",
    ]);

    // Snapshot the global runtime proxy config so we can restore it afterwards
    // and avoid leaking state into other tests.
    let previous_runtime = runtime_proxy_config();

    // Build a config with proxy fields set directly on the struct.
    // We cannot pre-configure via apply_env_overlay_with + a HashMapEnv and
    // then call apply_env_overrides(), because apply_env_overrides() internally
    // re-runs apply_env_overlay_with(&ProcessEnv) which reads the real process
    // environment — overwriting anything set via a HashMapEnv beforehand.
    // Setting fields directly ensures they survive the ProcessEnv overlay
    // (which only writes fields when the corresponding env var is present).
    let mut cfg = Config::default();
    cfg.proxy.http_proxy = Some("http://proxy.test:8080".to_string());
    cfg.proxy.enabled = true;

    // apply_env_overrides commits side effects: it calls set_runtime_proxy_config
    // with the current proxy config after the ProcessEnv overlay.
    cfg.apply_env_overrides();

    // `set_runtime_proxy_config` must have been called: the global should
    // reflect the proxy URL we set on cfg.proxy.
    let runtime = runtime_proxy_config();
    assert!(
        runtime.enabled,
        "runtime proxy must be enabled after apply_env_overrides"
    );
    assert_eq!(
        runtime.http_proxy.as_deref(),
        Some("http://proxy.test:8080"),
        "runtime proxy URL must match the value set on cfg.proxy"
    );

    // Restore the global runtime proxy state so this test doesn't bleed into
    // other tests that inspect runtime_proxy_config().
    set_runtime_proxy_config(previous_runtime);
}

#[tokio::test]
async fn load_or_init_recovers_from_backup_when_config_corrupted() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let config_path = root.join("config.toml");
    let backup_path = root.join("config.toml.bak");

    write_file(&config_path, CORRUPTED_TOML).await;
    write_file(
        &backup_path,
        r#"default_model = "gpt-recovery-test"
default_temperature = 0.7
"#,
    )
    .await;

    let config = load_or_init_for_workspace(root).await;

    assert_eq!(
        config.default_model.as_deref(),
        Some("gpt-recovery-test"),
        "must load values from backup"
    );

    // The recovered config must have been persisted to disk.
    let persisted = tokio::fs::read_to_string(&config_path).await.unwrap();
    assert!(
        persisted.contains("default_model"),
        "recovered config must be written back to config.toml: {persisted}"
    );

    // The .bak must still be intact (save() must NOT have overwritten it
    // with the corrupted primary).
    let bak_contents = tokio::fs::read_to_string(&backup_path).await.unwrap();
    assert!(
        bak_contents.contains("gpt-recovery-test"),
        "backup must not be overwritten by corrupted config during save: {bak_contents}"
    );
}

#[tokio::test]
async fn load_or_init_falls_back_to_defaults_when_backup_also_corrupted() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let config_path = root.join("config.toml");
    let backup_path = root.join("config.toml.bak");

    write_file(&config_path, CORRUPTED_TOML).await;
    write_file(&backup_path, CORRUPTED_TOML).await;

    let config = load_or_init_for_workspace(root).await;

    // Config::default() sets default_model = Some("reasoning-v1").
    assert_eq!(
        config.default_model.as_deref(),
        Some(crate::openhuman::config::schema::DEFAULT_MODEL),
        "must fall back to defaults when backup is also corrupted"
    );

    assert!(tokio::fs::try_exists(&config_path).await.unwrap());

    // The corrupted backup should not be deleted by the recovery flow.
    assert!(
        tokio::fs::try_exists(&backup_path).await.unwrap(),
        ".bak must not be deleted during recovery"
    );

    // The corrupted primary must have been renamed to .corrupted.
    let corrupted_path = root.join("config.toml.corrupted");
    assert!(
        tokio::fs::try_exists(&corrupted_path).await.unwrap(),
        "corrupted primary must be renamed to config.toml.corrupted"
    );
}

#[tokio::test]
async fn load_or_init_falls_back_to_defaults_when_no_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let config_path = root.join("config.toml");
    write_file(&config_path, CORRUPTED_TOML).await;

    let config = load_or_init_for_workspace(root).await;

    assert_eq!(
        config.default_model.as_deref(),
        Some(crate::openhuman::config::schema::DEFAULT_MODEL),
        "must fall back to defaults when no backup exists"
    );

    assert!(tokio::fs::try_exists(&config_path).await.unwrap());

    // The corrupted primary must have been renamed to .corrupted.
    let corrupted_path = root.join("config.toml.corrupted");
    assert!(
        tokio::fs::try_exists(&corrupted_path).await.unwrap(),
        "corrupted primary must be renamed to config.toml.corrupted"
    );
}

#[tokio::test]
async fn load_or_init_does_not_trigger_recovery_on_valid_config() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write_file(
        &root.join("config.toml"),
        r#"default_model = "gpt-valid"
default_temperature = 0.7
"#,
    )
    .await;

    let config = load_or_init_for_workspace(root).await;

    assert_eq!(
        config.default_model.as_deref(),
        Some("gpt-valid"),
        "valid config must load normally without recovery"
    );
    assert!(
        !config.recovered_from_corruption,
        "a valid config must not set the recovery flag"
    );
}

#[tokio::test]
async fn load_or_init_reads_valid_config_through_retry_wrapper() {
    // OPENHUMAN-TAURI-9R regression: the config read is wrapped in
    // `retry_with_backoff_async`. Confirm the happy path is untouched —
    // a present, readable, valid config loads on the first attempt with
    // no behavior change from the wrapper.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write_file(
        &root.join("config.toml"),
        r#"default_model = "gpt-through-retry"
default_temperature = 0.5
"#,
    )
    .await;

    let config = load_or_init_for_workspace(root).await;

    assert_eq!(
        config.default_model.as_deref(),
        Some("gpt-through-retry"),
        "valid config must load on first attempt through the retry wrapper"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn load_or_init_read_failure_embeds_path_in_error_context() {
    // OPENHUMAN-TAURI-9R (~8k events, Windows): the read at the
    // `config_path.exists()` branch raced `Config::save`'s atomic rename
    // and surfaced the opaque "Failed to read config file" with no path
    // or underlying cause. The fix retries transient Windows locking
    // errors AND embeds the config path in the context; #3962 additionally
    // surfaces the underlying io cause (`os error N`) through `{:#}`.
    //
    // Trigger a genuine non-transient read failure with a 0o000 (unreadable)
    // *regular* file — not a directory, which `impl_load` now rejects with a
    // distinct message before the read (see the directory guard / Codex P2).
    // `exists()` is true so we enter the read branch; `read_to_string` fails
    // with EACCES, which `is_transient_fs_error` does not retry. Skipped under
    // root, which ignores file permissions.
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let config_path = root.join("config.toml");
    std::fs::write(&config_path, "default_temperature = 0.5\n").unwrap();
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o000)).unwrap();

    if std::fs::read_to_string(&config_path).is_ok() {
        return; // running as root — permissions are ignored, assertion is moot
    }

    let env = MapEnv::default().with("OPENHUMAN_WORKSPACE", root.to_str().unwrap());
    let err = Config::load_or_init_with_env_lookup(root, &root.join("workspace"), &env)
        .await
        .expect_err("reading an unreadable config.toml must fail");

    let msg = format!("{err:#}");
    assert!(
        msg.contains("Failed to read config file"),
        "error must carry the read-failure context: {msg}"
    );
    assert!(
        msg.contains("config.toml"),
        "error context must embed the config path so Sentry titles are triageable: {msg}"
    );
    assert!(
        msg.contains("os error"),
        "error must carry the underlying io cause via {{:#}} (#3962): {msg}"
    );
}

/// A read denial on an existing config is fully explained by the file's
/// uid/gid/mode and the process's euid — numbers the loader already has in
/// hand. Without them a report of
/// `Permission denied (os error 13)` cannot distinguish a mis-owned container
/// volume (our defect) from a host ACL (the user's), which is exactly the
/// ambiguity that made the sign-in failure undiagnosable.
#[cfg(unix)]
#[tokio::test]
async fn load_or_init_read_failure_reports_file_and_process_ownership() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let config_path = root.join("config.toml");
    std::fs::write(&config_path, "default_temperature = 0.5\n").unwrap();
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o000)).unwrap();

    if std::fs::read_to_string(&config_path).is_ok() {
        return; // running as root — permissions are ignored, assertion is moot
    }

    let env = MapEnv::default().with("OPENHUMAN_WORKSPACE", root.to_str().unwrap());
    let err = Config::load_or_init_with_env_lookup(root, &root.join("workspace"), &env)
        .await
        .expect_err("reading an unreadable config.toml must fail");

    let msg = format!("{err:#}");
    for needle in ["file uid=", "gid=", "mode=0000", "process euid="] {
        assert!(
            msg.contains(needle),
            "error must carry `{needle}` so the denial is diagnosable: {msg}"
        );
    }
    // We created the file, so it is NOT an ownership mismatch — the marker
    // must stay off, or every ordinary ACL denial would start paging.
    assert!(
        !msg.contains(CONFIG_OWNER_MISMATCH_MARKER),
        "a config we own must not be reported as an ownership mismatch: {msg}"
    );
}

/// `Config::save` writes the live config through a temp file + atomic rename,
/// so the temp file's mode becomes the config's mode. It used to be created at
/// `0o666 & ~umask` (0644), silently re-widening a file that holds `enc2:`
/// provider keys and channel tokens on every settings change, and propagating
/// the same mode onto `config.toml.bak` via `fs::copy`.
#[cfg(unix)]
#[tokio::test]
async fn save_keeps_config_and_backup_owner_only_readable() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let config = Config {
        config_path: config_path.clone(),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };

    config.save().await.expect("first save");
    let mode = std::fs::metadata(&config_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "config.toml must be owner-only after save");

    // The second save is the one that creates the .bak (from the temp file).
    config.save().await.expect("second save");
    let backup_mode = std::fs::metadata(config_path.with_extension("toml.bak"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        backup_mode, 0o600,
        "config.toml.bak holds the same secrets and must be owner-only too"
    );
    let mode_after = std::fs::metadata(&config_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode_after, 0o600, "an overwriting save must not re-widen");
}

#[tokio::test]
async fn load_or_init_recovers_from_non_utf8_config() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Write binary data that is NOT valid UTF-8.
    let config_path = root.join("config.toml");
    let binary_bytes: Vec<u8> = vec![0xff, 0xfe, 0x00, 0x01, 0x02];
    write_binary(&config_path, &binary_bytes).await;

    let config = load_or_init_for_workspace(root).await;

    // Should have loaded defaults (not crashed).
    assert!(
        config.default_model.is_some(),
        "must load defaults from non-UTF-8 config"
    );

    // The runtime recovery flag must be set so the boot path can surface a
    // user-visible notice (#5167).
    assert!(
        config.recovered_from_corruption,
        "recovered_from_corruption must be set after non-UTF-8 recovery"
    );

    // The original binary file should have been renamed to .corrupted.<ts>.
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

    // A fresh config.toml must have been created by the persistence logic.
    assert!(
        tokio::fs::try_exists(&config_path).await.unwrap(),
        "a fresh config.toml must exist after recovery"
    );
}
