//! Raw-line oriented E2E coverage for the connectivity domain.
//!
//! The public JSON-RPC surface is intentionally small (`connectivity_diag`),
//! while the module also owns embedded-core port selection. These tests drive
//! both through exported production APIs so the E2E lcov captures the real
//! success and error branches.

use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::connectivity::ops::is_port_in_use;
use openhuman_core::openhuman::connectivity::rpc::{
    diag, pick_listen_port, pick_listen_port_for_host, PickListenPortError,
};
use openhuman_core::openhuman::connectivity::{
    all_connectivity_controller_schemas, all_connectivity_registered_controllers,
    connectivity_controller_schema,
};
use openhuman_core::openhuman::socket::{set_global_socket_manager, SocketManager};

const TEST_RPC_TOKEN: &str = "connectivity-raw-coverage-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

struct TestHarness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    rpc_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

struct ProbeListener {
    port: u16,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    join: tokio::task::JoinHandle<()>,
}

impl Drop for ProbeListener {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.join.abort();
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    let mutex = ENV_LOCK.get_or_init(|| Mutex::new(()));
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn ensure_rpc_auth() {
    AUTH_INIT.get_or_init(|| {
        std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);
        let token_dir = std::env::temp_dir().join("openhuman-connectivity-raw-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
}

async fn serve_rpc() -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });
    (addr, join)
}

fn write_min_config(openhuman_dir: &Path) {
    std::fs::create_dir_all(openhuman_dir).expect("create .openhuman");
    std::fs::write(
        openhuman_dir.join("config.toml"),
        r#"api_url = "http://127.0.0.1:9"
default_model = "e2e-model"

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0
"#,
    )
    .expect("write config.toml");
}

async fn setup() -> TestHarness {
    let tmp = tempdir().expect("tempdir");
    let openhuman_dir = tmp.path().join(".openhuman");
    write_min_config(&openhuman_dir);
    let guards = vec![
        EnvVarGuard::set_to_path("OPENHUMAN_HOME", &openhuman_dir),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path()),
        EnvVarGuard::set("OPENHUMAN_API_URL", "http://127.0.0.1:9"),
        EnvVarGuard::set("OPENHUMAN_SECRETS_ENCRYPT", "false"),
        EnvVarGuard::unset("OPENHUMAN_CORE_RPC_URL"),
        EnvVarGuard::unset("OPENHUMAN_CORE_PORT"),
    ];
    let (addr, rpc_join) = serve_rpc().await;
    TestHarness {
        _tmp: tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}/rpc"),
        rpc_join,
    }
}

async fn rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("client");
    let response = client
        .post(rpc_base)
        .bearer_auth(TEST_RPC_TOKEN)
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .expect("send rpc");
    assert_eq!(response.status(), StatusCode::OK, "rpc status for {method}");
    response.json().await.expect("rpc json")
}

fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    value
        .get("result")
        .and_then(|r| r.get("payload").or_else(|| r.get("result")))
        .unwrap_or_else(|| panic!("{context} should include result.payload: {value}"))
}

fn reserve_port() -> StdTcpListener {
    StdTcpListener::bind("127.0.0.1:0").expect("reserve ephemeral port")
}

fn reserve_contiguous_ports(count: usize) -> Option<Vec<StdTcpListener>> {
    const FIRST_CANDIDATE_PORT: u16 = 20_000;
    const LAST_CANDIDATE_PORT: u16 = 60_000;

    for first in FIRST_CANDIDATE_PORT..=LAST_CANDIDATE_PORT {
        let Some(last) = first.checked_add(count.saturating_sub(1) as u16) else {
            break;
        };
        if last > LAST_CANDIDATE_PORT {
            break;
        }

        let mut listeners = Vec::with_capacity(count);
        let mut reserved = true;
        for port in first..=last {
            match StdTcpListener::bind(("127.0.0.1", port)) {
                Ok(listener) => listeners.push(listener),
                Err(_) => {
                    reserved = false;
                    break;
                }
            }
        }

        if reserved {
            return Some(listeners);
        }
    }

    None
}

async fn spawn_probe_listener(status: &str, body: &'static str) -> ProbeListener {
    spawn_probe_listener_on("127.0.0.1", status, body).await
}

async fn spawn_probe_listener_on(host: &str, status: &str, body: &'static str) -> ProbeListener {
    let listener = tokio::net::TcpListener::bind((host, 0))
        .await
        .expect("bind probe listener");
    spawn_probe_listener_from(listener, status, body)
}

