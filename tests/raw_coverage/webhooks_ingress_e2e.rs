//! End-to-end coverage for the `webhooks` RPC namespace (13 controllers, 0% before this file).
//!
//! Two independent halves, because the namespace has two independent backends:
//!
//! 1. **Router-local** (`list_registrations`, `register_echo`, `unregister_echo`,
//!    `register_agent`, `trigger_agent`, `list_logs`, `clear_logs`) — served out of the
//!    `WebhookRouter` hanging off the global `SocketManager`. These tests install a real
//!    `SocketManager` + `WebhookRouter` (persisting to a tempdir) and drive the registry through
//!    the JSON-RPC surface, asserting on the registration rows and the debug-log ring that come
//!    back.
//!
//! 2. **Backend tunnel CRUD** (`list_tunnels`, `create_tunnel`, `get_tunnel`, `update_tunnel`,
//!    `delete_tunnel`, `get_bandwidth`) — thin adapters over `/webhooks/core*` on the hosted
//!    backend. These run against an in-process axum mock that records what the core actually
//!    sent, so the tests assert on the *request* the adapter built (method, path, body key
//!    casing) as well as the response it surfaced.
//!
//! Shape follows `tests/composio_post_oauth_retry_e2e.rs`: an env lock (HOME / BACKEND_URL are
//! process-global), an ephemeral mock backend, an ephemeral core JSON-RPC server, and
//! `auth_store_session` to mint the JWT the tunnel adapters require.
//!
//! This file is a **module** of the aggregated `raw_coverage_all` target (globbed in by
//! `build.rs`), not its own binary — so every suite in that binary shares one process and libtest
//! runs them concurrently. Two consequences are load-bearing here:
//!
//!   * env mutation binds to `crate::SHARED_ENV_LOCK`, not a lock private to this file;
//!   * `set_global_socket_manager` writes a `OnceLock` that
//!     `connectivity_raw_coverage_e2e.rs` also writes. Whoever gets there first owns the
//!     manager, so `shared_router` attaches the router to whichever instance won rather than
//!     assuming its own was installed.
//!
//! Run with:
//!   ~/tinyhuman/ci-slot.sh cargo test --test raw_coverage_all \
//!       --features "$(bash scripts/ci/product-features.sh)" webhooks_ingress

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Path as AxumPath, State};
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::auth::{get_rpc_token, init_rpc_token};
use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::platform::socket::{
    global_socket_manager, set_global_socket_manager, SocketManager,
};
use openhuman_core::openhuman::skills::webhooks::{WebhookRequest, WebhookRouter};

// ── env serialisation ────────────────────────────────────────────────────────
//
// HOME / BACKEND_URL / VITE_BACKEND_URL are process-global and this file shares its process with
// ~76 sibling suites. A lock private to this file would compile, pass in isolation, and race them
// under load — so bind to the crate-wide one.

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn webhooks_e2e_env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const TEST_JWT: &str = "e2e-webhooks-jwt";

/// The bearer every request in this file sends.
///
/// **Read back, never asserted.** `RPC_TOKEN` in `core::auth` is a process-global `OnceLock` and
/// this file shares its process with every other aggregated suite, several of which also call
/// `init_rpc_token`. Whichever runs first fixes the token for the whole binary and every later
/// `init_rpc_token` is a documented no-op — so a suite that hard-codes its own literal and sends
/// that would 401 whenever it lost the race. Initialising and then asking `get_rpc_token()` for
/// the value that actually took is correct either way round.
fn rpc_bearer() -> &'static str {
    static BEARER: OnceLock<&'static str> = OnceLock::new();
    BEARER.get_or_init(|| {
        let token_dir = std::env::temp_dir().join("openhuman-webhooks-e2e-auth");
        std::fs::create_dir_all(&token_dir).expect("rpc token dir");
        init_rpc_token(&token_dir).expect("init rpc token for webhooks_ingress_e2e");
        get_rpc_token().expect("an RPC token is initialised for this process")
    })
}

fn ensure_rpc_auth() {
    let _ = rpc_bearer();
}

