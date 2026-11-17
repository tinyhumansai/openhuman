//! In-process JSON-RPC E2E coverage for app state.

use crate::config_auth_harness::*;

#[tokio::test]
async fn app_state_update_persists_and_snapshot_reads_local_state() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let updated = rpc(
        &harness.rpc_base,
        30_001,
        "openhuman.app_state_update_local_state",
        json!({
            "encryptionKey": "worker-a-key",
            "onboardingTasks": {
                "accessibilityPermissionGranted": true,
                "localModelConsentGiven": true,
                "localModelDownloadStarted": false,
                "enabledTools": ["memory.search", "tools.web_search"],
                "connectedSources": ["gmail"],
                "updatedAtMs": 123456
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&updated, "app_state_update_local_state")
            .get("encryptionKey")
            .and_then(Value::as_str),
        Some("worker-a-key")
    );

    let snapshot = rpc(
        &harness.rpc_base,
        30_002,
        "openhuman.app_state_snapshot",
        json!({}),
    )
    .await;
    let local_state = payload(&snapshot, "app_state_snapshot")
        .get("localState")
        .unwrap_or_else(|| panic!("snapshot should include localState: {snapshot}"));
    assert_eq!(
        local_state.get("encryptionKey").and_then(Value::as_str),
        Some("worker-a-key")
    );
    assert_eq!(
        local_state.pointer("/onboardingTasks/enabledTools/0"),
        Some(&json!("memory.search"))
    );

    let cleared = rpc(
        &harness.rpc_base,
        30_003,
        "openhuman.app_state_update_local_state",
        json!({
            "encryptionKey": null,
            "onboardingTasks": null
        }),
    )
    .await;
    assert!(
        payload(&cleared, "app_state_update_local_state")
            .get("encryptionKey")
            .is_none(),
        "null patch should clear optional encryption key: {cleared}"
    );

    let blank_cleared = rpc(
        &harness.rpc_base,
        30_004,
        "openhuman.app_state_update_local_state",
        json!({ "encryptionKey": "   " }),
    )
    .await;
    assert!(
        payload(&blank_cleared, "app_state_update_local_state blank key")
            .get("encryptionKey")
            .is_none(),
        "blank encryption key should also clear the optional value: {blank_cleared}"
    );

    let invalid_patch = rpc(
        &harness.rpc_base,
        30_005,
        "openhuman.app_state_update_local_state",
        json!({ "onboardingTasks": "not-an-object" }),
    )
    .await;
    assert_error_contains(
        &invalid_patch,
        "app_state_update_local_state invalid onboardingTasks",
        "invalid params",
    );

    let unchanged = rpc(
        &harness.rpc_base,
        30_006,
        "openhuman.app_state_update_local_state",
        json!({}),
    )
    .await;
    assert!(
        payload(&unchanged, "app_state_update_local_state empty patch")
            .get("encryptionKey")
            .is_none(),
        "empty patch should preserve the already-cleared encryption key: {unchanged}"
    );

    harness.join.abort();
}


#[tokio::test]
async fn app_state_snapshot_degrades_runtime_service_status_failures() {
    let _lock = env_lock_async().await;
    let harness = setup().await;
    let service_state_path = harness.home.join("service-status-failure.json");
    std::fs::write(
        &service_state_path,
        serde_json::to_vec_pretty(&json!({
            "installed": true,
            "running": true,
            "agent_running": true,
            "failures": {
                "status": "forced status failure from app_state test"
            }
        }))
        .expect("serialize service mock state"),
    )
    .expect("write service mock state");
    let _service_mock = EnvVarGuard::set("OPENHUMAN_SERVICE_MOCK", "1");
    let _service_state =
        EnvVarGuard::set_to_path("OPENHUMAN_SERVICE_MOCK_STATE_FILE", &service_state_path);

    // The runtime snapshot cache is keyed by config identity (workspace_dir), so
    // this harness's unique workspace guarantees a cache miss regardless of prior
    // app_state_snapshot tests — the call exercises the service-status fallback
    // against our injected mock rather than returning a foreign cached runtime.
    let snapshot = rpc(
        &harness.rpc_base,
        30_051,
        "openhuman.app_state_snapshot",
        json!({}),
    )
    .await;
    let service = payload(&snapshot, "app_state_snapshot with service status failure")
        .pointer("/runtime/service")
        .expect("snapshot should include runtime service status");
    assert!(
        service
            .pointer("/state/Unknown")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("forced status failure")),
        "service status failures should degrade to Unknown state: {service}"
    );
    assert_eq!(
        service.get("label").and_then(Value::as_str),
        Some("OpenHuman")
    );

    harness.join.abort();
}


