//! JSON-RPC E2E coverage for the `openhuman.medulla_*` namespace — all nine
//! controllers, which had none.
//!
//! Run:
//! `cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)" medulla_session_e2e`
//!
//! ## What is being tested, and against what
//!
//! `openhuman::medulla` is OpenHuman acting as a **client** of the Medulla
//! orchestration backend — outbound HTTP to `/medulla/v1/*`, unwrapping a
//! `{success, data}` envelope (`client/mod.rs:118-159`). Do not confuse it with
//! `platform::socket::medulla`, which is the inbound worker side; they share a
//! product name and nothing else (`medulla/mod.rs:13-21`).
//!
//! Every case here runs against a loopback axum server speaking that envelope,
//! selected with `OPENHUMAN_MEDULLA_BASE_URL` — the documented override for
//! pointing a host at a different deployment (`resolve.rs:12-19`). Nothing
//! leaves the machine.
//!
//! ## Two preconditions, and both are worth asserting in their own right
//!
//! `resolve::client` needs a base URL **and** a session token, and reports the
//! two failures with distinct `data.kind` discriminators so a host can render
//! "sign in" separately from "misconfigured" (`ops.rs:189-201`). This suite
//! plants a session token through `AuthService` — the same store
//! `get_session_token` reads — and also covers the signed-out path, because a
//! signed-out host is the state most installs are in when they first touch this
//! surface.
//!
//! ## Feature note
//!
//! The `medulla` gate is default-ON but deliberately **not** forwarded to the
//! desktop shell (allow-listed in `INTENTIONALLY_NOT_FORWARDED` — the app never
//! dials a Medulla backend; the Medulla TUI embeds this crate instead). The
//! product feature string does not turn defaults off, so these controllers are
//! present in this binary.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::header::AUTHORIZATION;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

/// The bearer this suite *proposes*. It is only used if this suite happens to
/// be the first in the aggregated binary to initialise the token subsystem —
/// see `rpc_token()` below, which is what actually gets sent.
const PROPOSED_RPC_TOKEN: &str = "medulla-session-e2e-token";
/// The session token this suite plants and the fixture backend demands.
const SESSION_JWT: &str = "planted-medulla-session-jwt";

static AUTH_INIT: OnceLock<()> = OnceLock::new();

/// The crate-wide env lock, not a private one.
static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ── Env plumbing ────────────────────────────────────────────────────────────

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
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

/// The bearer the running process will actually validate.
///
/// `core::auth::RPC_TOKEN` is a process-global `OnceLock` and `init_rpc_token`
/// returns early once it is set. Every `tests/raw_coverage/` suite now shares
/// one process, so the FIRST suite to initialise pins the token for all of
/// them; a suite that sent its own hard-coded literal would get 401 whenever it
/// lost that race, in an order that depends on libtest scheduling and load.
///
/// So: propose a token, initialise, then ask what the process settled on and
/// send that. Correct whether this suite wins the race or loses it.
fn rpc_token() -> &'static str {
    AUTH_INIT.get_or_init(|| {
        if std::env::var(CORE_TOKEN_ENV_VAR)
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
        {
            std::env::set_var(CORE_TOKEN_ENV_VAR, PROPOSED_RPC_TOKEN);
        }
        let token_dir = std::env::temp_dir().join("openhuman-medulla-session-e2e-auth");
        std::fs::create_dir_all(&token_dir).expect("token dir");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
    openhuman_core::core::auth::get_rpc_token()
        .expect("the token subsystem is initialised by the line above")
}

// ── The fixture Medulla backend ─────────────────────────────────────────────

/// Records what the backend was asked, so the suite can assert the client sent
/// the right thing rather than only that it parsed the reply.
#[derive(Default)]
struct BackendState {
    /// `(method, path, query)` for every request that arrived.
    seen: Mutex<Vec<(String, String, String)>>,
    /// Bodies of the message posts.
    bodies: Mutex<Vec<Value>>,
}

impl BackendState {
    fn record(&self, method: &str, path: &str, query: &str) {
        self.seen.lock().unwrap_or_else(|p| p.into_inner()).push((
            method.to_string(),
            path.to_string(),
            query.to_string(),
        ));
    }

    fn queries_for(&self, path: &str) -> Vec<String> {
        self.seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|(_, p, _)| p == path)
            .map(|(_, _, q)| q.clone())
            .collect()
    }
}

