//! End-to-end coverage for the `socket` namespace (5 controllers, 0% before this file) and the
//! three uncovered `channel` queue controllers.
//!
//! ## Why the socket cases look the way they do
//!
//! `socket_*` operates on a **process-global** `SocketManager` (`OnceLock`), and this file is a
//! module of the aggregated `raw_coverage_all` binary — so it shares that manager with
//! `connectivity_raw_coverage_e2e.rs`, which asserts `socket_state == "disconnected"`, and with
//! `webhooks_ingress_e2e.rs`, which hangs its router off it. Every case here therefore:
//!
//!   * holds `crate::SHARED_ENV_LOCK` for its whole body — the same lock the connectivity suite
//!     holds across its socket-state assertion, which is what keeps the two from interleaving;
//!   * offers a manager via `set_global_socket_manager` but reads back whatever is installed,
//!     since the `OnceLock` may already be owned by a sibling suite;
//!   * leaves the manager **disconnected** on the way out, so the next suite sees the state it
//!     expects.
//!
//! No case dials a real backend: `connect` spawns a background reconnect loop, so the one case
//! that connects points at a closed loopback port and tears the loop down immediately.
//!
//! Run with:
//!   ~/tinyhuman/ci-slot.sh cargo test --test raw_coverage_all \
//!       --features "$(bash scripts/ci/product-features.sh)" channel_socket

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::auth::{get_rpc_token, init_rpc_token};
use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::platform::socket::{
    global_socket_manager, set_global_socket_manager, SocketManager,
};

// ── env serialisation ────────────────────────────────────────────────────────

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn socket_e2e_env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const TEST_JWT: &str = "e2e-channel-socket-jwt";

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
        let token_dir = std::env::temp_dir().join("openhuman-channel-socket-e2e-auth");
        std::fs::create_dir_all(&token_dir).expect("rpc token dir");
        init_rpc_token(&token_dir).expect("init rpc token for channel_socket_e2e");
        get_rpc_token().expect("an RPC token is initialised for this process")
    })
}

fn ensure_rpc_auth() {
    let _ = rpc_bearer();
}

/// Ensure a global `SocketManager` exists and return the one that is actually installed.
///
/// The `OnceLock` may already be owned by a sibling suite in this binary; `set` is then a logged
/// no-op, so the return value — not the argument — is the object the RPC handlers will resolve.
fn installed_socket_manager() -> &'static Arc<SocketManager> {
    set_global_socket_manager(Arc::new(SocketManager::new()));
    global_socket_manager().expect("a global SocketManager after set_global_socket_manager")
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

// ── minimal mock backend ─────────────────────────────────────────────────────

async fn mock_current_user(headers: HeaderMap) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let authed = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == format!("Bearer {TEST_JWT}"))
        .unwrap_or(false);
    if !authed {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "success": false, "error": "unauthorized" })),
        ));
    }
    Ok(Json(json!({
        "success": true,
        "data": { "_id": "channel-socket-e2e-user", "username": "channel-socket-e2e" }
    })))
}

fn mock_backend_router() -> Router {
    Router::new()
        .route("/settings", get(mock_current_user))
        .route("/auth/me", get(mock_current_user))
}

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

/// Bind an ephemeral port, read it, then drop the listener — so the address is well-formed and
/// reliably *closed*. `socket_connect` against it fails at TCP, which is what keeps the spawned
/// reconnect loop from ever reaching a real handshake.
async fn closed_loopback_addr() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind for a closed port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr
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
    write_cfg(
        &openhuman_dir.join("users").join("channel-socket-e2e-user"),
        &cfg,
    );
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

fn peel(v: &Value) -> &Value {
    if v.get("logs").is_some() {
        v.get("result").unwrap_or(v)
    } else {
        v
    }
}

/// Boilerplate every case shares: temp HOME, mock backend, config, core RPC server.
struct Harness {
    rpc_base: String,
    mock_join: tokio::task::JoinHandle<()>,
    rpc_join: tokio::task::JoinHandle<()>,
    _home: EnvGuard,
    _ws: EnvGuard,
    _backend: EnvGuard,
    _vite: EnvGuard,
    _tmp: tempfile::TempDir,
}

impl Harness {
    async fn start() -> Self {
        let tmp = tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let openhuman_home = home.join(".openhuman");
        let _home = EnvGuard::set_to_path("HOME", &home);
        let _ws = EnvGuard::unset("OPENHUMAN_WORKSPACE");
        let _backend = EnvGuard::unset("BACKEND_URL");
        let _vite = EnvGuard::unset("VITE_BACKEND_URL");

        let (mock_addr, mock_join) = serve_ephemeral(mock_backend_router()).await;
        write_test_config(&openhuman_home, &format!("http://{mock_addr}"));

        let (rpc_addr, rpc_join) = serve_ephemeral(build_core_http_router(false)).await;
        Self {
            rpc_base: format!("http://{rpc_addr}"),
            mock_join,
            rpc_join,
            _home,
            _ws,
            _backend,
            _vite,
            _tmp: tmp,
        }
    }

