//! End-to-end coverage for the notification centre and the small platform namespaces that had no
//! e2e target at all: `notification` (7 uncovered), `health` (2), `doctor` (2), `service`'s
//! daemon-host pair, `provider_surfaces` (2), `slack_memory` (2) and `announcements` (1).
//!
//! Everything here goes over the real JSON-RPC surface (`build_core_http_router`) against an
//! in-process axum mock for the hosted backend. Nothing reaches the network.
//!
//! This file is a **module** of the aggregated `raw_coverage_all` target, so it shares one process
//! with ~76 sibling suites and libtest runs them concurrently. Every case that mutates
//! process-global env (`HOME`, `BACKEND_URL`, …) holds `crate::SHARED_ENV_LOCK`, and the two
//! process-global registries touched here — the health component registry and the
//! `provider_surfaces` respond queue — are addressed with keys unique to this file so a sibling
//! suite's rows cannot make an assertion here pass or fail by accident.
//!
//! Run with:
//!   ~/tinyhuman/ci-slot.sh cargo test --test raw_coverage_all \
//!       --features "$(bash scripts/ci/product-features.sh)" notification_platform

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::auth::{get_rpc_token, init_rpc_token};
use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::config::rpc::load_config_with_timeout;
use openhuman_core::openhuman::desktop::notifications::store as notification_store;
use openhuman_core::openhuman::desktop::notifications::types::{
    CoreNotificationCategory, CoreNotificationEvent,
};
use openhuman_core::openhuman::platform::health::{mark_component_error, mark_component_ok};

// ── env serialisation ────────────────────────────────────────────────────────

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn platform_e2e_env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const TEST_JWT: &str = "e2e-notification-platform-jwt";
const TEST_USER: &str = "notification-platform-e2e-user";

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
        let token_dir = std::env::temp_dir().join("openhuman-notification-platform-e2e-auth");
        std::fs::create_dir_all(&token_dir).expect("rpc token dir");
        init_rpc_token(&token_dir).expect("init rpc token for notification_platform_e2e");
        get_rpc_token().expect("an RPC token is initialised for this process")
    })
}

fn ensure_rpc_auth() {
    let _ = rpc_bearer();
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

/// What the mock should answer on `GET /announcements/latest`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AnnouncementMode {
    /// Return a real announcement object.
    Present,
    /// Return 404 — the "no announcement for this user" case the adapter folds into `null`.
    NotFound,
}

#[derive(Clone)]
struct BackendState {
    announcement: AnnouncementMode,
    /// Rows served by `GET /agent-integrations/composio/connections`.
    connections: Arc<Value>,
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
        "data": { "_id": TEST_USER, "username": "notification-platform-e2e" }
    })))
}

async fn mock_announcements_latest(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !is_authed(&headers) {
        return Err(unauthorized());
    }
    match state.announcement {
        AnnouncementMode::Present => Ok(Json(json!({
            "success": true,
            "data": {
                "id": "ann-1",
                "title": "Scheduled maintenance",
                "body": "The backend is being upgraded.",
                "severity": "info"
            }
        }))),
        AnnouncementMode::NotFound => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": "no announcement" })),
        )),
    }
}

async fn mock_composio_connections(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !is_authed(&headers) {
        return Err(unauthorized());
    }
    Ok(Json(json!({ "success": true, "data": (*state.connections).clone() })))
}