/// The `WebhookRouter` the webhook ops resolve through `global_socket_manager()`.
///
/// `set_global_socket_manager` writes a `OnceLock`, and `connectivity_raw_coverage_e2e.rs`
/// installs a bare `SocketManager` of its own into the same slot. Whichever suite runs first
/// wins and the other's `set` is a logged no-op — so this offers a manager, then reads back
/// *whatever* is actually installed and attaches the router to that. `set_webhook_router` takes
/// `&self` and writes an `RwLock`, so this works on a manager we did not create.
///
/// The router is process-wide as a result, so each case below uses tunnel UUIDs unique to itself.
fn shared_router() -> &'static Arc<WebhookRouter> {
    static ROUTER: OnceLock<Arc<WebhookRouter>> = OnceLock::new();
    ROUTER.get_or_init(|| {
        let tmp = tempdir().expect("router persist tempdir");
        let path = tmp.path().join("webhook_routes.json");
        // Outlives the binary; the OS reaps the tmpdir.
        std::mem::forget(tmp);
        let router = Arc::new(WebhookRouter::new(Some(path)));
        set_global_socket_manager(Arc::new(SocketManager::new()));
        let installed = global_socket_manager()
            .expect("a global SocketManager is installed after set_global_socket_manager");
        installed.set_webhook_router(Arc::clone(&router));
        router
    })
}

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, prev }
    }

    fn unset(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

// ── mock backend ─────────────────────────────────────────────────────────────

/// One recorded inbound request to the mock backend.
#[derive(Clone, Debug)]
struct RecordedCall {
    method: String,
    path: String,
    body: Option<Value>,
}

#[derive(Clone, Default)]
struct BackendState {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl BackendState {
    fn record(&self, method: &str, path: &str, body: Option<Value>) {
        self.calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(RecordedCall {
                method: method.to_string(),
                path: path.to_string(),
                body,
            });
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

fn unauthorized() -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "success": false, "error": "unauthorized" })),
    )
}

fn is_authed(headers: &HeaderMap) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == format!("Bearer {TEST_JWT}"))
        .unwrap_or(false)
}

async fn mock_current_user(headers: HeaderMap) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !is_authed(&headers) {
        return Err(unauthorized());
    }
    Ok(Json(json!({
        "success": true,
        "data": { "_id": "webhooks-e2e-user", "username": "webhooks-e2e" }
    })))
}

fn tunnel_row(id: &str, name: &str, active: bool) -> Value {
    json!({
        "id": id,
        "uuid": format!("uuid-{id}"),
        "name": name,
        "isActive": active,
        "url": format!("https://tunnels.example/{id}"),
    })
}

async fn mock_list_tunnels(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !is_authed(&headers) {
        return Err(unauthorized());
    }
    state.record("GET", "/webhooks/core", None);
    Ok(Json(json!({
        "success": true,
        "data": { "tunnels": [tunnel_row("tun-1", "first", true)] }
    })))
}

async fn mock_create_tunnel(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !is_authed(&headers) {
        return Err(unauthorized());
    }
    state.record("POST", "/webhooks/core", Some(body.clone()));
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(Json(json!({
        "success": true,
        "data": tunnel_row("tun-created", &name, true)
    })))
}

async fn mock_get_tunnel(
    State(state): State<BackendState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !is_authed(&headers) {
        return Err(unauthorized());
    }
    state.record("GET", &format!("/webhooks/core/{id}"), None);
    Ok(Json(json!({
        "success": true,
        "data": tunnel_row(&id, "fetched", true)
    })))
}

async fn mock_patch_tunnel(
    State(state): State<BackendState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !is_authed(&headers) {
        return Err(unauthorized());
    }
    state.record("PATCH", &format!("/webhooks/core/{id}"), Some(body.clone()));
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unchanged");
    let active = body
        .get("isActive")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    Ok(Json(json!({
        "success": true,
        "data": tunnel_row(&id, name, active)
    })))
}

async fn mock_delete_tunnel(
    State(state): State<BackendState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !is_authed(&headers) {
        return Err(unauthorized());
    }
    state.record("DELETE", &format!("/webhooks/core/{id}"), None);
    Ok(Json(json!({ "success": true, "data": { "deleted": true, "id": id } })))
}

async fn mock_bandwidth(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !is_authed(&headers) {
        return Err(unauthorized());
    }
    state.record("GET", "/webhooks/core/bandwidth", None);
    Ok(Json(json!({
        "success": true,
        "data": { "bytesIn": 4096, "bytesOut": 8192, "limitBytes": 1_048_576 }
    })))
}

