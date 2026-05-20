//! `openhuman.connectivity_diag` RPC.
//!
//! Returns a snapshot of the local sidecar's process id + RPC port + backend
//! Socket.IO state, so the frontend's coreHealthMonitor can prove "the local
//! core is alive" without conflating that signal with the backend websocket
//! or the browser's internet connectivity. See issue #1527.

use serde::Serialize;
use serde_json::json;
use std::fmt;
use std::io::ErrorKind;
use tokio::net::TcpListener;
use tokio::time::{sleep, Duration};
use tracing::{debug, warn};

use crate::openhuman::socket::manager::global_socket_manager;
use crate::rpc::RpcOutcome;

use super::ops::is_port_in_use;

const DEFAULT_CORE_PORT: u16 = 7788;
const DEFAULT_FALLBACK_START: u16 = 7789;
const DEFAULT_FALLBACK_END: u16 = 7798;

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

/// Successful bind selection for the embedded core HTTP listener.
#[derive(Debug)]
pub struct PickListenPortResult {
    pub listener: TcpListener,
    pub port: u16,
    /// Present when the preferred port was occupied and we moved to another
    /// port in the fallback pool.
    pub fallback_from: Option<u16>,
}

#[derive(Debug, Clone, Copy)]
struct RetryPolicy {
    attempts: usize,
    backoff: Duration,
}

impl RetryPolicy {
    const DEFAULT: Self = Self {
        attempts: 3,
        backoff: Duration::from_millis(500),
    };
}

#[derive(Debug, Clone)]
enum ListenerFingerprint {
    OpenHumanCore,
    Other(String),
}

impl ListenerFingerprint {
    fn as_human_readable(&self) -> String {
        match self {
            Self::OpenHumanCore => "openhuman-core".to_string(),
            Self::Other(reason) => reason.clone(),
        }
    }
}

/// Failure modes for preferred-port selection.
#[derive(Debug, Clone)]
pub enum PickListenPortError {
    /// Port is occupied by another OpenHuman core; caller should run the stale
    /// listener takeover flow (#1130) before retrying startup.
    WouldTakeOver { preferred: u16, fingerprint: String },
    /// No candidate port was available after trying the fallback pool.
    NoAvailablePort {
        preferred: u16,
        fingerprint: String,
        attempted: Vec<u16>,
    },
    /// Bind failed with a non-AddrInUse error.
    BindFailed { port: u16, reason: String },
}

impl fmt::Display for PickListenPortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WouldTakeOver {
                preferred,
                fingerprint,
            } => write!(
                f,
                "preferred core port {preferred} is occupied by {fingerprint}; stale-listener takeover required"
            ),
            Self::NoAvailablePort {
                preferred,
                fingerprint,
                attempted,
            } => write!(
                f,
                "preferred core port {preferred} is occupied by {fingerprint}; no fallback ports available in {:?}",
                attempted
            ),
            Self::BindFailed { port, reason } => {
                write!(f, "failed to bind core listener on port {port}: {reason}")
            }
        }
    }
}

impl std::error::Error for PickListenPortError {}

/// Pick a listen port for the embedded core listener on loopback.
///
/// Behavior:
/// - first tries `preferred`
/// - retries transient `AddrInUse` races a few times
/// - if still occupied by another OpenHuman core, asks caller to takeover
/// - otherwise falls back to ports 7789..=7798
pub async fn pick_listen_port(preferred: u16) -> Result<PickListenPortResult, PickListenPortError> {
    pick_listen_port_for_host("127.0.0.1", preferred).await
}

/// Same as [`pick_listen_port`] but allows an explicit host (used by the core
/// server bootstrap so CLI host overrides remain respected).
pub async fn pick_listen_port_for_host(
    host: &str,
    preferred: u16,
) -> Result<PickListenPortResult, PickListenPortError> {
    let fallbacks: Vec<u16> = if preferred == DEFAULT_CORE_PORT {
        (DEFAULT_FALLBACK_START..=DEFAULT_FALLBACK_END).collect()
    } else {
        (1..=10)
            .filter_map(|delta| preferred.checked_add(delta))
            .collect()
    };
    pick_listen_port_with_policy(host, preferred, &fallbacks, RetryPolicy::DEFAULT).await
}