fn mock_backend_router(state: BackendState) -> Router {
    Router::new()
        .route("/settings", get(mock_current_user))
        .route("/auth/me", get(mock_current_user))
        .route(
            "/announcements/latest",
            get(mock_announcements_latest).with_state(state.clone()),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(mock_composio_connections).with_state(state),
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
    write_cfg(&openhuman_dir.join("users").join(TEST_USER), &cfg);
}

async fn post_json_rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
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

struct Harness {
    rpc_base: String,
    openhuman_home: std::path::PathBuf,
    mock_join: tokio::task::JoinHandle<()>,
    rpc_join: tokio::task::JoinHandle<()>,
    _home: EnvGuard,
    _ws: EnvGuard,
    _backend: EnvGuard,
    _vite: EnvGuard,
    _tmp: tempfile::TempDir,
}

impl Harness {
    async fn start(state: BackendState) -> Self {
        let tmp = tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let openhuman_home = home.join(".openhuman");
        let _home = EnvGuard::set_to_path("HOME", &home);
        let _ws = EnvGuard::unset("OPENHUMAN_WORKSPACE");
        let _backend = EnvGuard::unset("BACKEND_URL");
        let _vite = EnvGuard::unset("VITE_BACKEND_URL");

        let (mock_addr, mock_join) = serve_ephemeral(mock_backend_router(state)).await;
        write_test_config(&openhuman_home, &format!("http://{mock_addr}"));

        let (rpc_addr, rpc_join) = serve_ephemeral(build_core_http_router(false)).await;
        Self {
            rpc_base: format!("http://{rpc_addr}"),
            openhuman_home,
            mock_join,
            rpc_join,
            _home,
            _ws,
            _backend,
            _vite,
            _tmp: tmp,
        }
    }

    async fn sign_in(&self) {
        let store = post_json_rpc(
            &self.rpc_base,
            1,
            "openhuman.auth_store_session",
            json!({ "token": TEST_JWT, "user_id": TEST_USER }),
        )
        .await;
        assert_no_jsonrpc_error(&store, "auth_store_session");
    }

    fn stop(self) {
        self.mock_join.abort();
        self.rpc_join.abort();
    }
}

fn default_state() -> BackendState {
    BackendState {
        announcement: AnnouncementMode::Present,
        connections: Arc::new(json!({ "connections": [] })),
    }
}

// ── notification centre ──────────────────────────────────────────────────────

/// The full integration-notification lifecycle over RPC: ingest → list → stats → mark read →
/// dismiss → mark acted, with the not-found branch of each mutator checked too.
///
/// `notification_settings_set` / `_get` / `_ingest` already had coverage in `json_rpc_e2e.rs`
/// (only for the *disabled-provider skip* path); everything downstream of a successful ingest —
/// which is the whole notification centre — did not.
///
/// Covers: `openhuman.notification_list`, `openhuman.notification_stats`,
/// `openhuman.notification_mark_read`, `openhuman.notification_dismiss`,
/// `openhuman.notification_mark_acted`.
#[tokio::test]
async fn notification_centre_lifecycle_over_rpc() {
    let _env_lock = platform_e2e_env_lock();
    let h = Harness::start(default_state()).await;

    // Enable the provider so ingest stores rather than skipping.
    let enabled = post_json_rpc(
        &h.rpc_base,
        9001,
        "openhuman.notification_settings_set",
        json!({
            "provider": "gmail",
            "enabled": true,
            "importance_threshold": 0.0,
            "route_to_orchestrator": false
        }),
    )
    .await;
    assert_eq!(
        assert_no_jsonrpc_error(&enabled, "notification_settings_set")
            .get("ok")
            .and_then(Value::as_bool),
        Some(true)
    );

    let ingested = post_json_rpc(
        &h.rpc_base,
        9002,
        "openhuman.notification_ingest",
        json!({
            "provider": "gmail",
            "account_id": "acct-e2e",
            "title": "Quarterly report",
            "body": "The Q3 numbers are attached.",
            "raw_payload": { "messageId": "m-1" }
        }),
    )
    .await;
    let result = assert_no_jsonrpc_error(&ingested, "notification_ingest");
    assert_eq!(
        result.get("skipped").and_then(Value::as_bool),
        Some(false),
        "an enabled provider must not skip: {result}"
    );
    let id = result
        .get("id")
        .and_then(Value::as_str)
        .expect("ingest returns the new record id")
        .to_string();
    assert!(!id.is_empty());

    // ── list surfaces the stored record with the fields it was ingested with.
    let listed = post_json_rpc(
        &h.rpc_base,
        9003,
        "openhuman.notification_list",
        json!({ "provider": "gmail", "limit": 10 }),
    )
    .await;
    let result = assert_no_jsonrpc_error(&listed, "notification_list");
    let items = result
        .get("items")
        .and_then(Value::as_array)
        .expect("items array");
    let row = items
        .iter()
        .find(|i| i.get("id").and_then(Value::as_str) == Some(id.as_str()))
        .unwrap_or_else(|| panic!("the ingested notification must be listed: {result}"));
    assert_eq!(
        row.get("title").and_then(Value::as_str),
        Some("Quarterly report"),
        "list must return the stored title, not a placeholder: {row}"
    );
    assert_eq!(
        row.get("body").and_then(Value::as_str),
        Some("The Q3 numbers are attached.")
    );
    assert_eq!(row.get("provider").and_then(Value::as_str), Some("gmail"));
    assert_eq!(row.get("account_id").and_then(Value::as_str), Some("acct-e2e"));
    assert_eq!(
        row.get("raw_payload").and_then(|p| p.get("messageId")),
        Some(&json!("m-1")),
        "the raw payload must round-trip through SQLite intact: {row}"
    );
    assert_eq!(
        row.get("status").and_then(Value::as_str),
        Some("unread"),
        "a freshly ingested notification is unread: {row}"
    );
    assert!(
        result
            .get("unread_count")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            >= 1,
        "unread_count must account for the record just ingested: {result}"
    );

    // ── a provider filter that matches nothing returns an empty page, not everything.
    let other = post_json_rpc(
        &h.rpc_base,
        9004,
        "openhuman.notification_list",
        json!({ "provider": "no-such-provider" }),
    )
    .await;
    let result = assert_no_jsonrpc_error(&other, "notification_list filtered");
    assert_eq!(
        result.get("items").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "the provider filter must actually filter: {result}"
    );

    // ── stats counts the record and buckets it by provider.
    let stats = post_json_rpc(&h.rpc_base, 9005, "openhuman.notification_stats", json!({})).await;
    let result = assert_no_jsonrpc_error(&stats, "notification_stats");
    assert_eq!(
        result.get("total").and_then(Value::as_i64),
        Some(1),
        "one ingested record in a fresh workspace: {result}"
    );
    assert_eq!(result.get("unread").and_then(Value::as_i64), Some(1));
    assert_eq!(
        result.get("by_provider").and_then(|p| p.get("gmail")),
        Some(&json!(1)),
        "stats must bucket by provider slug: {result}"
    );
    assert!(
        result.get("by_action").is_some(),
        "by_action must be present even when nothing has been triaged yet: {result}"
    );

    // ── mark_read flips the status and the unread count.
    let read = post_json_rpc(
        &h.rpc_base,
        9006,
        "openhuman.notification_mark_read",
        json!({ "id": id }),
    )
    .await;
    assert_eq!(
        assert_no_jsonrpc_error(&read, "notification_mark_read")
            .get("ok")
            .and_then(Value::as_bool),
        Some(true)
    );

    let after_read = post_json_rpc(
        &h.rpc_base,
        9007,
        "openhuman.notification_list",
        json!({ "provider": "gmail" }),
    )
    .await;
    let result = assert_no_jsonrpc_error(&after_read, "notification_list after mark_read");
    assert_eq!(
        result.get("unread_count").and_then(Value::as_i64),
        Some(0),
        "mark_read must be visible in the unread count: {result}"
    );
    let row = result["items"]
        .as_array()
        .and_then(|items| items.first())
        .expect("the record is still listed after being read");
    assert_eq!(
        row.get("status").and_then(Value::as_str),
        Some("read"),
        "mark_read must persist the status transition: {row}"
    );

    // ── mark_acted and dismiss each report whether a row actually changed.
    let acted = post_json_rpc(
        &h.rpc_base,
        9008,
        "openhuman.notification_mark_acted",
        json!({ "id": id }),
    )
    .await;
    assert_eq!(
        assert_no_jsonrpc_error(&acted, "notification_mark_acted")
            .get("ok")
            .and_then(Value::as_bool),
        Some(true),
        "acting on an existing notification must report ok=true"
    );

    let acted_missing = post_json_rpc(
        &h.rpc_base,
        9009,
        "openhuman.notification_mark_acted",
        json!({ "id": "no-such-notification" }),
    )
    .await;
    assert_eq!(
        assert_no_jsonrpc_error(&acted_missing, "notification_mark_acted, missing id")
            .get("ok")
            .and_then(Value::as_bool),
        Some(false),
        "mark_acted must report ok=false for an id that matched no row, not a blanket true"
    );

    let dismissed = post_json_rpc(
        &h.rpc_base,
        9010,
        "openhuman.notification_dismiss",
        json!({ "id": id }),
    )
    .await;
    assert_eq!(
        assert_no_jsonrpc_error(&dismissed, "notification_dismiss")
            .get("ok")
            .and_then(Value::as_bool),
        Some(true)
    );

    let dismiss_missing = post_json_rpc(
        &h.rpc_base,
        9011,
        "openhuman.notification_dismiss",
        json!({ "id": "no-such-notification" }),
    )
    .await;
    assert_eq!(
        assert_no_jsonrpc_error(&dismiss_missing, "notification_dismiss, missing id")
            .get("ok")
            .and_then(Value::as_bool),
        Some(false),
        "dismiss must report ok=false for an id that matched no row"
    );

    // ── the id param is required on every mutator.
    for (rpc_id, method) in [
        (9012, "openhuman.notification_mark_read"),
        (9013, "openhuman.notification_dismiss"),
        (9014, "openhuman.notification_mark_acted"),
    ] {
        let response = post_json_rpc(&h.rpc_base, rpc_id, method, json!({})).await;
        let message = jsonrpc_error_message(&response, method);
        assert!(
            message.contains("missing required param 'id'"),
            "{method} must name the missing param, got: {message}"
        );
    }

    h.stop();
}

/// The persisted core-notification sync-down (#3805): list → mark read → list again.
///
/// Core notifications have no ingest RPC — the only writer is the bus subscriber — so the row is
/// seeded through `store::insert_core_notification` using the *same* `Config` the RPC handlers
/// load, then read back over the wire.
///
/// Covers: `openhuman.notification_core_list`, `openhuman.notification_core_mark_read`.
#[tokio::test]
async fn notification_core_sync_down_and_mark_read() {
    let _env_lock = platform_e2e_env_lock();
    let h = Harness::start(default_state()).await;

    // Empty workspace first: the contract is an empty page, not an error.
    let empty = post_json_rpc(
        &h.rpc_base,
        9101,
        "openhuman.notification_core_list",
        json!({}),
    )
    .await;
    let result = assert_no_jsonrpc_error(&empty, "notification_core_list (empty)");
    assert_eq!(
        result.get("items").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "a fresh workspace has no core notifications: {result}"
    );
    assert_eq!(result.get("unread_count").and_then(Value::as_i64), Some(0));

    let config = load_config_with_timeout()
        .await
        .expect("the same config the RPC handlers resolve");

    let unread_event = CoreNotificationEvent {
        id: "e2e-core-unread".to_string(),
        category: CoreNotificationCategory::Agents,
        title: "Sub-agent finished".to_string(),
        body: "The research run completed.".to_string(),
        deep_link: Some("/threads/t-1".to_string()),
        timestamp_ms: 1_700_000_002_000,
        actions: None,
        workspace: None,
        workspace_revision: None,
    };
    let read_later_event = CoreNotificationEvent {
        id: "e2e-core-second".to_string(),
        category: CoreNotificationCategory::System,
        title: "Update available".to_string(),
        body: "A new core build is ready.".to_string(),
        deep_link: None,
        timestamp_ms: 1_700_000_001_000,
        actions: None,
        workspace: None,
        workspace_revision: None,
    };
    assert!(
        notification_store::insert_core_notification(&config, &unread_event)
            .expect("seed core notification"),
        "the first insert of an id must report that it wrote a row"
    );
    assert!(
        notification_store::insert_core_notification(&config, &read_later_event)
            .expect("seed second core notification")
    );
    assert!(
        !notification_store::insert_core_notification(&config, &unread_event)
            .expect("re-insert core notification"),
        "the id is the primary key, so a re-publish must dedupe rather than duplicate"
    );

    let listed = post_json_rpc(
        &h.rpc_base,
        9102,
        "openhuman.notification_core_list",
        json!({}),
    )
    .await;
    let result = assert_no_jsonrpc_error(&listed, "notification_core_list");
    let items = result
        .get("items")
        .and_then(Value::as_array)
        .expect("items array");
    assert_eq!(items.len(), 2, "both seeded rows sync down: {result}");
    assert_eq!(
        items[0].get("id").and_then(Value::as_str),
        Some("e2e-core-unread"),
        "rows come back newest-first by timestamp_ms: {result}"
    );
    assert_eq!(
        items[0].get("title").and_then(Value::as_str),
        Some("Sub-agent finished")
    );
    assert_eq!(
        items[0].get("category").and_then(Value::as_str),
        Some("agents"),
        "the category serialises lowercase for the frontend notification centre: {}",
        items[0]
    );
    assert_eq!(
        items[0].get("deep_link").and_then(Value::as_str),
        Some("/threads/t-1"),
        "the deep link must survive the JSON round-trip through SQLite: {}",
        items[0]
    );
    assert_eq!(result.get("unread_count").and_then(Value::as_i64), Some(2));

    // ── mark one read: it drops out of the default (unread-only) page.
    let marked = post_json_rpc(
        &h.rpc_base,
        9103,
        "openhuman.notification_core_mark_read",
        json!({ "id": "e2e-core-unread" }),
    )
    .await;
    assert_eq!(
        assert_no_jsonrpc_error(&marked, "notification_core_mark_read")
            .get("ok")
            .and_then(Value::as_bool),
        Some(true)
    );

    let after = post_json_rpc(
        &h.rpc_base,
        9104,
        "openhuman.notification_core_list",
        json!({}),
    )
    .await;
    let result = assert_no_jsonrpc_error(&after, "notification_core_list after mark_read");
    let items = result
        .get("items")
        .and_then(Value::as_array)
        .expect("items array");
    assert_eq!(
        items.len(),
        1,
        "only_unread defaults to true, so the read row must be gone: {result}"
    );
    assert_eq!(
        items[0].get("id").and_then(Value::as_str),
        Some("e2e-core-second")
    );
    assert_eq!(result.get("unread_count").and_then(Value::as_i64), Some(1));

    // ── only_unread=false brings the read row back.
    let all = post_json_rpc(
        &h.rpc_base,
        9105,
        "openhuman.notification_core_list",
        json!({ "only_unread": false }),
    )
    .await;
    let result = assert_no_jsonrpc_error(&all, "notification_core_list, only_unread=false");
    assert_eq!(
        result.get("items").and_then(Value::as_array).map(Vec::len),
        Some(2),
        "only_unread=false must include read rows: {result}"
    );
    assert_eq!(
        result.get("unread_count").and_then(Value::as_i64),
        Some(1),
        "unread_count is a count of unread rows regardless of what the page returned: {result}"
    );

    // ── limit is honoured.
    let limited = post_json_rpc(
        &h.rpc_base,
        9106,
        "openhuman.notification_core_list",
        json!({ "only_unread": false, "limit": 1 }),
    )
    .await;
    assert_eq!(
        assert_no_jsonrpc_error(&limited, "notification_core_list, limit=1")
            .get("items")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1),
        "limit must bound the page"
    );

    // ── an unknown id changes nothing and says so.
    let missing = post_json_rpc(
        &h.rpc_base,
        9107,
        "openhuman.notification_core_mark_read",
        json!({ "id": "e2e-core-does-not-exist" }),
    )
    .await;
    assert_eq!(
        assert_no_jsonrpc_error(&missing, "notification_core_mark_read, missing id")
            .get("ok")
            .and_then(Value::as_bool),
        Some(false),
        "marking a non-existent core notification must report ok=false"
    );

    let no_id = post_json_rpc(
        &h.rpc_base,
        9108,
        "openhuman.notification_core_mark_read",
        json!({}),
    )
    .await;
    assert!(
        jsonrpc_error_message(&no_id, "notification_core_mark_read without id")
            .contains("missing required param 'id'")
    );

    h.stop();
}