fn mock_backend_router(state: BackendState) -> Router {
    Router::new()
        .route("/settings", get(mock_current_user))
        .route("/auth/me", get(mock_current_user))
        // `/webhooks/core/bandwidth` must be declared before the `{id}` capture, otherwise the
        // capture swallows it and `get_bandwidth` silently reads a tunnel row.
        .route(
            "/webhooks/core/bandwidth",
            get(mock_bandwidth).with_state(state.clone()),
        )
        .route(
            "/webhooks/core",
            get(mock_list_tunnels)
                .post(mock_create_tunnel)
                .with_state(state.clone()),
        )
        .route(
            "/webhooks/core/{id}",
            get(mock_get_tunnel)
                .patch(mock_patch_tunnel)
                .delete(mock_delete_tunnel)
                .with_state(state),
        )
}

// ── infrastructure ───────────────────────────────────────────────────────────

async fn serve_ephemeral(app: Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    (addr, handle)
}

fn write_test_config(openhuman_dir: &Path, api_origin: &str) {
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "e2e-mock-model"
default_temperature = 0.7
chat_onboarding_completed = true

[secrets]
encrypt = false
"#
    );
    fn write_cfg(dir: &Path, cfg: &str) {
        std::fs::create_dir_all(dir).expect("mkdir config dir");
        std::fs::write(dir.join("config.toml"), cfg).expect("write config.toml");
    }
    write_cfg(openhuman_dir, &cfg);
    write_cfg(&openhuman_dir.join("users").join("local"), &cfg);
    write_cfg(&openhuman_dir.join("users").join("webhooks-e2e-user"), &cfg);
}

async fn post_json_rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("reqwest client");
    let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", rpc_bearer()))
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {url}: {e}"));
    assert!(
        resp.status().is_success(),
        "HTTP error {} calling {method}",
        resp.status()
    );
    resp.json::<Value>()
        .await
        .unwrap_or_else(|e| panic!("json parse for {method}: {e}"))
}

fn assert_no_jsonrpc_error<'a>(v: &'a Value, ctx: &str) -> &'a Value {
    if let Some(err) = v.get("error") {
        panic!("{ctx}: unexpected JSON-RPC error: {err}");
    }
    v.get("result")
        .unwrap_or_else(|| panic!("{ctx}: missing result field: {v}"))
}

fn jsonrpc_error_message(v: &Value, ctx: &str) -> String {
    let err = v
        .get("error")
        .unwrap_or_else(|| panic!("{ctx}: expected a JSON-RPC error, got: {v}"));
    err.get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{ctx}: error had no message: {err}"))
        .to_string()
}

/// Peel the `{"result": inner, "logs": [...]}` envelope `RpcOutcome` adds when logs are present.
fn peel<'a>(v: &'a Value) -> &'a Value {
    if v.get("logs").is_some() {
        v.get("result").unwrap_or(v)
    } else {
        v
    }
}

/// The registration rows for `tunnel_uuid`, from a `list_registrations`-shaped payload.
fn registration<'a>(result: &'a Value, tunnel_uuid: &str) -> Option<&'a Value> {
    result
        .get("registrations")?
        .as_array()?
        .iter()
        .find(|r| r.get("tunnel_uuid").and_then(Value::as_str) == Some(tunnel_uuid))
}

fn sample_webhook_request(correlation_id: &str, tunnel_uuid: &str) -> WebhookRequest {
    WebhookRequest {
        correlation_id: correlation_id.to_string(),
        tunnel_id: "backend-tunnel-1".to_string(),
        tunnel_uuid: tunnel_uuid.to_string(),
        tunnel_name: "logging-tunnel".to_string(),
        method: "POST".to_string(),
        path: "/hook".to_string(),
        headers: HashMap::new(),
        query: HashMap::new(),
        body: String::new(),
    }
}

// ── router-local registry ────────────────────────────────────────────────────