async fn try_spawn_probe_listener_on(
    host: &str,
    status: &str,
    body: &'static str,
) -> Option<ProbeListener> {
    let listener = tokio::net::TcpListener::bind((host, 0)).await.ok()?;
    Some(spawn_probe_listener_from(listener, status, body))
}

fn spawn_probe_listener_from(
    listener: tokio::net::TcpListener,
    status: &str,
    body: &'static str,
) -> ProbeListener {
    let port = listener.local_addr().expect("probe addr").port();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let status = status.to_string();

    let join = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((mut stream, _addr)) = accepted else {
                        break;
                    };
                    let mut req_buf = [0u8; 1024];
                    let _ = stream.read(&mut req_buf).await;
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                }
            }
        }
    });

    ProbeListener {
        port,
        shutdown: Some(shutdown_tx),
        join,
    }
}

#[tokio::test]
async fn connectivity_diag_rpc_reports_live_listener_port_and_process() {
    let _lock = env_lock();
    let harness = setup().await;
    let rpc_port = harness
        .rpc_base
        .parse::<url::Url>()
        .expect("rpc url")
        .port()
        .expect("rpc port");
    let _core_port = EnvVarGuard::set("OPENHUMAN_CORE_PORT", &rpc_port.to_string());

    let diag_result = rpc(
        &harness.rpc_base,
        91_001,
        "openhuman.connectivity_diag",
        json!({}),
    )
    .await;
    let diag_payload = payload(&diag_result, "connectivity_diag")
        .get("diag")
        .unwrap_or_else(|| panic!("diag payload missing: {diag_result}"));

    assert_eq!(diag_payload["listen_port"], json!(rpc_port));
    assert_eq!(diag_payload["listen_port_in_use"], json!(true));
    assert!(
        diag_payload["socket_state"] == json!("uninitialized")
            || diag_payload["socket_state"] == json!("disconnected"),
        "unexpected socket state: {diag_payload}"
    );
    assert_eq!(
        diag_payload["sidecar_pid"],
        json!(u64::from(std::process::id()))
    );

    harness.rpc_join.abort();
}

#[tokio::test]
async fn connectivity_diag_direct_path_prefers_rpc_url_and_handles_invalid_port_env() {
    let _lock = env_lock();
    let listener = reserve_port();
    let port = listener.local_addr().expect("reserved addr").port();
    let _rpc_url = EnvVarGuard::set(
        "OPENHUMAN_CORE_RPC_URL",
        &format!("http://127.0.0.1:{port}/rpc"),
    );
    let _port_env = EnvVarGuard::set("OPENHUMAN_CORE_PORT", "not-a-port");

    let outcome = diag().await.expect("diag should serialize");
    let value = outcome
        .into_cli_compatible_json()
        .expect("diag cli-compatible json");
    let diag_payload = value
        .get("payload")
        .or_else(|| value.get("result"))
        .and_then(|p| p.get("diag"))
        .unwrap_or_else(|| panic!("diag payload missing: {value}"));

    assert_eq!(diag_payload["listen_port"], json!(port));
    assert_eq!(diag_payload["listen_port_in_use"], json!(true));
    assert_eq!(diag_payload["last_ws_error"], Value::Null);
    drop(listener);

    drop(_rpc_url);
    let _invalid_port = EnvVarGuard::set("OPENHUMAN_CORE_PORT", "still-not-a-port");
    let fallback_default = diag()
        .await
        .expect("diag with invalid port env")
        .into_cli_compatible_json()
        .expect("invalid port diag json");
    assert_eq!(
        fallback_default
            .get("result")
            .and_then(|p| p.get("diag"))
            .and_then(|d| d.get("listen_port")),
        Some(&json!(7788))
    );

    drop(_invalid_port);
    let _valid_port = EnvVarGuard::set("OPENHUMAN_CORE_PORT", &port.to_string());
    let env_port = diag()
        .await
        .expect("diag with valid port env")
        .into_cli_compatible_json()
        .expect("valid port diag json");
    assert_eq!(
        env_port
            .get("result")
            .and_then(|p| p.get("diag"))
            .and_then(|d| d.get("listen_port")),
        Some(&json!(port))
    );

    drop(_valid_port);
    let _url_without_port = EnvVarGuard::set("OPENHUMAN_CORE_RPC_URL", "http://127.0.0.1/rpc");
    let _fallback_port = EnvVarGuard::set("OPENHUMAN_CORE_PORT", &port.to_string());
    let url_without_port = diag()
        .await
        .expect("diag should fall through URL without explicit port")
        .into_cli_compatible_json()
        .expect("url without port diag json");
    assert_eq!(
        url_without_port
            .get("result")
            .and_then(|p| p.get("diag"))
            .and_then(|d| d.get("listen_port")),
        Some(&json!(port))
    );
}

