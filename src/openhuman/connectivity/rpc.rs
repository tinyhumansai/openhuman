//! `openhuman.connectivity_diag` RPC.
//!
//! Returns a snapshot of the local sidecar's process id + RPC port + backend
//! Socket.IO state, so the frontend's coreHealthMonitor can prove "the local
//! core is alive" without conflating that signal with the backend websocket
//! or the browser's internet connectivity. See issue #1527.

use serde::Serialize;
use serde_json::json;
use tracing::{debug, warn};

use crate::openhuman::socket::manager::global_socket_manager;
use crate::rpc::RpcOutcome;

use super::ops::is_port_in_use;

/// Lightweight diagnostic payload returned by `openhuman.connectivity_diag`.
///
/// Field shape is intentionally flat so a curl/jq dump is human-readable,
/// and so the frontend can map straight into typed Redux state.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectivityDiagResponse {
    /// Backend Socket.IO state, lowercased (e.g. `"connected"`,
    /// `"disconnected"`, `"connecting"`, `"reconnecting"`, `"error"`). When
    /// the SocketManager has not been bootstrapped yet (test runs, early
    /// startup) we report `"uninitialized"`.
    pub socket_state: String,
    /// Last user-visible socket error surfaced via `SocketManager`'s
    /// `SharedState.error` slot. `None` when no error pending.
    pub last_ws_error: Option<String>,
    /// Sidecar process id — i.e. the PID of *this* core binary handling the
    /// RPC. The frontend matches this against the PID it started so it can
    /// detect a stale-process scenario where the bound port belongs to an
    /// older crashed sidecar.
    pub sidecar_pid: Option<u32>,
    /// Port the core is configured to listen on.
    pub listen_port: u16,
    /// Whether the configured port currently has a listener bound. Always
    /// `true` while the core is healthy (we are answering the RPC after
    /// all). Surfaced for diagnostic completeness so the UI can detect
    /// "I think I started the sidecar but the port is owned by another
    /// process" if the sidecar is talked to via a different transport.
    pub listen_port_in_use: bool,
}

/// Resolve the configured core RPC port from the environment.
///
/// Mirrors the resolution order in `core_server::transport::http_listener`,
/// but lighter — we only need a number for a TCP probe, not a bound listener.
fn resolve_listen_port() -> u16 {
    if let Ok(raw) = std::env::var("OPENHUMAN_CORE_PORT") {
        match raw.trim().parse::<u16>() {
            Ok(parsed) => {
                debug!(
                    "[connectivity][rpc] resolve_listen_port: using env override port={}",
                    parsed
                );
                return parsed;
            }
            Err(err) => {
                // Log so misconfiguration is visible in diagnostics rather
                // than silently using the default. (addresses @coderabbitai
                // on rpc.rs:56)
                warn!(
                    "[connectivity][rpc] resolve_listen_port: invalid OPENHUMAN_CORE_PORT='{}': {}",
                    raw, err
                );
            }
        }
    }
    debug!("[connectivity][rpc] resolve_listen_port: using default port=7788");
    7788
}

/// Snapshot the backend socket state. Returns `("uninitialized", None)`
/// when the SocketManager singleton hasn't been registered yet — typical
/// during early startup or in unit tests.
fn snapshot_socket_state() -> (String, Option<String>) {
    match global_socket_manager() {
        Some(mgr) => {
            let state = mgr.get_state();
            // ConnectionStatus serializes lowercase via the enum's serde
            // attribute, but `Debug` formats the variant name PascalCase.
            // Funnel through serde_json so the on-the-wire shape stays
            // stable even if Debug formatting changes upstream.
            let status_value = serde_json::to_value(state.status)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "unknown".to_string());
            (status_value, state.error)
        }
        None => ("uninitialized".to_string(), None),
    }
}

/// Build a `ConnectivityDiagResponse` for the live process. Pure-ish: only
/// sources are the env, the in-memory SocketManager state, and a TCP probe.
pub fn snapshot() -> ConnectivityDiagResponse {
    let listen_port = resolve_listen_port();
    let listen_port_in_use = is_port_in_use(listen_port);
    let (socket_state, last_ws_error) = snapshot_socket_state();
    let sidecar_pid = Some(std::process::id());

    ConnectivityDiagResponse {
        socket_state,
        last_ws_error,
        sidecar_pid,
        listen_port,
        listen_port_in_use,
    }
}

pub async fn diag() -> Result<RpcOutcome<serde_json::Value>, String> {
    debug!("[connectivity][rpc] diag: entry");
    let payload = snapshot();
    debug!(
        socket_state = %payload.socket_state,
        listen_port = payload.listen_port,
        listen_port_in_use = payload.listen_port_in_use,
        "[connectivity][rpc] diag: snapshot built"
    );
    let value = serde_json::to_value(&payload)
        .map_err(|e| format!("connectivity diag: serialize failed: {e}"))?;
    Ok(RpcOutcome::single_log(
        json!({ "diag": value }),
        "connectivity diag returned",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize env-var mutation across the three `resolve_listen_port_*`
    /// tests so they don't race each other under Rust's default parallel
    /// runner. Process-global env state means one test's restore can land
    /// in another test's read window without this. Same pattern used in
    /// `webview_accounts/ops.rs` and `tools/impl/system/lsp.rs`.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
        let (state, err) = snapshot_socket_state();
        assert_eq!(state, "uninitialized");
        assert!(err.is_none());
    }

    #[test]
    fn resolve_listen_port_defaults_to_7788_when_env_unset() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Use a UUID-ish guard so we don't clobber an env the test runner
        // genuinely needs. SAFETY: env mutation is process-global; we
        // restore at the end. See SAFETY note in `cargo test --doc`.
        let prev = std::env::var("OPENHUMAN_CORE_PORT").ok();
        // SAFETY: standard Rust test pattern — env access is unsafe in 2024
        // edition because it isn't thread-safe. Tests are single-threaded
        // for this scope and we restore in the same body.
        unsafe {
            std::env::remove_var("OPENHUMAN_CORE_PORT");
        }
        assert_eq!(resolve_listen_port(), 7788);
        if let Some(value) = prev {
            unsafe {
                std::env::set_var("OPENHUMAN_CORE_PORT", value);
            }
        }
    }

    #[test]
    fn resolve_listen_port_honours_env_override() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("OPENHUMAN_CORE_PORT").ok();
        unsafe {
            std::env::set_var("OPENHUMAN_CORE_PORT", "65000");
        }
        assert_eq!(resolve_listen_port(), 65000);
        match prev {
            Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_PORT", value) },
            None => unsafe { std::env::remove_var("OPENHUMAN_CORE_PORT") },
        }
    }

    #[test]
    fn resolve_listen_port_falls_back_on_invalid_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("OPENHUMAN_CORE_PORT").ok();
        unsafe {
            std::env::set_var("OPENHUMAN_CORE_PORT", "not-a-number");
        }
        assert_eq!(resolve_listen_port(), 7788);
        match prev {
            Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_PORT", value) },
            None => unsafe { std::env::remove_var("OPENHUMAN_CORE_PORT") },
        }
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
        assert!(diag.get("listen_port").is_some());
        assert!(diag.get("listen_port_in_use").is_some());
    }
}
