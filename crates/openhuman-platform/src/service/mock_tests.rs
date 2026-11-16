//! Unit tests for `service::mock` — the deterministic file-backed service
//! manager activated by `OPENHUMAN_SERVICE_MOCK`.
//!
//! These tests cover:
//! - `is_enabled()` responds to the env-var values accepted as truthy/falsy.
//! - State-machine transitions: install → start → stop → uninstall, with the
//!   correct `ServiceState` at each step.
//! - Forced-failure injection via the `failures` JSON field.
//! - Dispatch routing: `service::install / status` delegate to the mock when
//!   `OPENHUMAN_SERVICE_MOCK` is set.

use super::*;
use crate::test_env::lock_env;
use tempfile::TempDir;

// ── is_enabled ────────────────────────────────────────────────────────────────

#[test]
fn is_enabled_returns_false_when_var_absent() {
    let _g = lock_env();
    // Remove any stale value from a previous test.
    std::env::remove_var("OPENHUMAN_SERVICE_MOCK");
    assert!(!is_enabled());
}

#[test]
fn is_enabled_returns_true_for_truthy_values() {
    let _g = lock_env();
    for v in ["1", "true", "yes", "on", "TRUE", "YES", "ON"] {
        std::env::set_var("OPENHUMAN_SERVICE_MOCK", v);
        assert!(
            is_enabled(),
            "is_enabled should be true for OPENHUMAN_SERVICE_MOCK={v}"
        );
    }
    std::env::remove_var("OPENHUMAN_SERVICE_MOCK");
}

#[test]
fn is_enabled_returns_false_for_falsy_values() {
    let _g = lock_env();
    for v in ["0", "false", "no", "off", "FALSE"] {
        std::env::set_var("OPENHUMAN_SERVICE_MOCK", v);
        assert!(
            !is_enabled(),
            "is_enabled should be false for OPENHUMAN_SERVICE_MOCK={v}"
        );
    }
    std::env::remove_var("OPENHUMAN_SERVICE_MOCK");
}

// ── state-machine transitions ─────────────────────────────────────────────────

fn test_spec(tmp: &TempDir) -> ServiceSpec {
    ServiceSpec {
        label: "com.openhuman.core".to_string(),
        legacy_labels: Vec::new(),
        exe: tmp.path().join("openhuman-core"),
        args: vec!["run".to_string()],
        logs_dir: tmp.path().join("logs"),
        work_dir: tmp.path().to_path_buf(),
    }
}

/// Use an isolated state file to avoid cross-test interference.
struct StateFileGuard {
    _dir: TempDir,
}

impl StateFileGuard {
    fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("svc-mock-state.json");
        std::env::set_var("OPENHUMAN_SERVICE_MOCK_STATE_FILE", &path);
        StateFileGuard { _dir: dir }
    }
}

impl Drop for StateFileGuard {
    fn drop(&mut self) {
        std::env::remove_var("OPENHUMAN_SERVICE_MOCK_STATE_FILE");
    }
}

#[test]
fn initial_status_is_not_installed() {
    let _g = lock_env();
    let _sf = StateFileGuard::new();
    let tmp = TempDir::new().unwrap();
    let spec = test_spec(&tmp);

    let status = status(&spec).expect("status should succeed");
    assert!(
        matches!(status.state, ServiceState::NotInstalled),
        "fresh mock state must be NotInstalled, got {:?}",
        status.state
    );
}

#[test]
fn install_transitions_to_stopped() {
    let _g = lock_env();
    let _sf = StateFileGuard::new();
    let tmp = TempDir::new().unwrap();
    let spec = test_spec(&tmp);

    let status = install(&spec).expect("install should succeed");
    assert!(
        matches!(status.state, ServiceState::Stopped),
        "after install, state must be Stopped, got {:?}",
        status.state
    );
}

#[test]
fn start_after_install_transitions_to_running() {
    let _g = lock_env();
    let _sf = StateFileGuard::new();
    let tmp = TempDir::new().unwrap();
    let spec = test_spec(&tmp);

    install(&spec).expect("install");
    let status = start(&spec).expect("start");
    assert!(
        matches!(status.state, ServiceState::Running),
        "after install+start, state must be Running, got {:?}",
        status.state
    );
}

#[test]
fn stop_transitions_from_running_to_stopped() {
    let _g = lock_env();
    let _sf = StateFileGuard::new();
    let tmp = TempDir::new().unwrap();
    let spec = test_spec(&tmp);

    install(&spec).expect("install");
    start(&spec).expect("start");
    let status = stop(&spec).expect("stop");
    assert!(
        matches!(status.state, ServiceState::Stopped),
        "after stop, state must be Stopped, got {:?}",
        status.state
    );
}

#[test]
fn uninstall_transitions_to_not_installed() {
    let _g = lock_env();
    let _sf = StateFileGuard::new();
    let tmp = TempDir::new().unwrap();
    let spec = test_spec(&tmp);

    install(&spec).expect("install");
    start(&spec).expect("start");
    let status = uninstall(&spec).expect("uninstall");
    assert!(
        matches!(status.state, ServiceState::NotInstalled),
        "after uninstall, state must be NotInstalled, got {:?}",
        status.state
    );
}

