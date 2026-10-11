//! Shared harness for the memory-engine e2e suites: a hosted-CortexDB double,
//! env isolation, and a `Fixture` that boots the JSON-RPC router with the
//! module loaded and a backend API key stored.
//!
//! Include with `#[path = "support/memory_engine_fixture.rs"] mod fixture;`.

#![allow(dead_code, clippy::await_holding_lock)]

#[path = "memory_module.rs"]
pub mod memory_module;
#[path = "tinyhumans_boot.rs"]
pub mod tinyhumans_boot;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_rpc::server::build_core_http_router;

pub const TEST_RPC_TOKEN: &str = "memory-engine-e2e-token";
pub const TEST_API_KEY: &str = "tiny_live_memory_engine_e2e";
pub const NS: &str = "engine-e2e";

// ── Hosted CortexDB double ──────────────────────────────────────────────────

#[derive(Default)]
pub struct HostedState {
    pub events: Mutex<Vec<Value>>,
    pub idempotency: Mutex<std::collections::BTreeMap<String, (String, String)>>,
    pub claims: Mutex<HashSet<String>>,
    pub bearers: Mutex<Vec<String>>,
    pub next_id: Mutex<u64>,
    /// When non-zero every `/memory/*` call fails with this status.
    pub force_status: AtomicU16,
    /// Milliseconds every `experience` write is delayed by (0 = none).
    pub delay_ms: AtomicU64,
    /// What `GET /memory/{facts,beliefs,understanding}` answers, by
    /// `(layer, scope)`.
    pub layers: Mutex<std::collections::HashMap<(String, String), Vec<Value>>>,
}

pub type Hosted = Arc<HostedState>;

pub fn err(status: u16, code: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::from_u16(status).unwrap(),
        Json(json!({ "success": false, "error": format!("failed: {code}"), "errorCode": code })),
    )
}

pub fn ok(data: Value) -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(json!({ "success": true, "data": data })),
    )
}

pub fn gate(state: &Hosted, headers: &HeaderMap) -> Option<(StatusCode, Json<Value>)> {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_string();
    state.bearers.lock().unwrap().push(token.clone());
    if token.is_empty() {
        return Some(err(401, "UNAUTHORIZED"));
    }
    match state.force_status.load(Ordering::SeqCst) {
        0 => None,
        401 => Some(err(401, "UNAUTHORIZED")),
        402 => Some(err(402, "USER_INSUFFICIENT_CREDITS")),
        other => Some(err(other, "UPSTREAM_ERROR")),
    }
}

pub async fn experience(
    State(state): State<Hosted>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if let Some(early) = gate(&state, &headers) {
        return early;
    }
    let delay = state.delay_ms.load(Ordering::SeqCst);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    if let Some(claim) = headers.get("idempotency-key").and_then(|v| v.to_str().ok()) {
        if !state.claims.lock().unwrap().insert(claim.to_string()) {
            return err(409, "CONFLICT");
        }
    }
    let key = body["idempotency_key"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let text = body["content"]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if let Some((seen, id)) = state.idempotency.lock().unwrap().get(&key) {
        return if seen == &text {
            ok(json!({ "event_id": id, "replayed_from_idempotency": true }))
        } else {
            err(409, "IDEMPOTENCY_CONFLICT")
        };
    }
    let id = {
        let mut next = state.next_id.lock().unwrap();
        *next += 1;
        format!("evt_{}", *next)
    };
    let offset = state.events.lock().unwrap().len() as u64 * 2 + 2;
    state
        .idempotency
        .lock()
        .unwrap()
        .insert(key, (text, id.clone()));
    // The engine keeps the caller's context (labels, `observed_at`) and stamps
    // its own `recorded_at`; the hosted families look records up by label.
    let mut context = body["context"].clone();
    if !context.is_object() {
        context = json!({});
    }
    context["recorded_at"] = json!("2026-09-02T00:00:00Z");
    let mut event = json!({
        "id": id,
        "scope": body["scope"],
        "modality": body["modality"],
        "wal_offset": offset,
        "content": body["content"],
        "context": context,
    });
    if !body["directives"].is_null() {
        event["directives"] = body["directives"].clone();
    }
    state.events.lock().unwrap().push(event);
    ok(json!({ "event_id": id, "status": "captured", "replayed_from_idempotency": false }))
}

pub async fn events(
    State(state): State<Hosted>,
    headers: HeaderMap,
    Query(params): Query<std::collections::BTreeMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    if let Some(early) = gate(&state, &headers) {
        return early;
    }
    let scope = params.get("scope").cloned().unwrap_or_default();
    let cursor: usize = params
        .get("cursor")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let limit: usize = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    // The engine splits its label filter on commas and keeps an event carrying
    // any one of the pieces.
    let wanted: Vec<String> = params
        .get("labels")
        .map(|labels| labels.split(',').map(|l| l.trim().to_string()).collect())
        .unwrap_or_default();
    let labelled = |event: &Value| {
        wanted.is_empty()
            || event["context"]["labels"].as_array().is_some_and(|labels| {
                labels
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|label| wanted.iter().any(|w| w == label))
            })
    };
    let mut stream = Vec::new();
    for event in state.events.lock().unwrap().iter().rev() {
        if event["scope"].as_str() == Some(scope.as_str()) && labelled(event) {
            stream.push(event.clone());
            stream.push(event.clone());
        }
    }
    let page: Vec<Value> = stream.iter().skip(cursor).take(limit).cloned().collect();
    let next = cursor + page.len();
    ok(json!({ "items": page, "has_more": next < stream.len(), "next_cursor": next.to_string() }))
}

