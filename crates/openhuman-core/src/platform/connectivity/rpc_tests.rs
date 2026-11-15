use super::*;
use crate::config::test_env::EnvVarGuard;

#[test]
fn snapshot_socket_state_is_uninitialized_without_manager() {
    // The global SocketManager OnceLock may already be set if other
    // tests in this binary installed it. Skip in that case rather than
    // fail; we already cover the live path implicitly.
    if global_socket_manager().is_some() {
        eprintln!(
            "[connectivity::rpc tests] global socket manager installed — \
             skipping uninitialized-state assertion"
        );
        return;
    }
    let (state, err, loop_active, stopped_on_failure) = snapshot_socket_state();
    assert_eq!(state, "uninitialized");
    assert!(err.is_none());
    assert!(!loop_active, "no manager means nobody is retrying");
    assert!(
        !stopped_on_failure,
        "no manager means nothing has failed either"
    );
}

// `resolve_listen_port()` reads OPENHUMAN_CORE_RPC_URL ahead of
// OPENHUMAN_CORE_PORT, so every test pins both under the crate-wide env lock.

#[test]
fn resolve_listen_port_defaults_to_7788_when_env_unset() {
    let _env = EnvVarGuard::locked_unset_many(&["OPENHUMAN_CORE_PORT", "OPENHUMAN_CORE_RPC_URL"]);
    assert_eq!(resolve_listen_port(), DEFAULT_CORE_PORT);
}

#[test]
fn resolve_listen_port_honours_env_override() {
    // The URL has higher priority, so clear it for the port to resolve.
    let _env =
        EnvVarGuard::locked_unset("OPENHUMAN_CORE_RPC_URL").with("OPENHUMAN_CORE_PORT", "65000");
    assert_eq!(resolve_listen_port(), 65000);
}

#[test]
fn resolve_listen_port_falls_back_on_invalid_env() {
    let _env = EnvVarGuard::locked_unset("OPENHUMAN_CORE_RPC_URL")
        .with("OPENHUMAN_CORE_PORT", "not-a-number");
    assert_eq!(resolve_listen_port(), DEFAULT_CORE_PORT);
}

#[test]
fn resolve_listen_port_prefers_openhuman_core_rpc_url() {
    let _env = EnvVarGuard::locked_set("OPENHUMAN_CORE_RPC_URL", "http://127.0.0.1:7794/rpc")
        .with("OPENHUMAN_CORE_PORT", "7788");
    assert_eq!(resolve_listen_port(), 7794);
}

#[test]
fn snapshot_populates_all_fields() {
    let snap = snapshot();
    // Don't assert exact pid; just that we set one.
    assert!(snap.sidecar_pid.is_some(), "sidecar_pid should be set");
    assert!(snap.listen_port > 0, "listen_port should be non-zero");
    assert!(
        !snap.socket_state.is_empty(),
        "socket_state should be non-empty"
    );
    // A manager that was never connected (or no manager at all) cannot have a
    // live loop behind it; `connected` without a loop would be a contradiction.
    if snap.socket_state == "uninitialized" || snap.socket_state == "disconnected" {
        assert!(!snap.socket_loop_active || snap.socket_state == "disconnected");
    }
}

#[tokio::test]
async fn diag_returns_serializable_payload() {
    let outcome = diag().await.expect("diag rpc");
    let json = outcome
        .into_cli_compatible_json()
        .expect("into_cli_compatible_json");
    assert!(json.is_object(), "payload should be a JSON object");
    // `single_log` adds a log entry, so `into_cli_compatible_json` wraps
    // the value inside `{ "result": ..., "logs": [...] }`. Look for the
    // diag payload under `result`.
    let result = json.get("result").expect("result envelope key present");
    let diag = result.get("diag").expect("diag key present under result");
    assert!(diag.get("socket_state").is_some());
    assert!(
        diag.get("socket_loop_active")
            .and_then(|v| v.as_bool())
            .is_some(),
        "socket_loop_active must be a bool on the wire (#6256)"
    );
    assert!(
        diag.get("socket_loop_stopped_on_failure")
            .and_then(|v| v.as_bool())
            .is_some(),
        "socket_loop_stopped_on_failure must be a bool on the wire (#6270)"
    );
    assert!(diag.get("listen_port").is_some());
    assert!(diag.get("listen_port_in_use").is_some());
}