// ── health ───────────────────────────────────────────────────────────────────

/// `health_snapshot` reflects what the component registry was told, and `health_system_info`
/// reports this process.
///
/// The component registry is process-global and shared with every other suite in this binary, so
/// the assertions are on components this file names uniquely — never on the size of the map.
///
/// Covers: `openhuman.health_snapshot`, `openhuman.health_system_info`.
#[tokio::test]
async fn health_snapshot_and_system_info_report_this_process() {
    let _env_lock = platform_e2e_env_lock();
    let h = Harness::start(default_state()).await;

    mark_component_ok("e2e_platform_probe_ok");
    mark_component_error("e2e_platform_probe_bad", "synthetic failure for the e2e probe");

    let snapshot = post_json_rpc(&h.rpc_base, 9201, "openhuman.health_snapshot", json!({})).await;
    let result = peel(assert_no_jsonrpc_error(&snapshot, "health_snapshot"));

    let components = result
        .get("components")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("snapshot must carry a components map: {result}"));

    let ok = components
        .get("e2e_platform_probe_ok")
        .unwrap_or_else(|| panic!("a component marked ok must appear in the snapshot: {result}"));
    assert_eq!(
        ok.get("status").and_then(Value::as_str),
        Some("ok"),
        "mark_component_ok must surface as status=ok: {ok}"
    );
    assert!(
        ok.get("last_ok").map(|v| v.is_string()).unwrap_or(false),
        "an ok component records when it was last ok: {ok}"
    );
    assert!(
        ok.get("last_error").map(Value::is_null).unwrap_or(false),
        "marking ok must clear any previous error: {ok}"
    );

    let bad = components
        .get("e2e_platform_probe_bad")
        .unwrap_or_else(|| panic!("a component marked error must appear: {result}"));
    assert_eq!(bad.get("status").and_then(Value::as_str), Some("error"));
    assert_eq!(
        bad.get("last_error").and_then(Value::as_str),
        Some("synthetic failure for the e2e probe"),
        "the error message must be carried verbatim, not swallowed: {bad}"
    );

    let snapshot_pid = result
        .get("pid")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("snapshot must carry a pid: {result}"));
    assert_eq!(
        snapshot_pid,
        u64::from(std::process::id()),
        "the snapshot describes *this* process"
    );
    assert!(
        result.get("uptime_seconds").and_then(Value::as_u64).is_some(),
        "snapshot must carry uptime_seconds: {result}"
    );
    assert!(
        result
            .get("updated_at")
            .and_then(Value::as_str)
            .map(|t| chrono::DateTime::parse_from_rfc3339(t).is_ok())
            .unwrap_or(false),
        "updated_at must be RFC3339: {result}"
    );

    let info = post_json_rpc(&h.rpc_base, 9202, "openhuman.health_system_info", json!({})).await;
    let result = peel(assert_no_jsonrpc_error(&info, "health_system_info"));
    assert_eq!(
        result.get("os").and_then(Value::as_str),
        Some(std::env::consts::OS),
        "system_info must report the real target OS: {result}"
    );
    assert_eq!(
        result.get("arch").and_then(Value::as_str),
        Some(std::env::consts::ARCH)
    );
    assert_eq!(
        result.get("version").and_then(Value::as_str),
        Some(env!("CARGO_PKG_VERSION")),
        "system_info reports the core crate version this binary was built from: {result}"
    );
    // The declared schema says `pid: String`; the handler serialises a `u32`. Asserting the real
    // wire type pins the mismatch rather than hiding it — see
    // bugs/e2e-wave-health-system-info-pid-type.md.
    assert_eq!(
        result.get("pid").and_then(Value::as_u64),
        Some(u64::from(std::process::id())),
        "system_info must report this process's pid: {result}"
    );

    h.stop();
}