pub async fn recall(
    State(state): State<Hosted>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if let Some(early) = gate(&state, &headers) {
        return early;
    }
    let scope = body["scope"].as_str().unwrap_or_default();
    let query = body["query"].as_str().unwrap_or_default().to_lowercase();
    // `descend` recalls the scope and everything under it.
    let descend = body["view"].as_str() == Some("descend");
    // A metadata label filter keeps an event carrying any one of the labels.
    let wanted: Vec<&str> = body
        .pointer("/filters/metadata/labels")
        .and_then(Value::as_array)
        .map(|labels| labels.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let budget = body
        .pointer("/budgets/per_layer_limits/events")
        .and_then(Value::as_u64)
        .map_or(usize::MAX, |limit| limit as usize);
    let hits: Vec<Value> = state
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| {
            e["scope"].as_str().is_some_and(|held| {
                held == scope || (descend && held.starts_with(&format!("{scope}/")))
            })
        })
        .filter(|e| {
            wanted.is_empty()
                || e["context"]["labels"].as_array().is_some_and(|labels| {
                    labels
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|label| wanted.contains(&label))
                })
        })
        .filter(|e| {
            query.is_empty()
                || e["content"]["text"]
                    .as_str()
                    .is_some_and(|t| t.to_lowercase().contains(&query))
        })
        .map(|e| {
            let mut hit = e.clone();
            if let Some(text) = e["content"]["text"].as_str() {
                hit["content"]["text"] = json!(format!("[user] {text}"));
            }
            hit
        })
        .take(budget)
        .collect();
    ok(json!({ "pack_id": "pack_test", "layers": { "events": hits } }))
}

pub async fn scopes(
    State(state): State<Hosted>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    if let Some(early) = gate(&state, &headers) {
        return early;
    }
    let limit: usize = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    // A prefix names a scope and everything under it.
    let prefix = params.get("prefix").cloned();
    let mut paths: Vec<String> = state
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| e["scope"].as_str().map(str::to_string))
        .filter(|path| {
            prefix
                .as_deref()
                .is_none_or(|p| path == p || path.starts_with(&format!("{p}/")))
        })
        .collect();
    paths.sort();
    paths.dedup();
    paths.truncate(limit);
    ok(json!({ "items": paths.into_iter().map(|p| json!({ "path": p })).collect::<Vec<_>>() }))
}

pub async fn event_by_id(
    State(state): State<Hosted>,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
) -> (StatusCode, Json<Value>) {
    if let Some(early) = gate(&state, &headers) {
        return early;
    }
    let found = state
        .events
        .lock()
        .unwrap()
        .iter()
        .find(|e| e["id"].as_str() == Some(id.as_str()))
        .cloned();
    match found {
        Some(event) => ok(event),
        None => err(404, "NOT_FOUND"),
    }
}