    fn stop(self) {
        self.mock_join.abort();
        self.rpc_join.abort();
    }
}

// ── socket namespace ─────────────────────────────────────────────────────────

/// `socket_state` reports the manager's real state, and both its param-validation guards fire
/// before anything touches the network.
///
/// Covers: `openhuman.socket_state`, `openhuman.socket_connect`, `openhuman.socket_emit`.
#[tokio::test]
async fn socket_state_and_parameter_guards() {
    let _env_lock = socket_e2e_env_lock();
    let manager = installed_socket_manager();
    // Start from a known state regardless of what a sibling suite left behind.
    manager.disconnect().await.expect("baseline disconnect");

    let h = Harness::start().await;

    let state = post_json_rpc(&h.rpc_base, 8001, "openhuman.socket_state", json!({})).await;
    let result = peel(assert_no_jsonrpc_error(&state, "socket_state"));
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("disconnected"),
        "socket_state serialises ConnectionStatus through serde (lowercase): {result}"
    );
    assert!(
        result.get("socket_id").map(Value::is_null).unwrap_or(false),
        "a disconnected manager has no socket id: {result}"
    );

    // `connect` requires both params, and reports which one is missing.
    let no_url = post_json_rpc(
        &h.rpc_base,
        8002,
        "openhuman.socket_connect",
        json!({ "token": "t" }),
    )
    .await;
    assert!(
        jsonrpc_error_message(&no_url, "socket_connect without url")
            .contains("missing required param 'url'"),
        "socket_connect must name the missing param"
    );

    let no_token = post_json_rpc(
        &h.rpc_base,
        8003,
        "openhuman.socket_connect",
        json!({ "url": "ws://127.0.0.1:1" }),
    )
    .await;
    assert!(
        jsonrpc_error_message(&no_token, "socket_connect without token")
            .contains("missing required param 'token'"),
        "socket_connect must name the missing param"
    );

    // An empty token is refused *before* a reconnect loop is spawned — the guard that keeps an
    // unauthenticated core from producing a 401 retry storm.
    let empty_token = post_json_rpc(
        &h.rpc_base,
        8004,
        "openhuman.socket_connect",
        json!({ "url": "ws://127.0.0.1:1", "token": "   " }),
    )
    .await;
    assert!(
        jsonrpc_error_message(&empty_token, "socket_connect with a blank token")
            .contains("empty session token"),
        "a whitespace-only token must be rejected, not optimistically reported as Connecting"
    );

    let still_down = post_json_rpc(&h.rpc_base, 8005, "openhuman.socket_state", json!({})).await;
    let result = peel(assert_no_jsonrpc_error(&still_down, "socket_state after guards"));
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("disconnected"),
        "a rejected connect must not have moved the manager out of Disconnected: {result}"
    );

    // `emit` on a manager that has never connected has no channel to write to.
    let emit_offline = post_json_rpc(
        &h.rpc_base,
        8006,
        "openhuman.socket_emit",
        json!({ "event": "test:event", "data": { "k": "v" } }),
    )
    .await;
    assert!(
        jsonrpc_error_message(&emit_offline, "socket_emit while disconnected").contains("Not connected"),
        "emitting with no connection must be an error, not a silent success"
    );

    let emit_no_event = post_json_rpc(
        &h.rpc_base,
        8007,
        "openhuman.socket_emit",
        json!({ "data": { "k": "v" } }),
    )
    .await;
    assert!(
        jsonrpc_error_message(&emit_no_event, "socket_emit without event")
            .contains("missing required param 'event'"),
        "socket_emit must name the missing param"
    );

    manager.disconnect().await.expect("teardown disconnect");
    h.stop();
}

