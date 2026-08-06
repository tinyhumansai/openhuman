//! Lazily-started, process-wide in-process HTTP MCP server bound to localhost.
//!
//! The Claude Code provider points the sandboxed `claude` subprocess at this
//! URL so it can reach OpenHuman's memory/tools over loopback **without** the
//! MCP server inheriting CC's OS jail — the server runs here, in the trusted
//! (unjailed) core process, with full workspace access, while CC's own raw
//! tools are denied any access to `~/.openhuman`.
//!
//! Loopback alone is NOT treated as sufficient isolation: any *other* local
//! process could otherwise open sessions against OpenHuman tools/memory. The
//! singleton therefore mints a per-process random bearer token and only the
//! Claude-side MCP config (which carries the matching `Authorization` header)
//! can talk to it.

use std::net::SocketAddr;

// The in-process HTTP MCP server is axum-only, so everything that starts it is
// gated with `http-server` (#5048). `LocalMcpEndpoint` (an inert addr+token
// record) and a disabled-error `ensure_local_http` stay compiled so the
// always-on Claude-Code driver keeps a stable call surface — with the feature
// off, `ensure_local_http` returns a built-without-http-server error.
#[cfg(feature = "http-server")]
use tokio::sync::Mutex;
#[cfg(feature = "http-server")]
use uuid::Uuid;

#[cfg(feature = "http-server")]
use super::http::{run_http_reporting, HttpServerConfig};

/// Endpoint of the running in-process MCP server: its loopback address and the
/// bearer token a client must present.
#[derive(Debug, Clone)]
pub struct LocalMcpEndpoint {
    pub addr: SocketAddr,
    pub token: String,
}

#[cfg(feature = "http-server")]
struct RunningServer {
    endpoint: LocalMcpEndpoint,
    /// Liveness handle. If the server task has exited (bind drop, fatal error),
    /// `is_finished()` is true and we restart rather than handing back a dead
    /// address — `ensure_local_http` is called on every Claude Code turn.
    handle: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "http-server")]
static LOCAL_SERVER: Mutex<Option<RunningServer>> = Mutex::const_new(None);

/// 256-bit random bearer token (two v4 UUIDs, hex). Loopback-only, per process.
#[cfg(feature = "http-server")]
fn mint_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// Ensure the in-process HTTP MCP server is running and return its loopback
/// endpoint (address + bearer token). Idempotent: the server is started once
/// and reused across turns; if the previous instance has exited, it is
/// transparently restarted (and a fresh token minted) so callers never receive
/// a stale, dead URL.
#[cfg(feature = "http-server")]
pub async fn ensure_local_http() -> anyhow::Result<LocalMcpEndpoint> {
    let mut guard = LOCAL_SERVER.lock().await;

    if let Some(running) = guard.as_ref() {
        if !running.handle.is_finished() {
            return Ok(running.endpoint.clone());
        }
        log::warn!("[mcp_server] in-process MCP server had exited; restarting it");
    }

    let token = mint_token();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let config = HttpServerConfig {
        bind_addr: "127.0.0.1:0".parse().expect("valid loopback addr"),
        auth_token: Some(token.clone()),
    };
    let handle = tokio::spawn(async move {
        if let Err(e) = run_http_reporting(config, Some(tx)).await {
            log::error!("[mcp_server] in-process HTTP MCP server exited: {e}");
        }
    });

    let addr = rx
        .await
        .map_err(|_| anyhow::anyhow!("MCP HTTP server never reported its bind address"))?;
    log::info!("[mcp_server] in-process HTTP MCP server ready on {addr} (authenticated)");

    let endpoint = LocalMcpEndpoint { addr, token };
    *guard = Some(RunningServer {
        endpoint: endpoint.clone(),
        handle,
    });
    Ok(endpoint)
}

/// Disabled build: the in-process HTTP MCP server is axum-only and needs the
/// `http-server` feature (#5048). Returns an error so the always-on Claude-Code
/// driver falls back gracefully instead of pointing at a server that was never
/// started. Keeps `ensure_local_http` resolvable under `mcp` in both builds.
#[cfg(not(feature = "http-server"))]
pub async fn ensure_local_http() -> anyhow::Result<LocalMcpEndpoint> {
    Err(anyhow::anyhow!(
        "in-process MCP HTTP server unavailable: built without the http-server feature"
    ))
}

// The real-server tests below are `http-server`-gated; this pins the slim
// build's disabled fallback so both feature branches are covered by the matrix.
#[cfg(all(test, not(feature = "http-server")))]
mod disabled_tests {
    use super::*;

    #[tokio::test]
    async fn ensure_local_http_reports_unavailable_without_http_server() {
        let err = ensure_local_http()
            .await
            .expect_err("slim build without `http-server` must not start a server");
        assert!(
            err.to_string().contains("http-server feature"),
            "error must name the missing feature, got: {err}"
        );
    }
}

// Every test here starts the real HTTP server (`ensure_local_http`) or mints a
// token, both gated, so the module gates in lockstep (#5048).
#[cfg(all(test, feature = "http-server"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ensure_local_http_binds_loopback_with_token_and_is_idempotent() {
        let a = ensure_local_http().await.expect("first start");
        assert!(
            a.addr.ip().is_loopback(),
            "must bind loopback only, got {}",
            a.addr
        );
        assert_ne!(a.addr.port(), 0, "must report a concrete bound port");
        assert!(!a.token.is_empty(), "must mint a bearer token");
        // Singleton: a second call returns the same endpoint, not a new server.
        let b = ensure_local_http().await.expect("second start");
        assert_eq!(
            a.addr, b.addr,
            "ensure_local_http must be a process-wide singleton"
        );
        assert_eq!(a.token, b.token, "the token must be stable across calls");
    }

    #[test]
    fn mint_token_is_long_and_unique() {
        let t1 = mint_token();
        let t2 = mint_token();
        assert_eq!(t1.len(), 64, "two simple UUIDs → 64 hex chars");
        assert_ne!(t1, t2, "tokens must be random per mint");
    }
}