pub async fn forget(
    State(state): State<Hosted>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if let Some(early) = gate(&state, &headers) {
        return early;
    }
    let ids: Vec<String> = body["selector"]["memory_ids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    // CortexDB's default cascade, `derived_only`, keeps the events.
    if body["cascade"].as_str() != Some("redact_events") {
        return ok(json!({ "deleted": { "events": 0 }, "requested": ids.len(), "matched": 0 }));
    }
    let before = state.events.lock().unwrap().len();
    state
        .events
        .lock()
        .unwrap()
        .retain(|e| !ids.contains(&e["id"].as_str().unwrap_or_default().to_string()));
    let deleted = before - state.events.lock().unwrap().len();
    ok(json!({ "deleted": { "events": deleted }, "requested": ids.len(), "matched": deleted }))
}

/// One page of a derived layer: everything seeded for the scope, in one page.
pub async fn layer(
    State(state): State<Hosted>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Query(params): Query<std::collections::BTreeMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    if let Some(early) = gate(&state, &headers) {
        return early;
    }
    let name = uri
        .path()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string();
    let scope = params.get("scope").cloned().unwrap_or_default();
    let items = state
        .layers
        .lock()
        .unwrap()
        .get(&(name, scope))
        .cloned()
        .unwrap_or_default();
    ok(json!({ "items": items, "has_more": false }))
}

pub async fn start_hosted() -> (String, Hosted) {
    let state: Hosted = Arc::new(HostedState::default());
    let app = Router::new()
        .route("/memory/experience", post(experience))
        .route("/memory/events", get(events))
        .route("/memory/events/{id}", get(event_by_id))
        .route("/memory/recall", post(recall))
        .route("/memory/forget", post(forget))
        .route("/memory/scopes", get(scopes))
        .route("/memory/facts", get(layer))
        .route("/memory/beliefs", get(layer))
        .route("/memory/understanding", get(layer))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    (format!("http://{addr}"), state)
}

// ── Harness ─────────────────────────────────────────────────────────────────

pub struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    pub fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }
    pub fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

pub static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
pub static AUTH_INIT: OnceLock<()> = OnceLock::new();
pub static SEAMS_INIT: OnceLock<()> = OnceLock::new();

pub fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    openhuman_core::security::secret_storage::init_workspace(shared_workspace());
    match ENV_LOCK.get_or_init(|| Mutex::new(())).lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// The one workspace the process's memory module and every case share.
pub fn shared_workspace() -> &'static Path {
    static SHARED: OnceLock<PathBuf> = OnceLock::new();
    SHARED.get_or_init(|| {
        let tmp = tempdir().expect("shared workspace tempdir");
        let path = tmp.path().join("workspace");
        std::fs::create_dir_all(&path).expect("shared workspace dir");
        std::mem::forget(tmp);
        path
    })
}

/// The config file that workspace resolves to (legacy sibling layout).
pub fn shared_config_path() -> PathBuf {
    shared_workspace()
        .parent()
        .expect("workspace parent")
        .join(".openhuman")
        .join("config.toml")
}

pub fn ensure_seams() {
    SEAMS_INIT.get_or_init(|| {
        std::thread::Builder::new()
            .name("memory-engine-e2e-seams".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let config = Arc::new(openhuman_core::config::Config {
                    workspace_dir: shared_workspace().to_path_buf(),
                    ..openhuman_core::config::Config::default()
                });
                #[cfg(feature = "modules")]
                openhuman_core::modules::memory::set_modules_policy(config);
                #[cfg(not(feature = "modules"))]
                let _ = config;
            })
            .expect("spawn seam installer")
            .join()
            .expect("seam installer panicked");
    });
}

pub fn ensure_rpc_auth() {
    AUTH_INIT.get_or_init(|| {
        tinyhumans_boot::boot();
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN) };
        let dir = std::env::temp_dir().join("openhuman-memory-engine-e2e-auth");
        init_rpc_token(&dir).expect("init rpc token");
    });
}