#[tokio::test]
async fn app_state_snapshot_and_update_surface_state_dir_creation_errors() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let config = rpc(&harness.rpc_base, 30_101, "openhuman.config_get", json!({})).await;
    let workspace_dir = payload(&config, "config_get for app_state state-dir error")
        .get("workspace_dir")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .expect("config_get should expose workspace_dir");
    let state_path = workspace_dir.join("state");
    std::fs::create_dir_all(&workspace_dir).expect("create workspace dir");
    std::fs::write(&state_path, "not a directory").expect("write state path as file");

    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            30_102,
            "openhuman.app_state_snapshot",
            json!({}),
        )
        .await,
        "app_state_snapshot with file at state path",
        "failed to create workspace state dir",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            30_103,
            "openhuman.app_state_update_local_state",
            json!({ "encryptionKey": "cannot-save" }),
        )
        .await,
        "app_state_update_local_state with file at state path",
        "failed to create workspace state dir",
    );

    harness.join.abort();
}

#[cfg(unix)]

#[tokio::test]
async fn app_state_snapshot_keeps_unquarantinable_local_state_path_but_uses_defaults() {
    use std::os::unix::fs::PermissionsExt;

    let _lock = env_lock_async().await;
    let harness = setup().await;

    let config = rpc(&harness.rpc_base, 31_151, "openhuman.config_get", json!({})).await;
    let workspace_dir = payload(&config, "config_get for unquarantinable app_state path")
        .get("workspace_dir")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .expect("config_get should expose workspace_dir");
    let state_dir = workspace_dir.join("state");
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    let app_state_path = state_dir.join("app-state.json");
    std::fs::create_dir(&app_state_path).expect("create app-state directory");

    let original_permissions = std::fs::metadata(&state_dir)
        .expect("state dir metadata")
        .permissions();
    let mut read_only_permissions = original_permissions.clone();
    read_only_permissions.set_mode(0o500);
    std::fs::set_permissions(&state_dir, read_only_permissions).expect("make state dir unwritable");

    // Unix permission bits only block writes for non-root users. CI containers
    // frequently run as root, where 0o500 does NOT prevent rename/removal — the
    // quarantine would succeed and this test's precondition (an *unquarantinable*
    // path) can't be established. Probe whether the mode is actually enforced;
    // if writes still succeed, skip rather than assert a guarantee the OS isn't
    // providing.
    let probe = state_dir.join(".perm-probe");
    if std::fs::write(&probe, b"x").is_ok() {
        let _ = std::fs::remove_file(&probe);
        std::fs::set_permissions(&state_dir, original_permissions)
            .expect("restore state dir permissions");
        eprintln!(
            "[skip] app_state_snapshot_keeps_unquarantinable_local_state_path_but_uses_defaults: \
             filesystem permissions not enforced (running as root?); cannot make state dir unwritable"
        );
        harness.join.abort();
        return;
    }

    let snapshot = rpc(
        &harness.rpc_base,
        31_152,
        "openhuman.app_state_snapshot",
        json!({}),
    )
    .await;

    std::fs::set_permissions(&state_dir, original_permissions)
        .expect("restore state dir permissions");

    let local_state = payload(
        &snapshot,
        "app_state_snapshot after failed quarantine of unreadable path",
    )
    .get("localState")
    .expect("snapshot should include localState");
    assert!(
        local_state.as_object().is_some_and(|map| map.is_empty()),
        "unquarantinable app state should still fall back to defaults: {local_state}"
    );
    assert!(
        app_state_path.exists(),
        "unwritable state dir should prevent quarantine rename/removal of the live path"
    );

    harness.join.abort();
}


#[tokio::test]
async fn app_state_snapshot_quarantines_unreadable_local_state_path() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let config = rpc(&harness.rpc_base, 31_101, "openhuman.config_get", json!({})).await;
    let workspace_dir = payload(&config, "config_get for unreadable app_state path")
        .get("workspace_dir")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .expect("config_get should expose workspace_dir");
    let state_dir = workspace_dir.join("state");
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    let app_state_path = state_dir.join("app-state.json");
    std::fs::create_dir(&app_state_path).expect("create unreadable app-state directory");

    let snapshot = rpc(
        &harness.rpc_base,
        31_102,
        "openhuman.app_state_snapshot",
        json!({}),
    )
    .await;
    let local_state = payload(&snapshot, "app_state_snapshot after unreadable state path")
        .get("localState")
        .expect("snapshot should include localState");
    assert!(
        local_state.as_object().is_some_and(|map| map.is_empty()),
        "unreadable app state should fall back to defaults: {local_state}"
    );
    assert!(
        !app_state_path.exists(),
        "unreadable app state path should be moved out of the live path"
    );
    let quarantined = std::fs::read_dir(&state_dir)
        .expect("read state dir")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("app-state.json.corrupted.")
                && entry.path().is_dir()
        });
    assert!(
        quarantined,
        "unreadable app state directory should be quarantined under {state_dir:?}"
    );

    harness.join.abort();
}