/// `socket_connect` → `socket_disconnect` moves the manager through Connecting and back.
///
/// The target is a loopback port that was bound and released, so the spawned `ws_loop` fails at
/// TCP and never reaches a handshake; `disconnect` then tears it down. This is the whole
/// observable lifecycle of the two controllers without a backend.
///
/// Covers: `openhuman.socket_connect`, `openhuman.socket_disconnect`, `openhuman.socket_state`.
#[tokio::test]
async fn socket_connect_then_disconnect_round_trips_state() {
    let _env_lock = socket_e2e_env_lock();
    let manager = installed_socket_manager();
    manager.disconnect().await.expect("baseline disconnect");

    let h = Harness::start().await;
    let dead = closed_loopback_addr().await;

    let connected = post_json_rpc(
        &h.rpc_base,
        8101,
        "openhuman.socket_connect",
        json!({ "url": format!("ws://{dead}"), "token": "e2e-socket-token" }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&connected, "socket_connect"));
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("connecting"),
        "connect returns as soon as the loop is spawned, in the Connecting state. The spelling \
         is the serde one (`rename_all = \"lowercase\"`), the same encoding `socket_state` and \
         `connectivity_diag` publish — this handler used to emit Rust's `Debug` \
         (`\"Connecting\"`) and the split was #6111: {result}"
    );

    let disconnected = post_json_rpc(&h.rpc_base, 8102, "openhuman.socket_disconnect", json!({})).await;
    let result = peel(assert_no_jsonrpc_error(&disconnected, "socket_disconnect"));
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("disconnected"),
        "disconnect must report the manager back at rest, in the serde spelling (#6111): {result}"
    );

    let state = post_json_rpc(&h.rpc_base, 8103, "openhuman.socket_state", json!({})).await;
    let result = peel(assert_no_jsonrpc_error(&state, "socket_state after disconnect"));
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("disconnected"),
        "state must agree with disconnect's own report exactly — one namespace, one status \
         vocabulary (#6111): {result}"
    );
    assert!(
        result.get("socket_id").map(Value::is_null).unwrap_or(false),
        "disconnect clears the socket id: {result}"
    );

    manager.disconnect().await.expect("teardown disconnect");
    h.stop();
}