fn envelope(data: Value) -> Json<Value> {
    Json(json!({ "success": true, "data": data }))
}

/// Reject anything not carrying the planted bearer token.
///
/// Every session method must attach it (`client/mod.rs:108-115`); a method that
/// forgot would sail past a fixture that did not check.
fn authed(headers: &HeaderMap) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Bearer {SESSION_JWT}"))
}

async fn serve_fixture_backend() -> (
    SocketAddr,
    Arc<BackendState>,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let state = Arc::new(BackendState::default());

    async fn list_sessions(
        State(state): State<Arc<BackendState>>,
        headers: HeaderMap,
    ) -> Json<Value> {
        state.record("GET", "/medulla/v1/sessions", "");
        if !authed(&headers) {
            return Json(
                json!({ "success": false, "error": "unauthorized", "errorCode": "Unauthorized" }),
            );
        }
        envelope(json!([
            {
                "sessionId": "sess-alpha",
                "title": "Planted alpha",
                "lastActiveAt": 1_700_000_000_000_i64,
                "status": "active",
                "lastSeq": 7
            },
            {
                "sessionId": "sess-beta",
                "title": null,
                "status": "idle",
                "lastSeq": 0
            }
        ]))
    }

    async fn create_session(
        State(state): State<Arc<BackendState>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        state.record("POST", "/medulla/v1/sessions", "");
        state
            .bodies
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(body.clone());
        if !authed(&headers) {
            return Json(json!({ "success": false, "error": "unauthorized" }));
        }
        // Echo the title back through the id so the test can prove the body
        // travelled rather than being dropped on the floor.
        let suffix = body
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("untitled");
        envelope(json!({ "sessionId": format!("sess-new-{suffix}") }))
    }

    async fn get_session(
        State(state): State<Arc<BackendState>>,
        AxumPath(id): AxumPath<String>,
        headers: HeaderMap,
    ) -> Json<Value> {
        state.record("GET", "/medulla/v1/sessions/:id", &id);
        if !authed(&headers) {
            return Json(json!({ "success": false, "error": "unauthorized" }));
        }
        if id == "sess-missing" {
            // A backend refusal carrying its own vocabulary — `errorCode` is
            // what the host branches on.
            return Json(json!({
                "success": false,
                "error": "no such session",
                "errorCode": "SessionNotFound"
            }));
        }
        envelope(json!({
            "sessionId": id,
            "status": "active",
            "lastCycleId": "cycle-9",
            "lastSeq": 12,
            "eventSeq": 30
        }))
    }

    async fn send_message(
        State(state): State<Arc<BackendState>>,
        AxumPath(_id): AxumPath<String>,
        Query(query): Query<std::collections::HashMap<String, String>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let sync = query.get("sync").cloned().unwrap_or_default();
        state.record("POST", "/medulla/v1/sessions/:id/messages", &sync);
        state
            .bodies
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(body.clone());
        if !authed(&headers) {
            return Json(json!({ "success": false, "error": "unauthorized" }));
        }
        // The two documented shapes: 202 async carries {cycleId, seq}; the
        // sync form adds `reply`.
        if sync == "1" {
            envelope(json!({ "cycleId": "cycle-sync", "seq": 42, "reply": "PLANTED_SYNC_REPLY" }))
        } else {
            envelope(json!({ "cycleId": "cycle-async", "seq": 41 }))
        }
    }

    async fn list_messages(
        State(state): State<Arc<BackendState>>,
        AxumPath(id): AxumPath<String>,
        Query(query): Query<std::collections::HashMap<String, String>>,
        headers: HeaderMap,
    ) -> Json<Value> {
        let after = query.get("after").cloned().unwrap_or_default();
        state.record("GET", "/medulla/v1/sessions/:id/messages", &after);
        let _ = id;
        if !authed(&headers) {
            return Json(json!({ "success": false, "error": "unauthorized" }));
        }
        // Honour the cursor, so "replays only what is new" is a property the
        // test can actually observe rather than assume.
        let all = vec![
            json!({ "seq": 1, "role": "user", "body": "first", "ts": 1_700_000_000_000_i64, "cycleId": "cycle-1" }),
            json!({ "seq": 2, "role": "assistant", "body": "second", "cycleId": "cycle-1" }),
            json!({ "seq": 3, "role": "user", "body": "third" }),
        ];
        let cursor: i64 = after.parse().unwrap_or(0);
        envelope(Value::Array(
            all.into_iter()
                .filter(|m| m["seq"].as_i64().unwrap_or(0) > cursor)
                .collect(),
        ))
    }

    async fn list_events(
        State(state): State<Arc<BackendState>>,
        AxumPath(id): AxumPath<String>,
        Query(query): Query<std::collections::HashMap<String, String>>,
        headers: HeaderMap,
    ) -> Json<Value> {
        let after = query.get("after").cloned().unwrap_or_default();
        state.record("GET", "/medulla/v1/sessions/:id/events", &after);
        let _ = id;
        if !authed(&headers) {
            return Json(json!({ "success": false, "error": "unauthorized" }));
        }
        // NOTE the shape. There are TWO `EventEnvelope` types in this domain and
        // they are not the same on the wire:
        //
        //   * `medulla::events::EventEnvelope` — `{seq, at, event: SessionEvent}`,
        //     the contract type, publicly re-exported by `medulla/mod.rs`.
        //   * `medulla::client::types::event::EventEnvelope` — `{seq?, at,
        //     sessionId, cycleId?, event: Value}`, the wire type.
        //
        // `ops::list_events` returns the **client** one, so `sessionId` is
        // required and `event` stays raw JSON. Reading the domain's public
        // re-export and assuming it describes the RPC response is the trap; it
        // cost this suite one red run. See
        // `~/tinyhuman/bugs/e2e-wave-medulla-two-eventenvelope-types.md`.
        let all = vec![
            json!({ "seq": 1, "at": 1_700_000_000_000_u64, "sessionId": "sess-alpha", "cycleId": "cycle-1",
                    "event": { "kind": "cycle_start", "cycleId": "cycle-1" } }),
            json!({ "seq": 2, "at": 1_700_000_000_100_u64, "sessionId": "sess-alpha",
                    "event": { "kind": "assistant_delta", "delta": "hi" } }),
            // A kind this build does not model: it must survive rather than
            // dropping the row.
            json!({ "seq": 3, "at": 1_700_000_000_200_u64, "sessionId": "sess-alpha",
                    "event": { "kind": "some_future_kind", "whatever": 1 } }),
        ];
        let cursor: i64 = after.parse().unwrap_or(0);
        envelope(Value::Array(
            all.into_iter()
                .filter(|e| e["seq"].as_i64().unwrap_or(0) > cursor)
                .collect(),
        ))
    }

    async fn abort(
        State(state): State<Arc<BackendState>>,
        AxumPath(id): AxumPath<String>,
        headers: HeaderMap,
    ) -> Json<Value> {
        state.record("POST", "/medulla/v1/sessions/:id/abort", &id);
        if !authed(&headers) {
            return Json(json!({ "success": false, "error": "unauthorized" }));
        }
        envelope(json!({ "sessionId": id, "aborted": true }))
    }

    async fn roster(State(state): State<Arc<BackendState>>, headers: HeaderMap) -> Json<Value> {
        state.record("GET", "/medulla/v1/roster", "");
        if !authed(&headers) {
            return Json(json!({ "success": false, "error": "unauthorized" }));
        }
        // Note the wrapper: `roster()` unwraps `{workers: [...]}` out of `data`.
        // `selected` is the one non-defaulted bool on `RosterWorker`
        // (`client/program/types.rs`) — every other optional field carries
        // `#[serde(default)]`, so omitting it is the single way to make this
        // decode fail. It did, on the first run of this suite.
        envelope(json!({
            "workers": [
                {
                    "registryId": "worker-1",
                    "label": "Planted worker",
                    "description": "Does planted things.",
                    "availability": "idle",
                    "harness": "tinyagents",
                    "address": "127.0.0.1:0",
                    "selected": true
                }
            ]
        }))
    }

    let app = Router::new()
        .route(
            "/medulla/v1/sessions",
            get(list_sessions).post(create_session),
        )
        .route("/medulla/v1/sessions/{id}", get(get_session))
        .route(
            "/medulla/v1/sessions/{id}/messages",
            get(list_messages).post(send_message),
        )
        .route("/medulla/v1/sessions/{id}/events", get(list_events))
        .route("/medulla/v1/sessions/{id}/abort", post(abort))
        .route("/medulla/v1/roster", get(roster))
        .with_state(Arc::clone(&state));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture backend");
    let addr = listener.local_addr().expect("fixture backend addr");
    let join = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, state, join)
}