// ── doctor ───────────────────────────────────────────────────────────────────

/// `doctor_report` returns a self-consistent diagnostics report, and `doctor_models` reports one
/// entry per registered provider.
///
/// Covers: `openhuman.doctor_report`, `openhuman.doctor_models`.
#[tokio::test]
async fn doctor_report_and_models_are_internally_consistent() {
    let _env_lock = platform_e2e_env_lock();
    let h = Harness::start(default_state()).await;

    let report = post_json_rpc(&h.rpc_base, 9301, "openhuman.doctor_report", json!({})).await;
    let result = peel(assert_no_jsonrpc_error(&report, "doctor_report"));

    let items = result
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("doctor_report must return items: {result}"));
    assert!(
        !items.is_empty(),
        "the doctor always runs at least the config and workspace checks: {result}"
    );
    for item in items {
        assert!(
            matches!(
                item.get("severity").and_then(Value::as_str),
                Some("Ok" | "Warn" | "Error")
            ),
            "every item carries one of the three severities: {item}"
        );
        assert!(
            item.get("category")
                .and_then(Value::as_str)
                .map(|c| !c.is_empty())
                .unwrap_or(false),
            "every item is attributed to a category: {item}"
        );
        assert!(
            item.get("message")
                .and_then(Value::as_str)
                .map(|m| !m.is_empty())
                .unwrap_or(false),
            "every item carries a human-readable message: {item}"
        );
    }
    assert!(
        items
            .iter()
            .any(|i| i.get("category").and_then(Value::as_str) == Some("config")),
        "the config check always runs: {result}"
    );
    assert!(
        items
            .iter()
            .any(|i| i.get("category").and_then(Value::as_str) == Some("workspace")),
        "the workspace check always runs: {result}"
    );

    // The summary is derived from the items — if the tally drifts, this is what catches it.
    let summary = result
        .get("summary")
        .unwrap_or_else(|| panic!("doctor_report must return a summary: {result}"));
    let count_of = |severity: &str| {
        items
            .iter()
            .filter(|i| i.get("severity").and_then(Value::as_str) == Some(severity))
            .count() as u64
    };
    assert_eq!(
        summary.get("ok").and_then(Value::as_u64),
        Some(count_of("Ok")),
        "summary.ok must equal the number of Ok items: {result}"
    );
    assert_eq!(
        summary.get("warnings").and_then(Value::as_u64),
        Some(count_of("Warn")),
        "summary.warnings must equal the number of Warn items: {result}"
    );
    assert_eq!(
        summary.get("errors").and_then(Value::as_u64),
        Some(count_of("Error")),
        "summary.errors must equal the number of Error items: {result}"
    );

    // ── models: one entry per registered inference provider, summary consistent with them.
    let models = post_json_rpc(
        &h.rpc_base,
        9302,
        "openhuman.doctor_models",
        json!({ "use_cache": false }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&models, "doctor_models"));
    let entries = result
        .get("entries")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("doctor_models must return entries: {result}"));
    assert!(
        !entries.is_empty(),
        "the provider registry is never empty, and an empty target list is a hard error: {result}"
    );
    for entry in entries {
        assert!(
            entry
                .get("provider")
                .and_then(Value::as_str)
                .map(|p| !p.is_empty())
                .unwrap_or(false),
            "every probe entry names its provider: {entry}"
        );
        assert!(
            matches!(
                entry.get("outcome").and_then(Value::as_str),
                Some("Ok" | "Skipped" | "AuthOrAccess" | "Error")
            ),
            "every probe entry carries a known outcome: {entry}"
        );
    }
    let summary = result
        .get("summary")
        .unwrap_or_else(|| panic!("doctor_models must return a summary: {result}"));
    let total: u64 = ["ok", "skipped", "auth_or_access", "errors"]
        .iter()
        .map(|k| summary.get(*k).and_then(Value::as_u64).unwrap_or_else(|| {
            panic!("summary must carry {k}: {result}")
        }))
        .sum();
    assert_eq!(
        total,
        entries.len() as u64,
        "the summary buckets must add up to the number of entries: {result}"
    );

    // `use_cache` is optional and defaults to true; a non-bool must be rejected, not coerced.
    // The refusal comes from `core::all::validate_params`, which type-checks every present param
    // against its declared `TypeSchema` before dispatch.
    let bad_param = post_json_rpc(
        &h.rpc_base,
        9303,
        "openhuman.doctor_models",
        json!({ "use_cache": "yes please" }),
    )
    .await;
    let message = jsonrpc_error_message(&bad_param, "doctor_models with a non-bool use_cache");
    assert!(
        message.contains("invalid type for param 'use_cache'")
            && message.contains("expected bool")
            && message.contains("got string"),
        "a wrongly-typed optional param must be rejected naming the expected and actual types, \
         got: {message}"
    );

    h.stop();
}