/// `socket_connect_with_session` refuses before it dials when no session JWT is stored, and
/// resolves the stored one when there is.
///
/// Covers: `openhuman.socket_connect_with_session`.
#[tokio::test]
async fn socket_connect_with_session_requires_a_stored_session() {
    let _env_lock = socket_e2e_env_lock();
    let manager = installed_socket_manager();
    manager.disconnect().await.expect("baseline disconnect");

    let h = Harness::start().await;

    // No session stored yet: the credential lookup, not the socket, is what fails.
    let no_session = post_json_rpc(
        &h.rpc_base,
        8201,
        "openhuman.socket_connect_with_session",
        json!({}),
    )
    .await;
    let message = jsonrpc_error_message(&no_session, "socket_connect_with_session, signed out");
    assert!(
        message.contains("no session token stored"),
        "the error must say the user has to log in first, got: {message}"
    );

    let state = post_json_rpc(&h.rpc_base, 8202, "openhuman.socket_state", json!({})).await;
    let result = peel(assert_no_jsonrpc_error(&state, "socket_state"));
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("disconnected"),
        "the refused connect must not have spawned a loop: {result}"
    );

    // With a session stored, the same call gets past the credential guard and spawns the loop
    // against the mock origin — which speaks HTTP, not Socket.IO, so it never handshakes.
    let store = post_json_rpc(
        &h.rpc_base,
        8203,
        "openhuman.auth_store_session",
        json!({ "token": TEST_JWT, "user_id": "channel-socket-e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "auth_store_session");

    let with_session = post_json_rpc(
        &h.rpc_base,
        8204,
        "openhuman.socket_connect_with_session",
        json!({}),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(
        &with_session,
        "socket_connect_with_session, signed in",
    ));
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("connecting"),
        "a stored session must get past the guard and start the loop, reporting the serde \
         spelling (#6111): {result}"
    );

    let stopped = post_json_rpc(&h.rpc_base, 8205, "openhuman.socket_disconnect", json!({})).await;
    assert_eq!(
        peel(assert_no_jsonrpc_error(&stopped, "socket_disconnect"))
            .get("status")
            .and_then(Value::as_str),
        Some("disconnected")
    );

    manager.disconnect().await.expect("teardown disconnect");
    h.stop();
}

// ── channel queue controllers ────────────────────────────────────────────────

/// The three queue/cancel controllers on a thread with no in-flight turn.
///
/// This is the branch every one of them takes whenever the UI polls a quiet thread — the common
/// case, and the one whose *content* nobody was checking: each returns a fully-populated payload
/// (`active`/`cleared`/`cancelled` false, all counters zero) rather than an error or an empty
/// object, and each echoes the **trimmed** thread id back.
///
/// Covers: `openhuman.channel_web_queue_status`, `openhuman.channel_web_queue_clear`,
/// `openhuman.channel_web_cancel`.
#[tokio::test]
async fn channel_queue_controllers_report_an_idle_thread() {
    let _env_lock = socket_e2e_env_lock();
    let h = Harness::start().await;

    // Leading/trailing whitespace is deliberate: the handlers trim before echoing, and a caller
    // that keys UI state on the returned id needs that to be the canonical form.
    let padded = "  e2e-idle-thread  ";
    let canonical = "e2e-idle-thread";

    let status = post_json_rpc(
        &h.rpc_base,
        8301,
        "openhuman.channel_web_queue_status",
        json!({ "thread_id": padded }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&status, "channel_web_queue_status"));
    assert_eq!(
        result.get("thread_id").and_then(Value::as_str),
        Some(canonical),
        "queue_status must echo the trimmed thread id: {result}"
    );
    assert_eq!(
        result.get("active").and_then(Value::as_bool),
        Some(false),
        "no in-flight turn ⇒ active=false: {result}"
    );
    for counter in ["steers", "followups", "collects", "total"] {
        assert_eq!(
            result.get(counter).and_then(Value::as_u64),
            Some(0),
            "an idle thread must report {counter}=0, and must report it at all: {result}"
        );
    }
    assert!(
        result.get("request_id").is_none(),
        "the idle branch omits request_id entirely rather than sending a null: {result}"
    );

    let cleared = post_json_rpc(
        &h.rpc_base,
        8302,
        "openhuman.channel_web_queue_clear",
        json!({ "thread_id": padded }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&cleared, "channel_web_queue_clear"));
    assert_eq!(
        result.get("thread_id").and_then(Value::as_str),
        Some(canonical)
    );
    assert_eq!(
        result.get("cleared").and_then(Value::as_bool),
        Some(false),
        "clearing a thread with no queue must report cleared=false, not a cheerful true: {result}"
    );
    assert_eq!(result.get("dropped").and_then(Value::as_u64), Some(0));

    let cancelled = post_json_rpc(
        &h.rpc_base,
        8303,
        "openhuman.channel_web_cancel",
        json!({ "client_id": "  e2e-client  ", "thread_id": padded }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&cancelled, "channel_web_cancel"));
    assert_eq!(
        result.get("cancelled").and_then(Value::as_bool),
        Some(false),
        "cancelling an idle thread must report that nothing was cancelled: {result}"
    );
    assert_eq!(
        result.get("client_id").and_then(Value::as_str),
        Some("e2e-client"),
        "cancel must echo the trimmed client id: {result}"
    );
    assert_eq!(
        result.get("thread_id").and_then(Value::as_str),
        Some(canonical)
    );
    assert!(
        result
            .get("request_id")
            .map(Value::is_null)
            .unwrap_or(false),
        "nothing was cancelled ⇒ request_id is null: {result}"
    );

    // A request-scoped cancel for a turn that does not exist is also a no-op, not an error —
    // the stale-cancel path from #4760.
    let scoped = post_json_rpc(
        &h.rpc_base,
        8304,
        "openhuman.channel_web_cancel",
        json!({ "client_id": "e2e-client", "thread_id": canonical, "request_id": "req-that-never-ran" }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&scoped, "channel_web_cancel scoped"));
    assert_eq!(result.get("cancelled").and_then(Value::as_bool), Some(false));

    h.stop();
}

/// Every queue controller rejects a params object missing a required field, before it touches
/// the in-flight map.
///
/// The refusal comes from `core::all::validate_params`, which type- and presence-checks against
/// the declared `ControllerSchema` *before* dispatch — so the message names the field and its
/// schema comment rather than surfacing a serde error from the handler.
#[tokio::test]
async fn channel_queue_controllers_reject_missing_thread_id() {
    let _env_lock = socket_e2e_env_lock();
    let h = Harness::start().await;

    for (id, method, params) in [
        (8401, "openhuman.channel_web_queue_status", json!({})),
        (8402, "openhuman.channel_web_queue_clear", json!({})),
        (
            8403,
            "openhuman.channel_web_cancel",
            json!({ "client_id": "c" }),
        ),
    ] {
        let response = post_json_rpc(&h.rpc_base, id, method, params).await;
        let message = jsonrpc_error_message(&response, method);
        assert!(
            message.contains("missing required param 'thread_id'"),
            "{method} must name the missing field, got: {message}"
        );
    }

    // `channel_web_cancel` needs a client_id too — the pair is what scopes a cancel.
    let no_client = post_json_rpc(
        &h.rpc_base,
        8404,
        "openhuman.channel_web_cancel",
        json!({ "thread_id": "t" }),
    )
    .await;
    let message = jsonrpc_error_message(&no_client, "channel_web_cancel without client_id");
    assert!(
        message.contains("missing required param 'client_id'"),
        "cancel must require the client id that scopes it, got: {message}"
    );

    h.stop();
}