// ── Harness ─────────────────────────────────────────────────────────────────

const MIN_CONFIG: &str = r#"api_url = "http://127.0.0.1:9"
default_model = "medulla-e2e-model"

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[memory_tree]
embedding_strict = false
"#;

struct Harness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

/// `plant_session` decides whether `resolve::client` will find a token, which
/// is the difference between the configured and signed-out cases below.
async fn setup(extra: Vec<EnvVarGuard>, plant_session: bool) -> Harness {
    let _ = rpc_token();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    std::fs::create_dir_all(&openhuman_home).expect("create .openhuman");
    std::fs::write(openhuman_home.join("config.toml"), MIN_CONFIG).expect("write config.toml");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(MIN_CONFIG).expect("test config must match the config schema");

    let workspace = home.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let mut guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
    ];
    guards.extend(extra);

    if plant_session {
        // Write into the same profile store `get_session_token` reads
        // (`session_support.rs:243-248` → `AuthService::get_profile`), so the
        // token resolves through production's own path rather than a test seam.
        use openhuman_core::openhuman::security::credentials::{
            AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
        };
        let config = openhuman_core::openhuman::config::Config::load_or_init()
            .await
            .expect("load config for session planting");
        AuthService::from_config(&config)
            .store_provider_token(
                APP_SESSION_PROVIDER,
                DEFAULT_AUTH_PROFILE_NAME,
                SESSION_JWT,
                std::collections::HashMap::new(),
                true,
            )
            .expect("plant an app-session token");
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr: SocketAddr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });

    Harness {
        _tmp: tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        join,
    }
}

