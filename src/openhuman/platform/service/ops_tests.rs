use super::*;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

// NOTE: `service_install`, `service_start`, `service_stop`,
// `service_status`, `service_uninstall`, and `service_restart`
// mutate real OS state (launchctl / systemd) or terminate the
// process. They are not safe to exercise from unit tests; the
// RPC adapter tests live in tests/json_rpc_e2e.rs.

// ── daemon_host_get / set ────────────────────────────────────

#[tokio::test]
async fn daemon_host_get_returns_default_when_no_file_present() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // Ensure the config dir exists so `load_for_config_dir` can
    // operate (most loaders treat a missing dir as "use default").
    std::fs::create_dir_all(tmp.path()).unwrap();
    let out = daemon_host_get(&config).await.unwrap();
    // No assertion on `show_tray` value — defaults vary by build.
    // The contract under test is that the function returns Ok with
    // the canonical log line and a deterministic struct shape.
    assert!(out
        .logs
        .iter()
        .any(|l| l.contains("daemon host config loaded")));
    let _ = out.value.show_tray;
}

#[tokio::test]
async fn daemon_host_set_persists_value_visible_to_subsequent_get() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    std::fs::create_dir_all(tmp.path()).unwrap();

    // Write `show_tray = false`, then read it back.
    let saved = daemon_host_set(&config, false).await.unwrap();
    assert!(!saved.value.show_tray);
    assert!(saved
        .logs
        .iter()
        .any(|l| l.contains("daemon host config saved")));

    let loaded = daemon_host_get(&config).await.unwrap();
    assert!(
        !loaded.value.show_tray,
        "set→get round-trip must observe the persisted value"
    );

    // Flip it back and confirm the toggle round-trips too.
    let saved = daemon_host_set(&config, true).await.unwrap();
    assert!(saved.value.show_tray);
    let loaded = daemon_host_get(&config).await.unwrap();
    assert!(loaded.value.show_tray);
}

#[tokio::test]
async fn daemon_host_get_errors_when_config_path_has_no_parent() {
    // A config_path of just a filename (no parent directory) trips
    // the "failed to resolve config directory" guard.
    let mut config = Config::default();
    config.config_path = std::path::PathBuf::from("");
    let err = daemon_host_get(&config).await.unwrap_err();
    assert!(
        err.contains("failed to resolve config directory"),
        "expected config-dir error, got: {err}"
    );
}

#[tokio::test]
async fn daemon_host_set_errors_when_config_path_has_no_parent() {
    let mut config = Config::default();
    config.config_path = std::path::PathBuf::from("");
    let err = daemon_host_set(&config, true).await.unwrap_err();
    assert!(err.contains("failed to resolve config directory"));
}