#[tokio::test]
async fn connectivity_ops_schema_and_socket_snapshot_paths_are_exercised() {
    let _lock = env_lock();
    let reserved = reserve_port();
    let port = reserved.local_addr().expect("reserved addr").port();
    assert!(is_port_in_use(port));
    drop(reserved);
    assert!(!is_port_in_use(port));

    let schemas = all_connectivity_controller_schemas();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].namespace, "connectivity");
    assert_eq!(schemas[0].function, "diag");
    assert_eq!(schemas[0].outputs[0].name, "diag");

    let unknown = connectivity_controller_schema("missing");
    assert_eq!(unknown.namespace, "connectivity");
    assert_eq!(unknown.function, "unknown");
    assert_eq!(unknown.outputs[0].name, "error");
    assert!(unknown.description.contains("Unknown connectivity"));

    set_global_socket_manager(std::sync::Arc::new(SocketManager::new()));
    let _core_port = EnvVarGuard::set("OPENHUMAN_CORE_PORT", &port.to_string());
    let value = diag()
        .await
        .expect("diag with socket manager")
        .into_cli_compatible_json()
        .expect("socket manager diag json");
    assert_eq!(
        value
            .get("result")
            .and_then(|p| p.get("diag"))
            .and_then(|d| d.get("socket_state")),
        Some(&json!("disconnected"))
    );

    let registered = all_connectivity_registered_controllers();
    assert_eq!(registered.len(), 1);
    assert_eq!(
        registered[0].rpc_method_name(),
        "openhuman.connectivity_diag"
    );
    let handled = (registered[0].handler)(serde_json::Map::new())
        .await
        .expect("registered connectivity handler");
    assert_eq!(
        handled
            .get("result")
            .and_then(|p| p.get("diag"))
            .and_then(|d| d.get("listen_port")),
        Some(&json!(port))
    );
}

#[tokio::test]
async fn pick_listen_port_covers_preferred_free_wrapper_retry_and_bind_failure() {
    let _lock = env_lock();
    let reserved = reserve_port();
    let free_port = reserved.local_addr().expect("reserved addr").port();
    drop(reserved);

    let picked = pick_listen_port(free_port)
        .await
        .expect("preferred port should bind");
    assert_eq!(picked.port, free_port);
    assert_eq!(picked.fallback_from, None);
    drop(picked.listener);

    let transient_listener = StdTcpListener::bind("127.0.0.1:0").expect("bind transient port");
    let transient_port = transient_listener
        .local_addr()
        .expect("transient addr")
        .port();
    let release = tokio::task::spawn_blocking(move || {
        std::thread::sleep(std::time::Duration::from_millis(650));
        drop(transient_listener);
    });
    let picked = pick_listen_port_for_host("127.0.0.1", transient_port)
        .await
        .expect("preferred port should bind after transient release");
    assert_eq!(picked.port, transient_port);
    assert_eq!(picked.fallback_from, None);
    drop(picked.listener);
    release.await.expect("release task ok");

    let err = pick_listen_port_for_host("192.0.2.1", 7788)
        .await
        .expect_err("non-local bind should fail");
    assert!(
        matches!(err, PickListenPortError::BindFailed { .. }),
        "expected bind failure, got {err:?}"
    );
    assert!(err
        .to_string()
        .contains("failed to bind core listener on port"));
}