pub fn run_on_big_stack<F, Fut>(name: &str, factory: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
                .enable_all()
                .build()
                .expect("runtime");
            rt.block_on(factory());
        })
        .expect("spawn")
        .join()
        .expect("test body panicked");
}

pub async fn rpc(base: &str, method: &str, params: Value) -> Value {
    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap()
        .post(format!("{base}/rpc"))
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {method}: {e}"));
    assert!(
        resp.status().is_success(),
        "HTTP {} for {method}",
        resp.status()
    );
    resp.json().await.expect("json body")
}

pub fn result_of<'a>(v: &'a Value, ctx: &str) -> &'a Value {
    if let Some(e) = v.get("error") {
        panic!("{ctx}: JSON-RPC error: {e}");
    }
    let r = v
        .get("result")
        .unwrap_or_else(|| panic!("{ctx}: no result: {v}"));
    if r.get("logs").is_some() {
        r.get("result").unwrap_or(r)
    } else {
        r
    }
}

pub fn error_message(v: &Value, ctx: &str) -> String {
    v.pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{ctx}: expected a JSON-RPC error, got {v}"))
        .to_string()
}

/// Everything a case needs: the hosted double, the RPC base URL and the env.
pub struct Fixture {
    pub _lock: std::sync::MutexGuard<'static, ()>,
    pub _guards: Vec<EnvVarGuard>,
    pub _tmp: tempfile::TempDir,
    pub hosted: Hosted,
    pub origin: String,
    pub base: String,
    pub join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl Fixture {
    pub async fn new() -> Self {
        let lock = env_lock();
        let tmp = tempdir().expect("home tempdir");
        let guards = vec![
            EnvVarGuard::set_to_path("HOME", tmp.path()),
            EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", shared_workspace()),
            EnvVarGuard::unset("OPENHUMAN_MEMORY_DRIVER"),
            EnvVarGuard::unset("BACKEND_URL"),
            EnvVarGuard::unset("VITE_BACKEND_URL"),
            EnvVarGuard::unset("OPENHUMAN_BACKEND_API_KEY"),
        ];
        ensure_rpc_auth();
        ensure_seams();
        let (origin, hosted) = start_hosted().await;

        // A fresh config every case: the previous case's committed engine must
        // not leak into this one.
        let config_path = shared_config_path();
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(
            &config_path,
            format!(
                "api_url = \"{origin}\"\ndefault_model = \"e2e-mock-model\"\n\n[secrets]\nencrypt = false\n"
            ),
        )
        .unwrap();

        memory_module::settle().await;
        let config = openhuman_core::config::load_config_with_timeout()
            .await
            .expect("load config");
        openhuman_core::security::credentials::api_key::store_api_key(&config, TEST_API_KEY)
            .expect("store the backend api key");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = build_core_http_router(false);
        let join = tokio::spawn(async move { axum::serve(listener, app).await });
        Self {
            _lock: lock,
            _guards: guards,
            _tmp: tmp,
            hosted,
            origin,
            base: format!("http://{addr}"),
            join,
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> Value {
        rpc(&self.base, method, params).await
    }

    pub async fn state(&self) -> Value {
        let v = self.call("openhuman.memory_engine_get", json!({})).await;
        result_of(&v, "engine_get").clone()
    }

    /// Poll a migration job to a terminal state.
    pub async fn wait_job(&self, job_id: &str) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let v = self
                .call(
                    "openhuman.memory_engine_migrate_status",
                    json!({ "job_id": job_id }),
                )
                .await;
            let status = result_of(&v, "engine_migrate_status").clone();
            if status["state"] != "running" {
                return status;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "migration job {job_id} did not finish within 60 seconds"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn put_doc(&self, key: &str, content: &str) {
        let v = self
            .call(
                "openhuman.memory_doc_put",
                json!({
                    "namespace": NS, "key": key, "title": key, "content": content,
                    "source_type": "doc", "priority": "medium", "tags": [],
                    "metadata": null, "category": "core"
                }),
            )
            .await;
        result_of(&v, "doc_put");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.join.abort();
    }
}