async fn rpc(base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("build client");
    let url = format!("{}/rpc", base.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", rpc_token()))
        .json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
        .send()
        .await
        .unwrap_or_else(|err| panic!("POST {url} {method}: {err}"));
    assert!(
        response.status().is_success(),
        "HTTP {} for {method}",
        response.status()
    );
    response
        .json::<Value>()
        .await
        .unwrap_or_else(|err| panic!("json for {method}: {err}"))
}

fn ok<'a>(value: &'a Value, context: &str) -> &'a Value {
    if let Some(error) = value.get("error") {
        panic!("{context}: unexpected JSON-RPC error: {error}");
    }
    value
        .get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {value}"))
}

fn err<'a>(value: &'a Value, context: &str) -> &'a Value {
    value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected a JSON-RPC error, got: {value}"))
}

/// The `data.kind` discriminator hosts branch on.
///
/// `medulla`'s ops encode a `StructuredRpcError` into the controller error
/// channel, and the transport decodes it at the boundary into the JSON-RPC
/// `error.data` — deliberately without branching on the method name
/// (`core/jsonrpc.rs:86-99`). Reading `data.kind` rather than matching the
/// message is the whole point of that machinery: the message is prose and may
/// be reworded, the discriminator is the contract.
fn error_kind(value: &Value, context: &str) -> String {
    let error = err(value, context);
    error
        .get("data")
        .and_then(|d| d.get("kind"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            panic!("{context}: the error carries no structured `data.kind`: {error}")
        })
}

// ── status ──────────────────────────────────────────────────────────────────

