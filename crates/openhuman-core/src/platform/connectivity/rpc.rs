//! `openhuman.connectivity_diag` RPC.
//!
//! Returns a snapshot of the local sidecar's process id + RPC port + backend
//! Socket.IO state, so the frontend's coreHealthMonitor can prove "the local
//! core is alive" without conflating that signal with the backend websocket
//! or the browser's internet connectivity. See issue #1527.

use serde::Serialize;
use serde_json::json;
use tracing::{debug, warn};

use crate::core::Outcome;
use crate::platform::socket::global_socket_manager;

use super::ops::is_port_in_use;

const DEFAULT_CORE_PORT: u16 = 7788;

/// Lightweight diagnostic payload returned by `openhuman.connectivity_diag`.
///
/// Field shape is intentionally flat so a curl/jq dump is human-readable,
/// and so the frontend can map straight into typed Redux state.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectivityDiagResponse {
    /// Backend Socket.IO state, lowercased (e.g. `"connected"`,
    /// `"disconnected"`, `"connecting"`, `"reconnecting"`, `"error"`). When
    /// a host has not installed the socket port yet (test runs, early startup)
    /// we report `"uninitialized"`.
    pub socket_state: String,
    /// Last user-visible socket error surfaced by the host-owned socket port.
    /// `None` when no error is pending.
    pub last_ws_error: Option<String>,
    /// Whether the core's backend reconnect loop is running. `false` means
    /// nobody is retrying — never connected, signed out, a local session, or
    /// stopped after a terminal failure — and the frontend shows no outage
    /// for it. `true` with any `socket_state` other than `connected` is a
    /// link that is down and being retried, or a live transport whose
    /// Socket.IO namespace the server closed (#6256). `false` when the
    /// The socket port is not installed.
    pub socket_loop_active: bool,
    /// Whether that loop exited on a terminal failure — no usable session
    /// token: the provider returned nothing or errored, or the backend
    /// rejected the stored token and nothing fresher existed. Only meaningful
    /// while `socket_loop_active` is `false`; it lets the frontend show a link
    /// that stopped for good ("sign in again") as an outage instead of the
    /// absence of a link. Cleared by the next connect or disconnect (#6270).
    pub socket_loop_stopped_on_failure: bool,
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
    if let Ok(raw_url) = std::env::var("OPENHUMAN_CORE_RPC_URL") {
        if let Ok(url) = url::Url::parse(raw_url.trim()) {
            if let Some(port) = url.port() {
                debug!(
                    "[connectivity][rpc] resolve_listen_port: using OPENHUMAN_CORE_RPC_URL port={}",
                    port
                );
                return port;
            }
        }
    }

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
    debug!(
        "[connectivity][rpc] resolve_listen_port: using default port={}",
        DEFAULT_CORE_PORT
    );
    DEFAULT_CORE_PORT
}

/// Snapshot the backend socket state as `(status, last error, loop active,
/// loop stopped on failure)`. Returns `("uninitialized", None, false, false)`
/// when the SocketPort hasn't been registered yet — typical
/// during early startup or in unit tests.
fn snapshot_socket_state() -> (String, Option<String>, bool, bool) {
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
            (
                status_value,
                state.error,
                mgr.is_loop_active(),
                mgr.loop_stopped_on_failure(),
            )
        }
        None => ("uninitialized".to_string(), None, false, false),
    }
}

/// Build a `ConnectivityDiagResponse` for the live process. Pure-ish: only
/// sources are the env, in-memory socket-port state, and a TCP probe.
pub fn snapshot() -> ConnectivityDiagResponse {
    let listen_port = resolve_listen_port();
    let listen_port_in_use = is_port_in_use(listen_port);
    let (socket_state, last_ws_error, socket_loop_active, socket_loop_stopped_on_failure) =
        snapshot_socket_state();
    let sidecar_pid = Some(std::process::id());

    ConnectivityDiagResponse {
        socket_state,
        last_ws_error,
        socket_loop_active,
        socket_loop_stopped_on_failure,
        sidecar_pid,
        listen_port,
        listen_port_in_use,
    }
}

pub async fn diag() -> Result<Outcome<serde_json::Value>, String> {
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
    Ok(Outcome::single_log(
        json!({ "diag": value }),
        "connectivity diag returned",
    ))
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