// ── service: daemon-host preferences ─────────────────────────────────────────

/// `service_daemon_host_get` / `_set` round-trip through `daemon_host_config.json`.
///
/// Covers: `openhuman.service_daemon_host_get`, `openhuman.service_daemon_host_set`.
#[tokio::test]
async fn service_daemon_host_preferences_round_trip_to_disk() {
    let _env_lock = platform_e2e_env_lock();
    let h = Harness::start(default_state()).await;

    let initial = post_json_rpc(
        &h.rpc_base,
        9401,
        "openhuman.service_daemon_host_get",
        json!({}),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&initial, "service_daemon_host_get"));
    assert_eq!(
        result.get("showTray").and_then(Value::as_bool),
        Some(true),
        "with no file on disk the default is tray-visible, and the field is camelCase: {result}"
    );

    // The request param is snake_case (`show_tray`) while the response field is camelCase
    // (`showTray`) — `DaemonHostConfig` carries `rename_all = \"camelCase\"` but
    // `DaemonHostSetParams` does not. Pinning both spellings here keeps that asymmetry visible.
    let set = post_json_rpc(
        &h.rpc_base,
        9402,
        "openhuman.service_daemon_host_set",
        json!({ "show_tray": false }),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&set, "service_daemon_host_set"));
    assert_eq!(
        result.get("showTray").and_then(Value::as_bool),
        Some(false),
        "set must echo the value it stored: {result}"
    );

    // `daemon_host_set` writes to `config.config_path.parent()` — the *resolved* config
    // directory, which under a pre-login user is `.openhuman/users/local/`, not `.openhuman/`.
    // Resolving it here through the same loader the op uses asserts the real contract ("next to
    // config.toml") instead of a guess at where that lands.
    let config = load_config_with_timeout()
        .await
        .expect("the same config the daemon-host op resolves");
    let config_dir = config
        .config_path
        .parent()
        .expect("config.toml has a parent directory");
    assert!(
        config_dir.starts_with(&h.openhuman_home),
        "the resolved config dir must be inside this test's temp HOME, got {}",
        config_dir.display()
    );
    let persisted = config_dir.join("daemon_host_config.json");
    assert!(
        persisted.exists(),
        "the preference must be written next to config.toml, at {}",
        persisted.display()
    );
    let on_disk: Value = serde_json::from_str(
        &std::fs::read_to_string(&persisted).expect("read daemon_host_config.json"),
    )
    .expect("daemon_host_config.json is valid JSON");
    assert_eq!(
        on_disk.get("showTray").and_then(Value::as_bool),
        Some(false),
        "the on-disk file uses the camelCase key: {on_disk}"
    );

    let reread = post_json_rpc(
        &h.rpc_base,
        9403,
        "openhuman.service_daemon_host_get",
        json!({}),
    )
    .await;
    assert_eq!(
        peel(assert_no_jsonrpc_error(&reread, "service_daemon_host_get after set"))
            .get("showTray")
            .and_then(Value::as_bool),
        Some(false),
        "get must read back what set wrote, not the default"
    );

    // `show_tray` is required — there is no implicit default on the write path.
    let missing = post_json_rpc(
        &h.rpc_base,
        9404,
        "openhuman.service_daemon_host_set",
        json!({}),
    )
    .await;
    assert!(
        jsonrpc_error_message(&missing, "service_daemon_host_set without show_tray")
            .contains("show_tray"),
        "the error must name the missing field"
    );

    h.stop();
}