#[tokio::test]
async fn pick_listen_port_detects_openhuman_listener_for_takeover() {
    let _lock = env_lock();
    let probe = spawn_probe_listener("200 OK", r#"{"name":"openhuman","ok":true}"#).await;

    let err = pick_listen_port_for_host("127.0.0.1", probe.port)
        .await
        .expect_err("openhuman listener should request takeover");
    assert!(err.to_string().contains("stale-listener takeover required"));
    match err {
        PickListenPortError::WouldTakeOver {
            preferred,
            fingerprint,
        } => {
            assert_eq!(preferred, probe.port);
            assert_eq!(fingerprint, "openhuman-core");
        }
        other => panic!("expected takeover error, got {other:?}"),
    }
}

#[tokio::test]
async fn pick_listen_port_falls_back_for_non_openhuman_and_status_fingerprints() {
    let _lock = env_lock();
    let probe = spawn_probe_listener("200 OK", r#"{"name":"not-openhuman"}"#).await;

    let picked = pick_listen_port_for_host("127.0.0.1", probe.port)
        .await
        .expect("non-openhuman listener should fall back");
    assert!(
        picked.port > probe.port,
        "fallback port must be higher than probe port"
    );
    assert_eq!(picked.fallback_from, Some(probe.port));
    drop(picked.listener);
    drop(probe);

    let status_probe = spawn_probe_listener("503 Service Unavailable", r#"unavailable"#).await;
    let picked = pick_listen_port_for_host("127.0.0.1", status_probe.port)
        .await
        .expect("non-success probe should fall back");
    assert!(
        picked.port > status_probe.port,
        "fallback port must be higher than status probe port"
    );
    assert_eq!(picked.fallback_from, Some(status_probe.port));
    drop(picked.listener);

    let invalid_body_probe = spawn_probe_listener_on("0.0.0.0", "200 OK", "not json").await;
    let picked = pick_listen_port_for_host("0.0.0.0", invalid_body_probe.port)
        .await
        .expect("invalid root JSON should fall back");
    assert_eq!(picked.fallback_from, Some(invalid_body_probe.port));
    drop(picked.listener);

    let raw_listener = StdTcpListener::bind("127.0.0.1:0").expect("bind raw listener");
    let raw_port = raw_listener.local_addr().expect("raw listener addr").port();
    let picked = pick_listen_port_for_host("127.0.0.1", raw_port)
        .await
        .expect("raw TCP listener should be classified as other and fall back");
    assert_eq!(picked.fallback_from, Some(raw_port));
    drop(picked.listener);
    drop(raw_listener);
}

#[tokio::test]
async fn pick_listen_port_identifies_ipv6_openhuman_listener_when_supported() {
    let _lock = env_lock();
    let Some(probe) =
        try_spawn_probe_listener_on("::1", "200 OK", r#"{"name":"openhuman","ok":true}"#).await
    else {
        eprintln!("IPv6 loopback unavailable; skipping IPv6 connectivity probe coverage");
        return;
    };

    let err = pick_listen_port_for_host("::1", probe.port)
        .await
        .expect_err("IPv6 openhuman listener should request takeover");
    match err {
        PickListenPortError::WouldTakeOver {
            preferred,
            fingerprint,
        } => {
            assert_eq!(preferred, probe.port);
            assert_eq!(fingerprint, "openhuman-core");
        }
        other => panic!("expected IPv6 takeover error, got {other:?}"),
    }
}

#[tokio::test]
async fn pick_listen_port_reports_no_available_fallbacks() {
    let _lock = env_lock();
    let mut reserved =
        reserve_contiguous_ports(11).expect("reserve preferred port and ten fallback ports");
    let preferred_std = reserved.remove(0);
    preferred_std
        .set_nonblocking(true)
        .expect("mark preferred probe listener nonblocking");
    let preferred_listener =
        tokio::net::TcpListener::from_std(preferred_std).expect("convert preferred probe listener");
    let preferred_probe =
        spawn_probe_listener_from(preferred_listener, "200 OK", r#"{"name":"not-openhuman"}"#);
    let preferred = preferred_probe.port;
    let occupied = reserved;

    let err = pick_listen_port_for_host("127.0.0.1", preferred)
        .await
        .expect_err("all fallback ports should be exhausted");
    assert!(
        err.to_string().contains("no fallback ports available"),
        "display should include fallback exhaustion detail: {err}"
    );
    match err {
        PickListenPortError::NoAvailablePort {
            preferred: actual_preferred,
            fingerprint,
            attempted,
        } => {
            assert_eq!(actual_preferred, preferred);
            assert!(
                fingerprint.contains("did not identify as openhuman"),
                "unexpected fingerprint: {fingerprint}"
            );
            assert_eq!(
                attempted,
                ((preferred + 1)..=(preferred + 10)).collect::<Vec<_>>()
            );
        }
        other => panic!("expected no available port error, got {other:?}"),
    }

    drop(occupied);
}