/// `medulla_status` is deliberately infallible — "not configured" is a state to
/// render, not an error to raise — and it must never leak the token.
#[tokio::test]
async fn medulla_status_reports_readiness_without_dialling_anything() {
    let _lock = env_lock();
    // Point at a port nothing is listening on: `status` must still answer,
    // because it reports resolvability and makes no network call.
    let harness = setup(
        vec![EnvVarGuard::set(
            "OPENHUMAN_MEDULLA_BASE_URL",
            "http://127.0.0.1:9/",
        )],
        true,
    )
    .await;

    let status = rpc(
        &harness.rpc_base,
        300,
        "openhuman.medulla_status",
        json!({}),
    )
    .await;
    let status = ok(&status, "medulla_status configured");
    assert_eq!(
        status.get("configured").and_then(Value::as_bool),
        Some(true),
        "a base URL plus a planted session token is the configured state: {status}"
    );
    assert_eq!(
        status.get("hasSessionToken").and_then(Value::as_bool),
        Some(true),
        "the planted token must be seen: {status}"
    );
    assert_eq!(
        status.get("baseUrl").and_then(Value::as_str),
        Some("http://127.0.0.1:9"),
        "the resolved base URL is surfaced, with its trailing slash trimmed: {status}"
    );
    assert!(
        status.get("reason").is_none_or(Value::is_null),
        "a configured host must carry no reason discriminator: {status}"
    );
    assert!(
        !status.to_string().contains(SESSION_JWT),
        "the session token must NEVER be returned: {status}"
    );

    harness.join.abort();
}

/// The signed-out host: `status` says so with a stable discriminator, and every
/// other method in the namespace refuses with the matching structured error.
///
/// This is the state a fresh install is in, and the `expected_user_state` flag
/// on these errors is what keeps them out of crash reporting — so it matters
/// that the discriminator is right and not merely that something failed.
#[tokio::test]
async fn medulla_signed_out_host_refuses_every_call_with_one_discriminator() {
    let _lock = env_lock();
    let (backend_addr, _state, backend_join) = serve_fixture_backend().await;
    let harness = setup(
        vec![EnvVarGuard::set(
            "OPENHUMAN_MEDULLA_BASE_URL",
            &format!("http://{backend_addr}"),
        )],
        // No session planted: signed out.
        false,
    )
    .await;

    let status = rpc(
        &harness.rpc_base,
        310,
        "openhuman.medulla_status",
        json!({}),
    )
    .await;
    let status = ok(&status, "medulla_status signed out");
    assert_eq!(
        status.get("configured").and_then(Value::as_bool),
        Some(false),
        "a host with no session token is not configured: {status}"
    );
    assert_eq!(
        status.get("hasSessionToken").and_then(Value::as_bool),
        Some(false),
        "and it must say the token specifically is what is missing: {status}"
    );
    assert_eq!(
        status.get("reason").and_then(Value::as_str),
        Some("MedullaNoSessionToken"),
        "the reason must be the stable discriminator, not prose: {status}"
    );
    // The base URL still resolves — that is the point of reporting them
    // separately, so a UI can say "sign in" rather than "misconfigured".
    assert!(
        status
            .get("baseUrl")
            .and_then(Value::as_str)
            .is_some_and(|u| u.contains(&backend_addr.to_string())),
        "a signed-out host still reports the base URL it would use: {status}"
    );

    // Every network-backed method refuses identically.
    for (id, method, params) in [
        (311, "openhuman.medulla_list_sessions", json!({})),
        (312, "openhuman.medulla_create_session", json!({})),
        (
            313,
            "openhuman.medulla_get_session",
            json!({ "sessionId": "sess-alpha" }),
        ),
        (
            314,
            "openhuman.medulla_send_message",
            json!({ "sessionId": "sess-alpha", "body": "hi" }),
        ),
        (
            315,
            "openhuman.medulla_abort",
            json!({ "sessionId": "sess-alpha" }),
        ),
        (
            316,
            "openhuman.medulla_list_messages",
            json!({ "sessionId": "sess-alpha" }),
        ),
        (
            317,
            "openhuman.medulla_list_events",
            json!({ "sessionId": "sess-alpha" }),
        ),
        (318, "openhuman.medulla_roster", json!({})),
    ] {
        let refused = rpc(&harness.rpc_base, id, method, params).await;
        assert_eq!(
            error_kind(&refused, method),
            "MedullaNoSessionToken",
            "{method} must refuse a signed-out host with the same discriminator \
             `status` reported: {refused}"
        );
    }

    backend_join.abort();
    harness.join.abort();
}

// ── the session lifecycle ───────────────────────────────────────────────────