// ── provider surfaces ────────────────────────────────────────────────────────

/// The local respond queue: ingest an event, see it in the queue, re-ingest and see it upserted
/// rather than duplicated.
///
/// The queue is a process-global `Vec` shared with every suite in this binary, so the assertions
/// are on rows keyed to this file (`provider = "e2e-surface"`), never on the queue's length.
///
/// Covers: `openhuman.provider_surfaces_ingest_event`,
/// `openhuman.provider_surfaces_list_queue`.
#[tokio::test]
async fn provider_surfaces_respond_queue_upserts_by_event_identity() {
    let _env_lock = platform_e2e_env_lock();
    let h = Harness::start(default_state()).await;

    let event = json!({
        "provider": "e2e-surface",
        "account_id": "acct-7",
        "event_kind": "message",
        "entity_id": "msg-1",
        "thread_id": "thr-1",
        "title": "Ping",
        "snippet": "are you around?",
        "sender_name": "A Colleague",
        "timestamp": "2026-09-07T10:00:00Z",
        "requires_attention": true
    });

    let ingested = post_json_rpc(
        &h.rpc_base,
        9501,
        "openhuman.provider_surfaces_ingest_event",
        event.clone(),
    )
    .await;
    let result = assert_no_jsonrpc_error(&ingested, "provider_surfaces_ingest_event");
    let item = result
        .get("data")
        .unwrap_or_else(|| panic!("ingest returns an ApiEnvelope with a data member: {result}"));
    assert_eq!(
        item.get("id").and_then(Value::as_str),
        Some("e2e-surface:acct-7:message:msg-1"),
        "the queue id is derived from provider:account:kind:entity — that composite is what \
         makes a re-delivery an upsert instead of a duplicate: {item}"
    );
    assert_eq!(
        item.get("status").and_then(Value::as_str),
        Some("pending"),
        "a newly queued item starts pending: {item}"
    );
    assert_eq!(
        item.get("requires_attention").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(item.get("title").and_then(Value::as_str), Some("Ping"));
    assert!(
        result
            .get("meta")
            .and_then(|m| m.get("request_id"))
            .and_then(Value::as_str)
            .map(|id| !id.is_empty())
            .unwrap_or(false),
        "the envelope carries a request id: {result}"
    );
    assert!(
        result.get("error").map(Value::is_null).unwrap_or(true),
        "a successful ingest leaves the envelope's error member null: {result}"
    );

    // ── re-ingest with a changed title: same identity ⇒ one row, updated in place.
    let mut updated_event = event.clone();
    updated_event["title"] = json!("Ping (edited)");
    updated_event["requires_attention"] = json!(false);
    let reingested = post_json_rpc(
        &h.rpc_base,
        9502,
        "openhuman.provider_surfaces_ingest_event",
        updated_event,
    )
    .await;
    assert_no_jsonrpc_error(&reingested, "provider_surfaces_ingest_event (upsert)");

    let listed = post_json_rpc(
        &h.rpc_base,
        9503,
        "openhuman.provider_surfaces_list_queue",
        json!({}),
    )
    .await;
    let result = assert_no_jsonrpc_error(&listed, "provider_surfaces_list_queue");
    let items = result
        .get("data")
        .and_then(|d| d.get("items"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("list_queue returns data.items: {result}"));
    let ours: Vec<_> = items
        .iter()
        .filter(|i| i.get("provider").and_then(Value::as_str) == Some("e2e-surface"))
        .collect();
    assert_eq!(
        ours.len(),
        1,
        "re-ingesting the same provider/account/kind/entity must upsert, not append: {result}"
    );
    assert_eq!(
        ours[0].get("title").and_then(Value::as_str),
        Some("Ping (edited)"),
        "the upsert must carry the newer field values: {}",
        ours[0]
    );
    assert_eq!(
        ours[0].get("requires_attention").and_then(Value::as_bool),
        Some(false),
        "the upsert must overwrite requires_attention, not OR it: {}",
        ours[0]
    );
    assert_eq!(
        result
            .get("data")
            .and_then(|d| d.get("count"))
            .and_then(Value::as_u64),
        Some(items.len() as u64),
        "the reported count must match the items actually returned: {result}"
    );

    // ── the event contract is strict on both sides: unknown fields and missing ones both fail.
    let mut unknown_field = event.clone();
    unknown_field["not_a_real_field"] = json!("surprise");
    let rejected = post_json_rpc(
        &h.rpc_base,
        9504,
        "openhuman.provider_surfaces_ingest_event",
        unknown_field,
    )
    .await;
    let message = jsonrpc_error_message(&rejected, "ingest_event with an unknown field");
    assert!(
        message.contains("not_a_real_field"),
        "an undeclared key must be refused naming it — `core::all::validate_params` rejects it \
         against the schema, and `ProviderEvent`'s `deny_unknown_fields` is the second line of \
         defence behind that; got: {message}"
    );

    let mut missing_timestamp = event.clone();
    missing_timestamp
        .as_object_mut()
        .expect("object")
        .remove("timestamp");
    let rejected = post_json_rpc(
        &h.rpc_base,
        9505,
        "openhuman.provider_surfaces_ingest_event",
        missing_timestamp,
    )
    .await;
    assert!(
        jsonrpc_error_message(&rejected, "ingest_event without timestamp").contains("timestamp"),
        "a missing required field must name itself"
    );

    h.stop();
}

// ── slack_memory ─────────────────────────────────────────────────────────────

/// `slack_memory_sync_status` filters the Composio connection list down to active Slack rows and
/// reports the degraded detail fields at zero; `slack_memory_sync_trigger` refuses an id that is
/// not in that set.
///
/// Covers: `openhuman.slack_memory_sync_status`, `openhuman.slack_memory_sync_trigger`.
#[tokio::test]
async fn slack_memory_sync_status_filters_to_active_slack_connections() {
    let _env_lock = platform_e2e_env_lock();

    // Three rows that between them exercise every branch of the filter: an active slack one, an
    // active connection for a *different* toolkit, and a slack one that is not active.
    let state = BackendState {
        announcement: AnnouncementMode::Present,
        connections: Arc::new(json!({
            "connections": [
                { "id": "conn-slack-live", "toolkit": " Slack ", "status": "ACTIVE" },
                { "id": "conn-gmail-live", "toolkit": "gmail", "status": "ACTIVE" },
                { "id": "conn-slack-pending", "toolkit": "slack", "status": "PENDING" }
            ]
        })),
    };
    let h = Harness::start(state).await;
    h.sign_in().await;

    let status = post_json_rpc(
        &h.rpc_base,
        9601,
        "openhuman.slack_memory_sync_status",
        json!({}),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&status, "slack_memory_sync_status"));
    let rows = result
        .get("connections")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("sync_status returns a connections array: {result}"));
    assert_eq!(
        rows.len(),
        1,
        "only the active Slack connection is a sync candidate — the gmail row and the pending \
         slack row must both be filtered out: {result}"
    );
    let row = &rows[0];
    assert_eq!(
        row.get("connection_id").and_then(Value::as_str),
        Some("conn-slack-live"),
        "the toolkit match is case- and whitespace-insensitive (` Slack ` matched): {row}"
    );
    // The per-connection detail this endpoint used to report has no source since tinymemory
    // v1.13.4; the fields are kept on the wire at their zero value rather than removed. Pinning
    // that keeps a future "it started reporting real cursors again" from going unnoticed.
    assert_eq!(
        row.get("per_channel_cursors").and_then(Value::as_str),
        Some("{}"),
        "cursor detail is no longer available and is reported as an empty JSON object: {row}"
    );
    assert_eq!(row.get("synced_ids_count").and_then(Value::as_u64), Some(0));
    assert_eq!(
        row.get("requests_used_today").and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        row.get("daily_request_limit").and_then(Value::as_u64),
        Some(0)
    );

    // ── a trigger scoped to a connection that is not a candidate fails, naming the id.
    let unknown = post_json_rpc(
        &h.rpc_base,
        9602,
        "openhuman.slack_memory_sync_trigger",
        json!({ "connection_id": "conn-slack-pending" }),
    )
    .await;
    let message = jsonrpc_error_message(&unknown, "sync_trigger for a non-active connection");
    assert!(
        message.contains("no active Slack connection with id=conn-slack-pending"),
        "the error must name the id it could not find, got: {message}"
    );

    // The gmail connection is active but the wrong toolkit — same refusal, which is what proves
    // the filter is on toolkit *and* status rather than either alone.
    let wrong_toolkit = post_json_rpc(
        &h.rpc_base,
        9603,
        "openhuman.slack_memory_sync_trigger",
        json!({ "connection_id": "conn-gmail-live" }),
    )
    .await;
    assert!(
        jsonrpc_error_message(&wrong_toolkit, "sync_trigger for a gmail connection")
            .contains("no active Slack connection with id=conn-gmail-live")
    );

    h.stop();
}