#[test]
fn start_without_install_returns_not_installed() {
    let _g = lock_env();
    let _sf = StateFileGuard::new();
    let tmp = TempDir::new().unwrap();
    let spec = test_spec(&tmp);

    // Start without installing should leave us in NotInstalled (not error).
    let status = start(&spec).expect("start without install should not error");
    assert!(
        matches!(status.state, ServiceState::NotInstalled),
        "start without install must leave state NotInstalled, got {:?}",
        status.state
    );
}

// ── forced-failure injection ──────────────────────────────────────────────────

fn write_state_file_with_install_failure(path: &std::path::Path) {
    let json = serde_json::json!({
        "installed": false,
        "running": false,
        "agent_running": true,
        "failures": {
            "install": "injected install failure"
        }
    });
    std::fs::write(path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
}

fn write_state_file_with_start_failure(path: &std::path::Path) {
    let json = serde_json::json!({
        "installed": true,
        "running": false,
        "agent_running": true,
        "failures": {
            "start": "injected start failure"
        }
    });
    std::fs::write(path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
}

#[test]
fn forced_install_failure_returns_error() {
    let _g = lock_env();
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("fail-state.json");
    write_state_file_with_install_failure(&state_path);
    std::env::set_var("OPENHUMAN_SERVICE_MOCK_STATE_FILE", &state_path);

    let tmp = TempDir::new().unwrap();
    let spec = test_spec(&tmp);
    let result = install(&spec);

    std::env::remove_var("OPENHUMAN_SERVICE_MOCK_STATE_FILE");

    assert!(
        result.is_err(),
        "install with forced failure must return Err"
    );
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("injected install failure"),
        "error message should mention the injected failure, got: {err_str}"
    );
}

#[test]
fn forced_start_failure_returns_error() {
    let _g = lock_env();
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("fail-state.json");
    write_state_file_with_start_failure(&state_path);
    std::env::set_var("OPENHUMAN_SERVICE_MOCK_STATE_FILE", &state_path);

    let tmp = TempDir::new().unwrap();
    let spec = test_spec(&tmp);
    let result = start(&spec);

    std::env::remove_var("OPENHUMAN_SERVICE_MOCK_STATE_FILE");

    assert!(result.is_err(), "start with forced failure must return Err");
}

// ── dispatch routing via the service dispatchers ─────────────────────────────

#[test]
fn dispatch_routes_install_to_mock_when_env_set() {
    let _g = lock_env();
    let _sf = StateFileGuard::new();
    let tmp = TempDir::new().unwrap();
    let spec = test_spec(&tmp);

    // Enable the mock.
    std::env::set_var("OPENHUMAN_SERVICE_MOCK", "1");
    let status = crate::service::install(&spec).expect("service::install via mock");
    std::env::remove_var("OPENHUMAN_SERVICE_MOCK");

    assert!(
        matches!(status.state, ServiceState::Stopped),
        "dispatch → mock install must yield Stopped, got {:?}",
        status.state
    );
    assert_eq!(
        status.details.as_deref(),
        Some("service mock backend"),
        "dispatch must originate from the mock backend"
    );
}

#[test]
fn dispatch_routes_status_to_mock_when_env_set() {
    let _g = lock_env();
    let _sf = StateFileGuard::new();
    let tmp = TempDir::new().unwrap();
    let spec = test_spec(&tmp);

    std::env::set_var("OPENHUMAN_SERVICE_MOCK", "1");
    let status = crate::service::status(&spec).expect("service::status via mock");
    std::env::remove_var("OPENHUMAN_SERVICE_MOCK");

    // Fresh state → NotInstalled
    assert!(
        matches!(status.state, ServiceState::NotInstalled),
        "dispatch → mock status must be NotInstalled initially, got {:?}",
        status.state
    );
}

// ── mock_agent_running ────────────────────────────────────────────────────────

#[test]
fn mock_agent_running_returns_none_when_disabled() {
    let _g = lock_env();
    std::env::remove_var("OPENHUMAN_SERVICE_MOCK");
    assert!(
        mock_agent_running().is_none(),
        "mock_agent_running must be None when the mock env var is absent"
    );
}

#[test]
fn mock_agent_running_returns_true_by_default_state() {
    let _g = lock_env();
    let _sf = StateFileGuard::new();
    let tmp = TempDir::new().unwrap();
    let spec = test_spec(&tmp);
    // Initialise the state file by calling status (creates default).
    std::env::set_var("OPENHUMAN_SERVICE_MOCK", "1");
    let _ = status(&spec).ok();
    let result = mock_agent_running();
    std::env::remove_var("OPENHUMAN_SERVICE_MOCK");

    // Default state has agent_running = true.
    assert_eq!(
        result,
        Some(true),
        "default mock state must have agent_running=true"
    );
}