/// The full durable-session arc against the fixture backend:
/// create → list → get → send (async and sync) → abort.
#[tokio::test]
async fn medulla_session_lifecycle_round_trips_through_the_backend() {
    let _lock = env_lock();
    let (backend_addr, state, backend_join) = serve_fixture_backend().await;
    let harness = setup(
        vec![EnvVarGuard::set(
            "OPENHUMAN_MEDULLA_BASE_URL",
            &format!("http://{backend_addr}"),
        )],
        true,
    )
    .await;

    // ── create ──────────────────────────────────────────────────────────────
    let created = rpc(
        &harness.rpc_base,
        320,
        "openhuman.medulla_create_session",
        json!({ "title": "planted" }),
    )
    .await;
    assert_eq!(
        ok(&created, "medulla_create_session")
            .get("sessionId")
            .and_then(Value::as_str),
        Some("sess-new-planted"),
        "the title must reach the backend, not be dropped: {created}"
    );

    // Omitting the title is legal — the backend names an untitled session
    // itself rather than this host inventing one.
    let untitled = rpc(
        &harness.rpc_base,
        321,
        "openhuman.medulla_create_session",
        json!({}),
    )
    .await;
    assert_eq!(
        ok(&untitled, "medulla_create_session untitled")
            .get("sessionId")
            .and_then(Value::as_str),
        Some("sess-new-untitled"),
        "an absent title must send no title, not an empty one: {untitled}"
    );

    // ── list ────────────────────────────────────────────────────────────────
    let listed = rpc(
        &harness.rpc_base,
        322,
        "openhuman.medulla_list_sessions",
        json!({}),
    )
    .await;
    let listed = ok(&listed, "medulla_list_sessions");
    let sessions = listed
        .as_array()
        .unwrap_or_else(|| panic!("list_sessions must return an array: {listed}"));
    assert_eq!(sessions.len(), 2, "both fixture sessions: {listed}");
    assert_eq!(
        sessions[0].get("sessionId").and_then(Value::as_str),
        Some("sess-alpha")
    );
    assert_eq!(
        sessions[0].get("status").and_then(Value::as_str),
        Some("active"),
        "the wire status must round-trip through the modelled enum: {listed}"
    );
    assert_eq!(
        sessions[0].get("lastSeq").and_then(Value::as_i64),
        Some(7),
        "the replay cursor must survive: {listed}"
    );
    // The second row omits `lastActiveAt` entirely; an absent optional must
    // stay absent rather than becoming a fabricated zero.
    assert!(
        sessions[1].get("lastActiveAt").is_none_or(Value::is_null),
        "an unreported lastActiveAt must not be invented: {listed}"
    );

    // ── get ─────────────────────────────────────────────────────────────────
    let detail = rpc(
        &harness.rpc_base,
        323,
        "openhuman.medulla_get_session",
        json!({ "sessionId": "sess-alpha" }),
    )
    .await;
    let detail = ok(&detail, "medulla_get_session");
    assert_eq!(
        detail.get("sessionId").and_then(Value::as_str),
        Some("sess-alpha")
    );
    assert_eq!(
        detail.get("lastCycleId").and_then(Value::as_str),
        Some("cycle-9")
    );
    assert_eq!(detail.get("eventSeq").and_then(Value::as_i64), Some(30));

    // ── send, both modes ────────────────────────────────────────────────────
    let async_send = rpc(
        &harness.rpc_base,
        324,
        "openhuman.medulla_send_message",
        json!({ "sessionId": "sess-alpha", "body": "async turn" }),
    )
    .await;
    let async_send = ok(&async_send, "medulla_send_message async");
    assert_eq!(
        async_send.get("cycleId").and_then(Value::as_str),
        Some("cycle-async"),
        "omitting `sync` must take the non-blocking path a UI wants: {async_send}"
    );
    assert!(
        async_send.get("reply").is_none_or(Value::is_null),
        "the async form carries no reply: {async_send}"
    );

    let sync_send = rpc(
        &harness.rpc_base,
        325,
        "openhuman.medulla_send_message",
        json!({ "sessionId": "sess-alpha", "body": "sync turn", "sync": true }),
    )
    .await;
    let sync_send = ok(&sync_send, "medulla_send_message sync");
    assert_eq!(
        sync_send.get("cycleId").and_then(Value::as_str),
        Some("cycle-sync")
    );
    assert_eq!(
        sync_send.get("reply").and_then(Value::as_str),
        Some("PLANTED_SYNC_REPLY"),
        "the sync form must surface the backend's reply: {sync_send}"
    );

    // The `sync` flag must reach the wire as the backend's `0`/`1`, not as a
    // JSON boolean — that translation is the client's job.
    let sync_flags = state.queries_for("/medulla/v1/sessions/:id/messages");
    assert_eq!(
        sync_flags,
        vec!["0".to_string(), "1".to_string()],
        "the two sends must have carried sync=0 then sync=1: {sync_flags:?}"
    );
    let bodies = state
        .bodies
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    assert!(
        bodies
            .iter()
            .any(|b| b.get("body").and_then(Value::as_str) == Some("async turn")),
        "the message body must reach the backend: {bodies:?}"
    );

    // ── abort ───────────────────────────────────────────────────────────────
    let aborted = rpc(
        &harness.rpc_base,
        326,
        "openhuman.medulla_abort",
        json!({ "sessionId": "sess-alpha" }),
    )
    .await;
    let aborted = ok(&aborted, "medulla_abort");
    assert_eq!(
        aborted.get("aborted").and_then(Value::as_bool),
        Some(true),
        "abort must report whether it actually aborted: {aborted}"
    );
    assert_eq!(
        aborted.get("sessionId").and_then(Value::as_str),
        Some("sess-alpha")
    );

    // ── a backend refusal keeps the backend's own vocabulary ────────────────
    let missing = rpc(
        &harness.rpc_base,
        327,
        "openhuman.medulla_get_session",
        json!({ "sessionId": "sess-missing" }),
    )
    .await;
    assert_eq!(
        error_kind(&missing, "medulla_get_session missing"),
        "SessionNotFound",
        "the backend's `errorCode` must become `data.kind`, so a host branches \
         on its vocabulary rather than on prose: {missing}"
    );

    // ── params validation ───────────────────────────────────────────────────
    let no_id = rpc(
        &harness.rpc_base,
        328,
        "openhuman.medulla_get_session",
        json!({}),
    )
    .await;
    assert!(
        err(&no_id, "medulla_get_session no id")
            .to_string()
            .contains("sessionId"),
        "an absent sessionId must be named: {no_id}"
    );

    let no_body = rpc(
        &harness.rpc_base,
        329,
        "openhuman.medulla_send_message",
        json!({ "sessionId": "sess-alpha" }),
    )
    .await;
    assert!(
        err(&no_body, "medulla_send_message no body")
            .to_string()
            .contains("body"),
        "an absent message body must be named: {no_body}"
    );

    backend_join.abort();
    harness.join.abort();
}