async fn pick_listen_port_with_policy(
    host: &str,
    preferred: u16,
    fallback_ports: &[u16],
    retry_policy: RetryPolicy,
) -> Result<PickListenPortResult, PickListenPortError> {
    match TcpListener::bind((host, preferred)).await {
        Ok(listener) => {
            return Ok(PickListenPortResult {
                listener,
                port: preferred,
                fallback_from: None,
            });
        }
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            // Retry transient bind races before we decide whether this needs
            // stale-listener takeover (#1130) or a fallback port.
            for _ in 0..retry_policy.attempts {
                sleep(retry_policy.backoff).await;
                match TcpListener::bind((host, preferred)).await {
                    Ok(listener) => {
                        return Ok(PickListenPortResult {
                            listener,
                            port: preferred,
                            fallback_from: None,
                        });
                    }
                    Err(retry_err) if retry_err.kind() == ErrorKind::AddrInUse => {}
                    Err(retry_err) => {
                        return Err(PickListenPortError::BindFailed {
                            port: preferred,
                            reason: retry_err.to_string(),
                        });
                    }
                }
            }
        }
        Err(err) => {
            return Err(PickListenPortError::BindFailed {
                port: preferred,
                reason: err.to_string(),
            });
        }
    }

    let fingerprint = identify_listener(host, preferred).await;
    if matches!(fingerprint, ListenerFingerprint::OpenHumanCore) {
        return Err(PickListenPortError::WouldTakeOver {
            preferred,
            fingerprint: fingerprint.as_human_readable(),
        });
    }

    for fallback in fallback_ports {
        // Retry each fallback candidate on transient AddrInUse so a brief
        // race on 7789–7798 (AV scanner / prior-instance teardown) doesn't
        // surface as NoAvailablePort. Mirrors the preferred-port retry above.
        let mut bound: Option<TcpListener> = None;
        for attempt in 0..=retry_policy.attempts {
            match TcpListener::bind((host, *fallback)).await {
                Ok(listener) => {
                    bound = Some(listener);
                    break;
                }
                Err(err) if err.kind() == ErrorKind::AddrInUse => {
                    if attempt < retry_policy.attempts {
                        sleep(retry_policy.backoff).await;
                        continue;
                    }
                }
                Err(err) => {
                    debug!(
                        "[connectivity][rpc] fallback bind failed on {}:{}: {}",
                        host, fallback, err
                    );
                    break;
                }
            }
        }
        if let Some(listener) = bound {
            warn!(
                "[CORE] preferred port {} in use by {}; bound to {}",
                preferred,
                fingerprint.as_human_readable(),
                fallback
            );
            return Ok(PickListenPortResult {
                listener,
                port: *fallback,
                fallback_from: Some(preferred),
            });
        }
    }

    Err(PickListenPortError::NoAvailablePort {
        preferred,
        fingerprint: fingerprint.as_human_readable(),
        attempted: fallback_ports.to_vec(),
    })
}

async fn identify_listener(host: &str, port: u16) -> ListenerFingerprint {
    let probe_host = if host == "0.0.0.0" || host == "::" {
        "127.0.0.1"
    } else {
        host
    };
    // IPv6 literals must be bracketed in the URL authority per RFC 3986; an
    // un-bracketed `http://::1:7788/` parses the colons as host:port and
    // mis-classifies live OpenHuman cores on IPv6 hosts as `Other`.
    let authority = if probe_host.contains(':') && !probe_host.starts_with('[') {
        format!("[{probe_host}]")
    } else {
        probe_host.to_string()
    };
    let url = format!("http://{authority}:{port}/");
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(750))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            return ListenerFingerprint::Other(format!("probe client build failed: {err}"));
        }
    };

    let response = match client.get(&url).send().await {
        Ok(resp) => resp,
        Err(err) => {
            return ListenerFingerprint::Other(format!("probe GET / failed: {err}"));
        }
    };

    if !response.status().is_success() {
        return ListenerFingerprint::Other(format!(
            "probe GET / returned status {}",
            response.status()
        ));
    }

    let body = match response.text().await {
        Ok(text) => text,
        Err(err) => {
            return ListenerFingerprint::Other(format!("probe body read failed: {err}"));
        }
    };

    if is_openhuman_root_body(&body) {
        ListenerFingerprint::OpenHumanCore
    } else {
        let preview: String = body.chars().take(80).collect();
        ListenerFingerprint::Other(format!(
            "probe body did not identify as openhuman ({preview:?})"
        ))
    }
}

