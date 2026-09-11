use super::*;

// Regression for #4950 ("Clear App Data does nothing"). The GUI clear flow
// signs the user out — removing `active_user.toml` — *before* it asks which
// directory to delete, so a marker-based resolution falls back to the
// pre-login `users/local` dir and leaves the real user's data behind.
// `get_data_paths_for_user` must pin `current_openhuman_dir` to the explicit
// id's `users/<id>` slice, independent of any marker/env state.
#[tokio::test]
async fn get_data_paths_for_user_scopes_current_dir_to_explicit_id() {
    let outcome = get_data_paths_for_user("clear-me-4950").await.unwrap();

    let current = outcome
        .value
        .get("current_openhuman_dir")
        .and_then(|v| v.as_str())
        .expect("current_openhuman_dir present");
    // Normalize Windows separators so the suffix check is platform-agnostic.
    assert!(
        current.replace('\\', "/").ends_with("users/clear-me-4950"),
        "current dir must be scoped to the explicit user id, got {current}"
    );

    // Resolution must be genuinely user-scoped, not the shared root — the
    // reset must never `remove_dir_all` the root that holds sibling users.
    let default = outcome
        .value
        .get("default_openhuman_dir")
        .and_then(|v| v.as_str())
        .expect("default_openhuman_dir present");
    assert_ne!(
        current, default,
        "current dir must differ from the shared root"
    );
    let current_norm = current.replace('\\', "/");
    let default_norm = default.replace('\\', "/");
    assert!(
        current_norm.starts_with(default_norm.as_str()),
        "current dir ({current}) must live under the shared root ({default})"
    );
}

// #4950 hardening: `user_id` is caller-controlled (arrives over /rpc and via
// the Tauri reset command) and flows into remove_dir_all, so traversal or
// absolute ids must be rejected outright rather than resolving a delete
// target outside `<root>/users/<id>`.
#[tokio::test]
async fn get_data_paths_for_user_rejects_unsafe_ids() {
    for bad in ["..", ".", "../escape", "/etc", "a/b", "a\\b", ""] {
        assert!(
            get_data_paths_for_user(bad).await.is_err(),
            "unsafe user id {bad:?} must be rejected"
        );
    }
}

// A directory at the config path is corruption, not a transient/denied read:
// the read site fails it fast with distinct wording, and the observability
// classifier MUST keep paging it (never demote to ConfigReadIoFailure). This
// guards the Codex P2 hole where a Windows directory-at-config surfaces the
// same `os error 5` shape as a real ACL denial (#3962).
#[tokio::test]
async fn config_directory_pages_and_is_not_demoted() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    std::fs::create_dir(&config_path).unwrap();

    let snapshot = Config {
        config_path: config_path.clone(),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };

    let err = reload_config_snapshot_with_timeout(&snapshot)
        .await
        .expect_err("a directory at the config path must fail");

    assert!(
        err.contains("is a directory") || err.contains("not a file"),
        "directory-at-config must report a distinct, non-read error: {err}"
    );
    assert_ne!(
        crate::core::observability::expected_error_kind(&err),
        Some(crate::core::observability::ExpectedErrorKind::ConfigReadIoFailure),
        "a directory at the config path is corruption — it must keep paging, not demote: {err}"
    );
}

// load_config_with_timeout resolves the process-global OPENHUMAN_WORKSPACE,
// so serialize against the other env-mutating config tests. Exercises the
// load_or_init directory guard + the `Ok(Err) => format!("{e:#}")` arm.
#[tokio::test]
async fn load_config_with_timeout_rejects_directory_config() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    std::fs::create_dir(&config_path).unwrap();

    let _g = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var("OPENHUMAN_WORKSPACE").ok();
    std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path().to_str().unwrap());

    let result = load_config_with_timeout().await;

    match prev {
        Some(v) => std::env::set_var("OPENHUMAN_WORKSPACE", v),
        None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
    }

    let err = result.expect_err("a directory at the config path must fail");
    assert!(
        err.contains("is a directory") || err.contains("not a file"),
        "directory-at-config must report a distinct, non-read error: {err}"
    );
}

// The #3962 keystone: a genuine read failure on a regular file must surface
// the full anyhow chain (`{:#}`) — the read context PLUS the underlying io
// cause (`os error N`) — through the RPC String boundary, not just the top
// `with_context` line. Triggered portably with a 0o000 (unreadable) file;
// skipped under root, which ignores file permissions.
#[cfg(unix)]
#[tokio::test]
async fn load_surfaces_full_io_chain_on_unreadable_file() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    std::fs::write(&config_path, "default_temperature = 0.5\n").unwrap();
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o000)).unwrap();

    // Root bypasses file-permission checks — the read would succeed and the
    // assertion would be meaningless, so skip in that environment.
    if std::fs::read_to_string(&config_path).is_ok() {
        return;
    }

    let snapshot = Config {
        config_path: config_path.clone(),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };

    let err = reload_config_snapshot_with_timeout(&snapshot)
        .await
        .expect_err("an unreadable config file must fail");

    assert!(
        err.contains("Failed to read config file"),
        "error must carry the read context: {err}"
    );
    assert!(
        err.contains("os error"),
        "error must carry the underlying io cause via {{:#}} (#3962): {err}"
    );
}