// ── replay + roster ─────────────────────────────────────────────────────────

/// `medulla_list_messages` / `medulla_list_events` are cursors, not page
/// offsets: passing the last seq already seen must return only what is newer.
/// That property is what makes a reconnect cheap, and it is asserted here by
/// replaying the same session twice with different cursors.
#[tokio::test]
async fn medulla_replay_is_a_cursor_and_tolerates_unmodelled_event_kinds() {
    let _lock = env_lock();
    let (backend_addr, state, backend_join) = serve_fixture_backend().await;
    let harness = setup(
        vec![EnvVarGuard::set(
            "OPENHUMAN_MEDULLA_BASE_URL",
            &format!("http://{backend_addr}"),
        )],
        true,
    )
    .await;

    // No cursor: everything.
    let all = rpc(
        &harness.rpc_base,
        330,
        "openhuman.medulla_list_messages",
        json!({ "sessionId": "sess-alpha" }),
    )
    .await;
    let all = ok(&all, "medulla_list_messages all");
    let messages = all
        .as_array()
        .unwrap_or_else(|| panic!("list_messages must return an array: {all}"));
    assert_eq!(messages.len(), 3, "the whole history: {all}");
    assert_eq!(
        messages[0].get("role").and_then(Value::as_str),
        Some("user"),
        "roles must round-trip lowercase through the modelled enum: {all}"
    );
    assert_eq!(
        messages[1].get("role").and_then(Value::as_str),
        Some("assistant")
    );
    assert_eq!(
        messages[0].get("body").and_then(Value::as_str),
        Some("first")
    );

    // With a cursor: only what is newer. This is the assertion that separates
    // a cursor from an offset — an offset of 2 would return one row, a cursor
    // of 2 returns exactly the rows with seq > 2.
    let tail = rpc(
        &harness.rpc_base,
        331,
        "openhuman.medulla_list_messages",
        json!({ "sessionId": "sess-alpha", "after": 2 }),
    )
    .await;
    let tail = ok(&tail, "medulla_list_messages after");
    let tail_messages = tail.as_array().expect("array");
    assert_eq!(tail_messages.len(), 1, "only seq 3 is newer than 2: {tail}");
    assert_eq!(
        tail_messages[0].get("seq").and_then(Value::as_i64),
        Some(3),
        "the cursor must be exclusive of its own value: {tail}"
    );

    // The cursor must actually have travelled to the backend as `after=2` —
    // filtering client-side would look identical from here.
    let message_cursors = state.queries_for("/medulla/v1/sessions/:id/messages");
    assert_eq!(
        message_cursors,
        vec![String::new(), "2".to_string()],
        "an absent cursor must send no `after` param at all, and a present one \
         must send its value: {message_cursors:?}"
    );

    // Events: same cursor semantics, plus forward compatibility.
    let events = rpc(
        &harness.rpc_base,
        332,
        "openhuman.medulla_list_events",
        json!({ "sessionId": "sess-alpha" }),
    )
    .await;
    let events = ok(&events, "medulla_list_events");
    let envelopes = events
        .as_array()
        .unwrap_or_else(|| panic!("list_events must return an array: {events}"));
    assert_eq!(
        envelopes.len(),
        3,
        "an unmodelled event kind must NOT be dropped — a newer backend would \
         silently lose rows: {events}"
    );
    assert_eq!(envelopes[0].get("seq").and_then(Value::as_u64), Some(1));
    assert_eq!(
        envelopes[0].get("at").and_then(Value::as_u64),
        Some(1_700_000_000_000),
        "the wall-clock stamp must survive: {events}"
    );
    assert_eq!(
        envelopes[0].get("sessionId").and_then(Value::as_str),
        Some("sess-alpha"),
        "the wire envelope's required sessionId must round-trip: {events}"
    );
    assert_eq!(
        envelopes[0]
            .get("event")
            .and_then(|e| e.get("kind"))
            .and_then(Value::as_str),
        Some("cycle_start"),
        "a modelled kind keeps its tag: {events}"
    );
    assert_eq!(
        envelopes[1]
            .get("event")
            .and_then(|e| e.get("delta"))
            .and_then(Value::as_str),
        Some("hi"),
        "a modelled kind keeps its payload: {events}"
    );
    // The unmodelled row survives, tagged with whatever the backend called it.
    assert_eq!(
        envelopes[2]
            .get("event")
            .and_then(|e| e.get("kind"))
            .and_then(Value::as_str),
        Some("some_future_kind"),
        "an unknown kind must keep its own tag rather than being relabelled: {events}"
    );

    let events_tail = rpc(
        &harness.rpc_base,
        333,
        "openhuman.medulla_list_events",
        json!({ "sessionId": "sess-alpha", "after": 1 }),
    )
    .await;
    assert_eq!(
        ok(&events_tail, "medulla_list_events after")
            .as_array()
            .map(Vec::len),
        Some(2),
        "the events cursor must drop what has already been seen: {events_tail}"
    );

    // ── roster ──────────────────────────────────────────────────────────────
    // The backend wraps the list in `{workers: [...]}`; the controller unwraps
    // it, so a caller gets the array and not the wrapper.
    let roster = rpc(
        &harness.rpc_base,
        334,
        "openhuman.medulla_roster",
        json!({}),
    )
    .await;
    let roster = ok(&roster, "medulla_roster");
    let workers = roster
        .as_array()
        .unwrap_or_else(|| panic!("roster must unwrap to a bare array, got: {roster}"));
    assert_eq!(workers.len(), 1, "one planted worker: {roster}");
    assert_eq!(
        workers[0].get("registryId").and_then(Value::as_str),
        Some("worker-1"),
        "the worker's stable id must survive: {roster}"
    );
    assert_eq!(
        workers[0].get("availability").and_then(Value::as_str),
        Some("idle")
    );
    assert_eq!(
        workers[0].get("harness").and_then(Value::as_str),
        Some("tinyagents")
    );
    assert_eq!(
        workers[0].get("selected").and_then(Value::as_bool),
        Some(true),
        "the roster's required `selected` flag must round-trip: {roster}"
    );

    backend_join.abort();
    harness.join.abort();
}