/// Both `slack_memory` controllers refuse before the network hop when the user is signed out.
#[tokio::test]
async fn slack_memory_controllers_require_a_backend_session() {
    let _env_lock = platform_e2e_env_lock();
    let h = Harness::start(default_state()).await;
    // Deliberately no sign_in().

    for (id, method) in [
        (9701, "openhuman.slack_memory_sync_status"),
        (9702, "openhuman.slack_memory_sync_trigger"),
    ] {
        let response = post_json_rpc(&h.rpc_base, id, method, json!({})).await;
        let message = jsonrpc_error_message(&response, method);
        assert!(
            message.contains("[slack_ingest] list_connections")
                && message.contains("no backend session token"),
            "{method} must fail at the client factory with an actionable message, got: {message}"
        );
    }

    h.stop();
}

// ── announcements ────────────────────────────────────────────────────────────

/// `announcements_get_latest` passes a backend announcement through verbatim, folds the backend's
/// 404 into `null`, and refuses when signed out.
///
/// The 404 → `null` fold is the whole point of the controller (it exists because the honest error
/// flooded Sentry with an unactionable signal), and it had no test at any level of the stack.
///
/// Covers: `openhuman.announcements_get_latest`.
#[tokio::test]
async fn announcements_get_latest_passes_through_and_folds_404_to_null() {
    let _env_lock = platform_e2e_env_lock();

    // ── signed out: refused locally, before the backend is dialled.
    let h = Harness::start(default_state()).await;
    let signed_out = post_json_rpc(
        &h.rpc_base,
        9801,
        "openhuman.announcements_get_latest",
        json!({}),
    )
    .await;
    assert!(
        jsonrpc_error_message(&signed_out, "announcements_get_latest, signed out")
            .contains("no backend session token"),
        "the controller requires a live session"
    );

    // ── signed in, announcement present: the backend object is surfaced verbatim.
    h.sign_in().await;
    let present = post_json_rpc(
        &h.rpc_base,
        9802,
        "openhuman.announcements_get_latest",
        json!({}),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(&present, "announcements_get_latest"));
    assert_eq!(
        result.get("id").and_then(Value::as_str),
        Some("ann-1"),
        "the backend payload must pass through unwrapped: {result}"
    );
    assert_eq!(
        result.get("title").and_then(Value::as_str),
        Some("Scheduled maintenance")
    );
    assert_eq!(result.get("severity").and_then(Value::as_str), Some("info"));
    h.stop();

    // ── signed in, backend 404: folded into `null`, not surfaced as an error.
    let h = Harness::start(BackendState {
        announcement: AnnouncementMode::NotFound,
        connections: Arc::new(json!({ "connections": [] })),
    })
    .await;
    h.sign_in().await;
    let absent = post_json_rpc(
        &h.rpc_base,
        9803,
        "openhuman.announcements_get_latest",
        json!({}),
    )
    .await;
    let result = peel(assert_no_jsonrpc_error(
        &absent,
        "announcements_get_latest, backend 404",
    ));
    assert!(
        result.is_null(),
        "a 404 from /announcements/latest means \"no announcement\" and must degrade to null \
         rather than propagate as an error: {result}"
    );

    h.stop();
}