fn is_openhuman_root_body(body: &str) -> bool {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return false,
    };
    value
        .get("name")
        .and_then(|v| v.as_str())
        .map(|name| name == "openhuman")
        .unwrap_or(false)
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serialize env-var mutation across the three `resolve_listen_port_*`
    /// tests so they don't race each other under Rust's default parallel
    /// runner. Process-global env state means one test's restore can land
    /// in another test's read window without this. Same pattern used in
    /// `webview_accounts/ops.rs` and `tools/impl/system/lsp.rs`.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn reserve_port() -> std::net::TcpListener {
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral test port")
    }

    async fn spawn_openhuman_probe_listener(
        port: u16,
    ) -> (
        tokio::task::JoinHandle<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .expect("bind probe listener");
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((mut stream, _addr)) = accepted else {
                            break;
                        };
                        let mut req_buf = [0u8; 1024];
                        let _ = stream.read(&mut req_buf).await;
                        let body = r#"{"name":"openhuman","ok":true}"#;
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    }
                }
            }
        });

        (task, shutdown_tx)
    }

    #[tokio::test]
    async fn pick_listen_port_preferred_free() {
        let holder = reserve_port();
        let preferred = holder.local_addr().expect("preferred local addr").port();
        drop(holder);

        let result = pick_listen_port_with_policy(
            "127.0.0.1",
            preferred,
            &[],
            RetryPolicy {
                attempts: 0,
                backoff: Duration::from_millis(1),
            },
        )
        .await
        .expect("preferred bind should succeed");

        assert_eq!(result.port, preferred);
        assert_eq!(result.fallback_from, None);
    }

    #[tokio::test]
    async fn pick_listen_port_openhuman_listener_requests_takeover() {
        let holder = reserve_port();
        let preferred = holder.local_addr().expect("preferred local addr").port();
        drop(holder);

        let (server_task, shutdown_tx) = spawn_openhuman_probe_listener(preferred).await;

        let result = pick_listen_port_with_policy(
            "127.0.0.1",
            preferred,
            &[],
            RetryPolicy {
                attempts: 1,
                backoff: Duration::from_millis(10),
            },
        )
        .await;

        let err = result.expect_err("openhuman listener should trigger takeover");
        assert!(
            matches!(err, PickListenPortError::WouldTakeOver { preferred: p, .. } if p == preferred),
            "expected WouldTakeOver for preferred port, got: {err:?}"
        );

        let _ = shutdown_tx.send(());
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn pick_listen_port_other_listener_falls_back() {
        let preferred_listener = reserve_port();
        let preferred = preferred_listener
            .local_addr()
            .expect("preferred local addr")
            .port();
        let busy_fallback_listener = reserve_port();
        let busy_fallback = busy_fallback_listener
            .local_addr()
            .expect("busy fallback local addr")
            .port();
        let free_fallback_holder = reserve_port();
        let free_fallback = free_fallback_holder
            .local_addr()
            .expect("free fallback local addr")
            .port();
        drop(free_fallback_holder);

        let result = pick_listen_port_with_policy(
            "127.0.0.1",
            preferred,
            &[busy_fallback, free_fallback],
            RetryPolicy {
                attempts: 1,
                backoff: Duration::from_millis(10),
            },
        )
        .await
        .expect("fallback bind should succeed");

        assert_eq!(result.port, free_fallback);
        assert_eq!(result.fallback_from, Some(preferred));
    }

    #[tokio::test]
    async fn pick_listen_port_all_candidates_busy_errors() {
        let preferred_listener = reserve_port();
        let preferred = preferred_listener
            .local_addr()
            .expect("preferred local addr")
            .port();
        let fallback1_listener = reserve_port();
        let fallback1 = fallback1_listener
            .local_addr()
            .expect("fallback1 local addr")
            .port();
        let fallback2_listener = reserve_port();
        let fallback2 = fallback2_listener
            .local_addr()
            .expect("fallback2 local addr")
            .port();

        let result = pick_listen_port_with_policy(
            "127.0.0.1",
            preferred,
            &[fallback1, fallback2],
            RetryPolicy {
                attempts: 1,
                backoff: Duration::from_millis(10),
            },
        )
        .await;

        let err = result.expect_err("all-busy path should fail");
        assert!(
            matches!(err, PickListenPortError::NoAvailablePort { preferred: p, ref attempted, .. } if p == preferred && attempted == &vec![fallback1, fallback2]),
            "expected NoAvailablePort with attempted fallback list, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn pick_listen_port_retries_transient_addr_in_use() {
        let preferred_listener = reserve_port();
        let preferred = preferred_listener
            .local_addr()
            .expect("preferred local addr")
            .port();
        let release_task = tokio::spawn(async move {
            sleep(Duration::from_millis(25)).await;
            drop(preferred_listener);
        });

        let result = pick_listen_port_with_policy(
            "127.0.0.1",
            preferred,
            &[],
            RetryPolicy {
                attempts: 6,
                backoff: Duration::from_millis(10),
            },
        )
        .await
        .expect("transient in-use should recover to preferred port");

        release_task.await.expect("release task");
        assert_eq!(result.port, preferred);
        assert_eq!(result.fallback_from, None);
    }

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
        let prev_port = std::env::var("OPENHUMAN_CORE_PORT").ok();
        // resolve_listen_port() also reads OPENHUMAN_CORE_RPC_URL ahead of
        // OPENHUMAN_CORE_PORT, so an inherited URL from the runner would
        // make this assertion nondeterministic. Save + clear both.
        let prev_url = std::env::var("OPENHUMAN_CORE_RPC_URL").ok();
        // SAFETY: standard Rust test pattern — env access is unsafe in 2024
        // edition because it isn't thread-safe. Tests are single-threaded
        // for this scope and we restore in the same body.
        unsafe {
            std::env::remove_var("OPENHUMAN_CORE_PORT");
            std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
        }
        assert_eq!(resolve_listen_port(), DEFAULT_CORE_PORT);
        if let Some(value) = prev_port {
            unsafe {
                std::env::set_var("OPENHUMAN_CORE_PORT", value);
            }
        }
        if let Some(value) = prev_url {
            unsafe {
                std::env::set_var("OPENHUMAN_CORE_RPC_URL", value);
            }
        }
    }

    #[test]
    fn resolve_listen_port_honours_env_override() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev_port = std::env::var("OPENHUMAN_CORE_PORT").ok();
        let prev_url = std::env::var("OPENHUMAN_CORE_RPC_URL").ok();
        unsafe {
            // Clear OPENHUMAN_CORE_RPC_URL so OPENHUMAN_CORE_PORT is the
            // resolved value (URL has higher priority in resolve_listen_port).
            std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
            std::env::set_var("OPENHUMAN_CORE_PORT", "65000");
        }
        assert_eq!(resolve_listen_port(), 65000);
        match prev_port {
            Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_PORT", value) },
            None => unsafe { std::env::remove_var("OPENHUMAN_CORE_PORT") },
        }
        match prev_url {
            Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_RPC_URL", value) },
            None => unsafe { std::env::remove_var("OPENHUMAN_CORE_RPC_URL") },
        }
    }

    #[test]
    fn resolve_listen_port_falls_back_on_invalid_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev_port = std::env::var("OPENHUMAN_CORE_PORT").ok();
        let prev_url = std::env::var("OPENHUMAN_CORE_RPC_URL").ok();
        unsafe {
            std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
            std::env::set_var("OPENHUMAN_CORE_PORT", "not-a-number");
        }
        assert_eq!(resolve_listen_port(), DEFAULT_CORE_PORT);
        match prev_port {
            Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_PORT", value) },
            None => unsafe { std::env::remove_var("OPENHUMAN_CORE_PORT") },
        }
        match prev_url {
            Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_RPC_URL", value) },
            None => unsafe { std::env::remove_var("OPENHUMAN_CORE_RPC_URL") },
        }
    }

    #[test]
    fn resolve_listen_port_prefers_openhuman_core_rpc_url() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev_rpc = std::env::var("OPENHUMAN_CORE_RPC_URL").ok();
        let prev_port = std::env::var("OPENHUMAN_CORE_PORT").ok();
        unsafe {
            std::env::set_var("OPENHUMAN_CORE_RPC_URL", "http://127.0.0.1:7794/rpc");
            std::env::set_var("OPENHUMAN_CORE_PORT", "7788");
        }
        assert_eq!(resolve_listen_port(), 7794);
        match prev_rpc {
            Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_RPC_URL", value) },
            None => unsafe { std::env::remove_var("OPENHUMAN_CORE_RPC_URL") },
        }
        match prev_port {
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