/// `register_echo` → `list_registrations` → `unregister_echo`, plus the two ownership guards
/// `register_target` enforces.
///
/// Covers: `openhuman.webhooks_register_echo`, `openhuman.webhooks_register_agent`,
/// `openhuman.webhooks_list_registrations`, `openhuman.webhooks_unregister_echo`.
#[tokio::test]
async fn webhooks_registration_lifecycle_and_ownership_guards() {
    let _env_lock = webhooks_e2e_env_lock();
    let _router = shared_router();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let _home_guard = EnvGuard::set_to_path("HOME", home);
    let _ws_guard = EnvGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_guard = EnvGuard::unset("BACKEND_URL");
    let _vite_guard = EnvGuard::unset("VITE_BACKEND_URL");

    let state = BackendState::default();
    let (mock_addr, mock_join) = serve_ephemeral(mock_backend_router(state)).await;
    write_test_config(&openhuman_home, &format!("http://{mock_addr}"));

    let (rpc_addr, rpc_join) = serve_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    let echo_uuid = "e2e-lifecycle-echo";
    let agent_uuid = "e2e-lifecycle-agent";

    // ── register_echo returns the *updated* registry, not just an ack.
    let registered = post_json_rpc(
        &rpc_base,
        7001,
        "openhuman.webhooks_register_echo",
        json!({ "tunnel_uuid": echo_uuid, "tunnel_name": "Echo Tunnel", "backend_tunnel_id": "bt-9" }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&registered, "webhooks_register_echo"));
    let row = registration(result, echo_uuid).expect("echo registration present in response");
    assert_eq!(
        row.get("target_kind").and_then(Value::as_str),
        Some("echo"),
        "register_echo must record target_kind=echo: {row}"
    );
    assert_eq!(row.get("skill_id").and_then(Value::as_str), Some("echo"));
    assert_eq!(
        row.get("tunnel_name").and_then(Value::as_str),
        Some("Echo Tunnel"),
        "the optional tunnel_name must be persisted, not dropped: {row}"
    );
    assert_eq!(
        row.get("backend_tunnel_id").and_then(Value::as_str),
        Some("bt-9")
    );

    // ── an agent registration on an echo-owned tunnel is refused by name.
    let conflict = post_json_rpc(
        &rpc_base,
        7002,
        "openhuman.webhooks_register_agent",
        json!({ "tunnel_uuid": echo_uuid, "agent_id": "agent-a" }),
    )
    .await;
    let message = jsonrpc_error_message(&conflict, "register_agent over an echo tunnel");
    assert!(
        message.contains(echo_uuid) && message.contains("already owned by"),
        "cross-target registration must be refused naming the owner, got: {message}"
    );

    // ── an agent tunnel binds its agent_id, and refuses a silent rebind.
    let agent = post_json_rpc(
        &rpc_base,
        7003,
        "openhuman.webhooks_register_agent",
        json!({ "tunnel_uuid": agent_uuid, "agent_id": "agent-a" }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&agent, "webhooks_register_agent"));
    let row = registration(result, agent_uuid).expect("agent registration present");
    assert_eq!(row.get("target_kind").and_then(Value::as_str), Some("agent"));
    assert_eq!(row.get("agent_id").and_then(Value::as_str), Some("agent-a"));

    let rebind = post_json_rpc(
        &rpc_base,
        7004,
        "openhuman.webhooks_register_agent",
        json!({ "tunnel_uuid": agent_uuid, "agent_id": "agent-b" }),
    )
    .await;
    let message = jsonrpc_error_message(&rebind, "agent rebind");
    assert!(
        message.contains("cannot rebind"),
        "rebinding an agent tunnel to a different agent must be refused, got: {message}"
    );

    // ── list_registrations sees both, with the values register_* stored.
    let listed = post_json_rpc(
        &rpc_base,
        7005,
        "openhuman.webhooks_list_registrations",
        json!({}),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(
        &listed,
        "webhooks_list_registrations",
    ));
    assert!(
        registration(result, echo_uuid).is_some() && registration(result, agent_uuid).is_some(),
        "list_registrations must return both tunnels: {result}"
    );

    // ── unregister_echo removes exactly the echo tunnel.
    let removed = post_json_rpc(
        &rpc_base,
        7006,
        "openhuman.webhooks_unregister_echo",
        json!({ "tunnel_uuid": echo_uuid }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(
        &removed,
        "webhooks_unregister_echo",
    ));
    assert!(
        registration(result, echo_uuid).is_none(),
        "unregister_echo must drop the echo tunnel: {result}"
    );
    assert!(
        registration(result, agent_uuid).is_some(),
        "unregister_echo must not touch the sibling agent tunnel: {result}"
    );

    // ── #6091: unregistering a tunnel that was never registered must not claim it removed
    // one. The wire shape is deliberately unchanged (the caller still diffs `registrations`),
    // so the log line is the observable signal that the no-op branch was taken.
    let absent = post_json_rpc(
        &rpc_base,
        7008,
        "openhuman.webhooks_unregister_echo",
        json!({ "tunnel_uuid": "e2e-never-registered-tunnel" }),
    )
    .await;
    let envelope = assert_no_jsonrpc_error(&absent, "webhooks_unregister_echo (absent tunnel)");
    let logs = envelope
        .get("logs")
        .and_then(Value::as_array)
        .expect("unregister_echo carries a log line");
    let line = logs[0].as_str().unwrap_or_default();
    assert!(
        line.contains("nothing removed"),
        "an absent tunnel must be reported as a no-op, not as a removal: {line}"
    );
    assert!(
        registration(peel(envelope), agent_uuid).is_some(),
        "the no-op must leave the unrelated agent tunnel registered: {envelope}"
    );

    // ── a missing required param is a params error, not a panic or a silent success.
    let bad = post_json_rpc(
        &rpc_base,
        7007,
        "openhuman.webhooks_register_echo",
        json!({ "tunnel_name": "no uuid here" }),
    )
    .await;
    let message = jsonrpc_error_message(&bad, "register_echo without tunnel_uuid");
    assert!(
        message.contains("missing required param 'tunnel_uuid'"),
        "a missing tunnel_uuid must be caught by the pre-dispatch schema validator \
         (`core::all::validate_params`) and name the field, got: {message}"
    );

    mock_join.abort();
    rpc_join.abort();
}

/// The debug-log ring: empty → populated by a recorded request → cleared.
///
/// Covers: `openhuman.webhooks_list_logs`, `openhuman.webhooks_clear_logs`.
#[tokio::test]
async fn webhooks_debug_log_ring_records_and_clears() {
    let _env_lock = webhooks_e2e_env_lock();
    let router = shared_router();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let _home_guard = EnvGuard::set_to_path("HOME", home);
    let _ws_guard = EnvGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_guard = EnvGuard::unset("BACKEND_URL");
    let _vite_guard = EnvGuard::unset("VITE_BACKEND_URL");

    let state = BackendState::default();
    let (mock_addr, mock_join) = serve_ephemeral(mock_backend_router(state)).await;
    write_test_config(&openhuman_home, &format!("http://{mock_addr}"));

    let (rpc_addr, rpc_join) = serve_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    // Start from a known-empty ring — `clear_logs` reports what it removed.
    post_json_rpc(&rpc_base, 7101, "openhuman.webhooks_clear_logs", json!({})).await;

    let empty = post_json_rpc(
        &rpc_base,
        7102,
        "openhuman.webhooks_list_logs",
        json!({ "limit": 50 }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&empty, "webhooks_list_logs (empty)"));
    assert_eq!(
        result.get("logs").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "the ring must be empty right after clear_logs: {result}"
    );

    // Drive the same entry point the socket ingress uses.
    router.record_request(
        &sample_webhook_request("corr-e2e-1", "e2e-log-tunnel"),
        Some("echo".to_string()),
    );

    let listed = post_json_rpc(
        &rpc_base,
        7103,
        "openhuman.webhooks_list_logs",
        json!({ "limit": 50 }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&listed, "webhooks_list_logs"));
    let logs = result
        .get("logs")
        .and_then(Value::as_array)
        .expect("logs array");
    assert_eq!(logs.len(), 1, "one recorded request → one log entry: {result}");
    let entry = &logs[0];
    assert_eq!(
        entry.get("correlation_id").and_then(Value::as_str),
        Some("corr-e2e-1"),
        "the log entry must carry the request's correlation id: {entry}"
    );
    assert_eq!(entry.get("method").and_then(Value::as_str), Some("POST"));
    assert_eq!(entry.get("path").and_then(Value::as_str), Some("/hook"));
    assert_eq!(entry.get("skill_id").and_then(Value::as_str), Some("echo"));
    assert_eq!(
        entry.get("stage").and_then(Value::as_str),
        Some("received"),
        "a request with no response yet must sit at stage=received: {entry}"
    );
    assert!(
        entry.get("status_code").map(Value::is_null).unwrap_or(true),
        "no response recorded yet ⇒ status_code must still be null: {entry}"
    );

    // #6090 — `limit` is a maximum, so an explicit zero returns nothing. The ring holds one
    // entry at this point, so a `.max(1)`-style clamp would return that entry and this fails.
    let zero = post_json_rpc(
        &rpc_base,
        7106,
        "openhuman.webhooks_list_logs",
        json!({ "limit": 0 }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&zero, "webhooks_list_logs (limit 0)"));
    assert_eq!(
        result.get("logs").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "limit 0 must return no entries, not one: {result}"
    );
    // The neighbouring value must still be honoured, so this cannot pass by returning
    // nothing for every limit.
    let one = post_json_rpc(
        &rpc_base,
        7107,
        "openhuman.webhooks_list_logs",
        json!({ "limit": 1 }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&one, "webhooks_list_logs (limit 1)"));
    assert_eq!(
        result.get("logs").and_then(Value::as_array).map(Vec::len),
        Some(1),
        "limit 1 must still return the single recorded entry: {result}"
    );

    // clear_logs reports the count it removed, and the ring is empty afterwards.
    let cleared = post_json_rpc(&rpc_base, 7104, "openhuman.webhooks_clear_logs", json!({})).await;
    let result = peel(assert_no_jsonrpc_error(&cleared, "webhooks_clear_logs"));
    assert_eq!(
        result.get("cleared").and_then(Value::as_u64),
        Some(1),
        "clear_logs must report the number of entries it removed: {result}"
    );

    let after = post_json_rpc(&rpc_base, 7105, "openhuman.webhooks_list_logs", json!({})).await;
    let result = peel(assert_no_jsonrpc_error(&after, "webhooks_list_logs (after)"));
    assert_eq!(
        result.get("logs").and_then(Value::as_array).map(Vec::len),
        Some(0)
    );

    mock_join.abort();
    rpc_join.abort();
}

/// `trigger_agent` rejects a source slug it does not implement, naming the supported set.
///
/// The three supported slugs all run the triage pipeline (an LLM call), so the failure path is
/// what this file covers; `tests/json_rpc_e2e.rs` owns the model-backed flows.
///
/// Covers: `openhuman.webhooks_trigger_agent`.
#[tokio::test]
async fn webhooks_trigger_agent_rejects_unsupported_source() {
    let _env_lock = webhooks_e2e_env_lock();
    let _router = shared_router();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let _home_guard = EnvGuard::set_to_path("HOME", home);
    let _ws_guard = EnvGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_guard = EnvGuard::unset("BACKEND_URL");
    let _vite_guard = EnvGuard::unset("VITE_BACKEND_URL");

    let state = BackendState::default();
    let (mock_addr, mock_join) = serve_ephemeral(mock_backend_router(state)).await;
    write_test_config(&openhuman_home, &format!("http://{mock_addr}"));

    let (rpc_addr, rpc_join) = serve_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    let bad_source = post_json_rpc(
        &rpc_base,
        7201,
        "openhuman.webhooks_trigger_agent",
        json!({ "caller_id": "caller-1", "source": "carrier-pigeon", "reason": "e2e" }),
    )
    .await;
    let message = jsonrpc_error_message(&bad_source, "trigger_agent with a bogus source");
    assert!(
        message.contains("unsupported trigger source `carrier-pigeon`"),
        "the error must name the rejected slug, got: {message}"
    );
    assert!(
        message.contains("webhook") && message.contains("cron") && message.contains("external"),
        "the error must list the supported slugs so a caller can fix it, got: {message}"
    );

    // `caller_id` is the one required field; omitting it must not reach the triage pipeline.
    let missing_caller = post_json_rpc(
        &rpc_base,
        7202,
        "openhuman.webhooks_trigger_agent",
        json!({ "source": "external" }),
    )
    .await;
    let message = jsonrpc_error_message(&missing_caller, "trigger_agent without caller_id");
    assert!(
        message.contains("missing required param 'caller_id'"),
        "a missing caller_id must be refused before the triage pipeline is reached, got: {message}"
    );

    mock_join.abort();
    rpc_join.abort();
}

// ── backend tunnel CRUD ──────────────────────────────────────────────────────

/// The full backend-tunnel surface against a recording mock: create → get → update → list →
/// bandwidth → delete, asserting on both the response and the request the adapter built.
///
/// Covers: `openhuman.webhooks_create_tunnel`, `openhuman.webhooks_get_tunnel`,
/// `openhuman.webhooks_update_tunnel`, `openhuman.webhooks_list_tunnels`,
/// `openhuman.webhooks_get_bandwidth`, `openhuman.webhooks_delete_tunnel`.
#[tokio::test]
async fn webhooks_backend_tunnel_crud_roundtrip() {
    let _env_lock = webhooks_e2e_env_lock();
    let _router = shared_router();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let _home_guard = EnvGuard::set_to_path("HOME", home);
    let _ws_guard = EnvGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_guard = EnvGuard::unset("BACKEND_URL");
    let _vite_guard = EnvGuard::unset("VITE_BACKEND_URL");

    let state = BackendState::default();
    let (mock_addr, mock_join) = serve_ephemeral(mock_backend_router(state.clone())).await;
    write_test_config(&openhuman_home, &format!("http://{mock_addr}"));

    let (rpc_addr, rpc_join) = serve_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    let store = post_json_rpc(
        &rpc_base,
        7300,
        "openhuman.auth_store_session",
        json!({ "token": TEST_JWT, "user_id": "webhooks-e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "auth_store_session");

    // ── create: the name is trimmed and an empty description is dropped, not sent blank.
    let created = post_json_rpc(
        &rpc_base,
        7301,
        "openhuman.webhooks_create_tunnel",
        json!({ "name": "  Payments Hook  ", "description": "   " }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&created, "webhooks_create_tunnel"));
    assert_eq!(
        result.get("name").and_then(Value::as_str),
        Some("Payments Hook"),
        "the mock echoes the name it received; it must arrive trimmed: {result}"
    );
    let create_call = state
        .calls()
        .into_iter()
        .find(|c| c.method == "POST" && c.path == "/webhooks/core")
        .expect("a POST /webhooks/core was issued");
    let body = create_call.body.expect("create body");
    assert_eq!(body.get("name").and_then(Value::as_str), Some("Payments Hook"));
    assert!(
        body.get("description").is_none(),
        "a whitespace-only description must be omitted from the body, not sent empty: {body}"
    );

    // ── get: the id is percent-encoded into the path.
    let fetched = post_json_rpc(
        &rpc_base,
        7302,
        "openhuman.webhooks_get_tunnel",
        json!({ "id": "  tun a b  " }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&fetched, "webhooks_get_tunnel"));
    assert_eq!(
        result.get("name").and_then(Value::as_str),
        Some("fetched"),
        "get_tunnel must surface the backend row: {result}"
    );
    let get_call = state
        .calls()
        .into_iter()
        .find(|c| c.method == "GET" && c.path.starts_with("/webhooks/core/tun"))
        .expect("a GET /webhooks/core/{id} was issued");
    assert_eq!(
        get_call.path, "/webhooks/core/tun a b",
        "the id must be trimmed and percent-encoded — the mock reports the *decoded* capture, so \
         an un-encoded id with spaces would not have reached this route at all"
    );

    // ── update: snake_case params are mapped to the backend's camelCase body keys.
    let updated = post_json_rpc(
        &rpc_base,
        7303,
        "openhuman.webhooks_update_tunnel",
        // `update_tunnel` is the one controller in this namespace whose declared inputs are
        // camelCase (`isActive`), matching `WebhookUpdateTunnelParams`' `rename_all`. Its
        // siblings take snake_case (`tunnel_uuid`, `backend_tunnel_id`). Asserting the real
        // contract here pins that asymmetry rather than papering over it.
        json!({ "id": "tun-created", "name": "Renamed", "isActive": false }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&updated, "webhooks_update_tunnel"));
    assert_eq!(result.get("name").and_then(Value::as_str), Some("Renamed"));
    assert_eq!(
        result.get("isActive").and_then(Value::as_bool),
        Some(false),
        "the mock reflects the isActive it was sent: {result}"
    );
    let patch_call = state
        .calls()
        .into_iter()
        .find(|c| c.method == "PATCH")
        .expect("a PATCH was issued");
    let body = patch_call.body.expect("patch body");
    assert_eq!(
        body.get("isActive").and_then(Value::as_bool),
        Some(false),
        "the active flag must reach the backend as `isActive`: {body}"
    );
    assert!(
        body.get("is_active").is_none(),
        "a snake_case duplicate must not also be sent: {body}"
    );
    assert!(
        body.get("description").is_none(),
        "an omitted field must be absent from the PATCH body, not sent as null: {body}"
    );

    // ── list
    let listed = post_json_rpc(
        &rpc_base,
        7304,
        "openhuman.webhooks_list_tunnels",
        json!({}),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&listed, "webhooks_list_tunnels"));
    let tunnels = result
        .get("tunnels")
        .and_then(Value::as_array)
        .expect("tunnels array");
    assert_eq!(tunnels.len(), 1, "list_tunnels must surface the backend rows: {result}");
    assert_eq!(tunnels[0].get("id").and_then(Value::as_str), Some("tun-1"));

    // ── bandwidth: its own path, not the `{id}` capture.
    let bandwidth = post_json_rpc(
        &rpc_base,
        7305,
        "openhuman.webhooks_get_bandwidth",
        json!({}),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&bandwidth, "webhooks_get_bandwidth"));
    assert_eq!(
        result.get("bytesIn").and_then(Value::as_u64),
        Some(4096),
        "get_bandwidth must read /webhooks/core/bandwidth, not a tunnel row: {result}"
    );
    assert_eq!(result.get("bytesOut").and_then(Value::as_u64), Some(8192));

    // ── delete
    let deleted = post_json_rpc(
        &rpc_base,
        7306,
        "openhuman.webhooks_delete_tunnel",
        json!({ "id": "tun-created" }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&deleted, "webhooks_delete_tunnel"));
    assert_eq!(result.get("deleted").and_then(Value::as_bool), Some(true));
    assert!(
        state
            .calls()
            .iter()
            .any(|c| c.method == "DELETE" && c.path == "/webhooks/core/tun-created"),
        "delete_tunnel must issue DELETE on the id path: {:?}",
        state.calls()
    );

    // ── local validation runs before the network hop.
    let blank_name = post_json_rpc(
        &rpc_base,
        7307,
        "openhuman.webhooks_create_tunnel",
        json!({ "name": "   " }),
    )
    .await;
    assert!(
        jsonrpc_error_message(&blank_name, "create_tunnel with a blank name").contains("name is required"),
        "a whitespace-only name must be rejected locally"
    );

    let blank_id = post_json_rpc(
        &rpc_base,
        7308,
        "openhuman.webhooks_get_tunnel",
        json!({ "id": "  " }),
    )
    .await;
    assert!(
        jsonrpc_error_message(&blank_id, "get_tunnel with a blank id").contains("id is required"),
        "a whitespace-only id must be rejected locally"
    );

    let before = state.calls().len();
    let blank_delete = post_json_rpc(
        &rpc_base,
        7309,
        "openhuman.webhooks_delete_tunnel",
        json!({ "id": "" }),
    )
    .await;
    assert!(
        jsonrpc_error_message(&blank_delete, "delete_tunnel with a blank id").contains("id is required")
    );
    assert_eq!(
        state.calls().len(),
        before,
        "a locally-rejected request must not reach the backend at all"
    );

    mock_join.abort();
    rpc_join.abort();
}

/// Every backend-tunnel adapter refuses before the network hop when no session is stored.
///
/// This is the guard that keeps an unauthenticated core from firing a doomed request at the
/// hosted backend; it is shared by all six tunnel controllers via `require_token`.
#[tokio::test]
async fn webhooks_tunnel_calls_require_a_stored_session() {
    let _env_lock = webhooks_e2e_env_lock();
    let _router = shared_router();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let _home_guard = EnvGuard::set_to_path("HOME", home);
    let _ws_guard = EnvGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_guard = EnvGuard::unset("BACKEND_URL");
    let _vite_guard = EnvGuard::unset("VITE_BACKEND_URL");

    let state = BackendState::default();
    let (mock_addr, mock_join) = serve_ephemeral(mock_backend_router(state.clone())).await;
    write_test_config(&openhuman_home, &format!("http://{mock_addr}"));

    let (rpc_addr, rpc_join) = serve_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    // Deliberately NO auth_store_session.
    for (id, method, params) in [
        (7401, "openhuman.webhooks_list_tunnels", json!({})),
        (7402, "openhuman.webhooks_get_bandwidth", json!({})),
        (
            7403,
            "openhuman.webhooks_create_tunnel",
            json!({ "name": "unauthenticated" }),
        ),
    ] {
        let response = post_json_rpc(&rpc_base, id, method, params).await;
        let message = jsonrpc_error_message(&response, method);
        assert!(
            message.contains("no backend session token"),
            "{method} must refuse without a stored session, got: {message}"
        );
    }

    assert!(
        state.calls().is_empty(),
        "no tunnel request may reach the backend without a session: {:?}",
        state.calls()
    );

    mock_join.abort();
    rpc_join.abort();
}
