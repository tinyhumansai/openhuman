//! Memory v2 over JSON-RPC, end to end against the shared mock backend.
//!
//! Boots the core router in-process, points it at `scripts/mock-api-server.mjs`
//! (the in-memory TinyHumans `/memory/*` CortexDB proxy in
//! `scripts/mock-api/routes/memory.mjs`), signs in as the mock user and drives
//! every `openhuman.memory_*` method in `docs/specs/memory-v2.md`:
//!
//! - engines: list / get / set, the off state, and the structured error codes;
//! - learn -> items_list -> fetch -> recall -> forget on the hosted engine;
//! - erase_all: the whole hosted memory in one `DELETE /memory`;
//! - conversations settings;
//! - sources: add a folder, sync it, read its items back, remove it;
//! - context.md: refresh / get / set;
//! - the explorer: explore by facet under a path, list by path, get by id;
//! - the v1 import gate (scan finds nothing, start needs consent);
//! - the retired v1 methods no longer dispatch.
//!
//! No real network: the only peer is the loopback node mock. Needs `node` on
//! `PATH` (the same requirement as `pnpm test:rust`).

#[path = "support/env_guard.rs"]
mod env_guard;
#[path = "support/rpc_auth.rs"]
mod rpc_auth;
#[path = "support/rpc_harness.rs"]
mod rpc_harness;
#[path = "support/tinyhumans_boot.rs"]
mod tinyhumans_boot;

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use env_guard::{env_lock_async, EnvVarGuard};
use rpc_harness::{rpc, serve_rpc};
use serde_json::{json, Value};

/// A JWT-shaped bearer the mock accepts; the mock isolates memory per bearer.
const MOCK_TOKEN: &str =
    "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJzdWIiOiJ1c2VyLTEyMyIsImV4cCI6NDEwMjQ0NDgwMH0.e2e";
const MOCK_USER_ID: &str = "memory-v2-e2e-user";

static NEXT_ID: AtomicI64 = AtomicI64::new(1);

/// The node mock backend, killed when dropped.
struct MockBackend {
    child: Child,
    origin: String,
}

impl MockBackend {
    async fn start() -> Self {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("reserve a port");
            listener.local_addr().expect("local addr").port()
        };
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/mock-api-server.mjs")
            .canonicalize()
            .expect("scripts/mock-api-server.mjs exists");
        let child = Command::new("node")
            .arg(&script)
            .arg("--port")
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn `node scripts/mock-api-server.mjs` (is node on PATH?)");
        let origin = format!("http://127.0.0.1:{port}");
        let mock = Self { child, origin };
        let client = reqwest::Client::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let up = client
                .get(format!("{}/__admin/health", mock.origin))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false);
            if up {
                return mock;
            }
            assert!(
                Instant::now() < deadline,
                "the mock backend did not become healthy at {}",
                mock.origin
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Sets a mock behaviour switch (e.g. `memoryForceStatus`).
    async fn set_behavior(&self, key: &str, value: &str) {
        let response = reqwest::Client::new()
            .post(format!("{}/__admin/behavior", self.origin))
            .json(&json!({ "key": key, "value": value }))
            .send()
            .await
            .expect("set mock behaviour");
        assert!(response.status().is_success(), "mock behaviour accepted");
    }

    /// Every request the mock logged, whole (url, method, headers, body).
    async fn request_rows(&self) -> Vec<Value> {
        let body: Value = reqwest::get(format!("{}/__admin/requests", self.origin))
            .await
            .expect("read the mock request log")
            .json()
            .await
            .expect("request log json");
        body.get("data")
            .or(Some(&body))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    /// Every request the mock logged, as `(url, body)` pairs.
    async fn request_bodies(&self) -> Vec<(String, String)> {
        let body: Value = reqwest::get(format!("{}/__admin/requests", self.origin))
            .await
            .expect("read the mock request log")
            .json()
            .await
            .expect("request log json");
        body.get("data")
            .or(Some(&body))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|row| {
                (
                    row.get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    row.get("body")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                )
            })
            .collect()
    }

    /// Every request the mock logged, as `"METHOD /path"` strings.
    async fn request_paths(&self) -> Vec<String> {
        let body: Value = reqwest::get(format!("{}/__admin/requests", self.origin))
            .await
            .expect("read the mock request log")
            .json()
            .await
            .expect("request log json");
        let rows = body
            .get("data")
            .or(Some(&body))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        rows.iter()
            .map(|row| {
                format!(
                    "{} {}",
                    row.get("method")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    row.get("url").and_then(Value::as_str).unwrap_or_default()
                )
            })
            .collect()
    }
}

impl Drop for MockBackend {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One isolated core + mock backend.
struct Fixture {
    rpc_base: String,
    mock: MockBackend,
    home: tempfile::TempDir,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
    _guards: Vec<EnvVarGuard>,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    /// Boots the stack; `signed_in` stores the mock session first.
    async fn new(signed_in: bool) -> Self {
        let lock = env_lock_async().await;
        tinyhumans_boot::boot();
        let mock = MockBackend::start().await;
        let home = tempfile::tempdir().expect("tempdir");
        let guards = vec![
            EnvVarGuard::set_to_path("HOME", home.path()),
            EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
            EnvVarGuard::unset("BACKEND_URL"),
            EnvVarGuard::unset("VITE_BACKEND_URL"),
            EnvVarGuard::unset("OPENHUMAN_API_URL"),
            EnvVarGuard::unset("OPENHUMAN_BACKEND_API_KEY"),
            EnvVarGuard::unset("OPENHUMAN_BACKEND_SESSION_TOKEN"),
            EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        ];
        write_config(&home.path().join(".openhuman"), &mock.origin);
        // Signing in moves the core to `users/<id>/`; every user dir a test
        // signs into must point at the mock too, or the signed-in core falls
        // back to the default (real) backend origin.
        for user in ["local", MOCK_USER_ID, "second-user"] {
            write_config(
                &home.path().join(".openhuman").join("users").join(user),
                &mock.origin,
            );
        }
        let (addr, join) = serve_rpc().await;
        let fixture = Self {
            rpc_base: format!("http://{addr}"),
            mock,
            home,
            join,
            _guards: guards,
            _lock: lock,
        };
        if signed_in {
            fixture.sign_in().await;
        }
        fixture
    }

    async fn sign_in(&self) {
        let response = self
            .call(
                "openhuman.auth_store_session",
                json!({ "token": MOCK_TOKEN, "user_id": MOCK_USER_ID }),
            )
            .await;
        assert!(
            response.get("error").is_none(),
            "auth_store_session failed: {response}"
        );
    }

    /// The raw JSON-RPC envelope of `method`.
    async fn call(&self, method: &str, params: Value) -> Value {
        rpc(
            &self.rpc_base,
            NEXT_ID.fetch_add(1, Ordering::SeqCst),
            method,
            params,
        )
        .await
    }

    /// [`Self::ok`] repeated until `done` holds for the result, or ten
    /// seconds pass, when the last result is returned: for reads of a write
    /// the engine may index after it answers. A call still unanswered at the
    /// deadline panics.
    async fn ok_until(&self, method: &str, params: Value, done: impl Fn(&Value) -> bool) -> Value {
        self.ok_until_with_timeout(method, params, Duration::from_secs(10), done)
            .await
    }

    async fn ok_until_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        done: impl Fn(&Value) -> bool,
    ) -> Value {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            // The deadline bounds each call too, so a hung call cannot
            // outlast it.
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let result = tokio::time::timeout(left, self.ok(method, params.clone()))
                .await
                .unwrap_or_else(|_| panic!("{method}: no answer within the polling deadline"));
            if done(&result) || std::time::Instant::now() >= deadline {
                return result;
            }
            // Never start a call the deadline leaves no time for: the sleep
            // may cross it, and the last result is then the answer.
            let pause = std::time::Duration::from_millis(100);
            if std::time::Instant::now() + pause >= deadline {
                return result;
            }
            tokio::time::sleep(pause).await;
            // A late wake can still cross it.
            if std::time::Instant::now() >= deadline {
                return result;
            }
        }
    }

    /// The unwrapped result of a call that must succeed.
    async fn ok(&self, method: &str, params: Value) -> Value {
        let response = self.call(method, params).await;
        if let Some(error) = response.get("error") {
            panic!("{method}: unexpected JSON-RPC error: {error}");
        }
        let result = response
            .get("result")
            .unwrap_or_else(|| panic!("{method}: missing result: {response}"))
            .clone();
        // Memory handlers return a bare object; tolerate a `{result, logs}` envelope.
        match result.get("result") {
            Some(inner) if result.get("logs").is_some() => inner.clone(),
            _ => result,
        }
    }

    /// The `data.code` of a call that must fail with a memory error.
    async fn code(&self, method: &str, params: Value) -> String {
        let response = self.call(method, params).await;
        let error = response
            .get("error")
            .unwrap_or_else(|| panic!("{method}: expected an error, got {response}"));
        if let Some(code) = error.pointer("/data/code").and_then(Value::as_str) {
            return code.to_string();
        }
        // The core's schema validation rejects a missing, mistyped or
        // out-of-range param before the memory handler runs, with its own
        // standard message and no `data.code`. That is an invalid request.
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let schema_rejection = [
            "missing required param",
            "invalid type for param",
            "unknown param",
        ]
        .iter()
        .any(|prefix| message.starts_with(prefix));
        assert!(
            schema_rejection,
            "{method}: error has no data.code: {error}"
        );
        "INVALID_REQUEST".to_string()
    }

    async fn learn(&self, text: &str) -> String {
        let learned = self
            .ok(
                "openhuman.memory_learn",
                json!({ "text": text, "kind": "fact", "confidence": 0.9 }),
            )
            .await;
        learned["id"]
            .as_str()
            .expect("learn returns an id")
            .to_string()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.join.abort();
        let _ = self.home.path();
    }
}

fn write_config(dir: &Path, api_origin: &str) {
    std::fs::create_dir_all(dir).expect("mkdir config dir");
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "e2e-mock-model"
default_temperature = 0.2
chat_onboarding_completed = true

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory]
engine = "tinyhumans"
"#
    );
    let _: openhuman_core::config::Config =
        toml::from_str(&cfg).expect("config toml must match the Config schema");
    std::fs::write(dir.join("config.toml"), cfg).expect("write config.toml");
}

fn ids_of(items: &Value, key: &str) -> Vec<String> {
    items[key]
        .as_array()
        .unwrap_or_else(|| panic!("`{key}` is an array: {items}"))
        .iter()
        .filter_map(|item| item["id"].as_str().map(str::to_string))
        .collect()
}

// ---------------------------------------------------------------------------
// Off state: signed out, no CortexDB key
// ---------------------------------------------------------------------------

#[tokio::test]
async fn memory_is_off_when_signed_out() {
    let f = Fixture::new(false).await;

    let engines = f.ok("openhuman.memory_engines_list", json!({})).await;
    assert!(engines["active"].is_null(), "no active engine: {engines}");
    let listed: Vec<&str> = engines["engines"]
        .as_array()
        .expect("engines array")
        .iter()
        .filter_map(|e| e["id"].as_str())
        .collect();
    assert!(
        listed.contains(&"tinyhumans") && listed.contains(&"cortexdb"),
        "{listed:?}"
    );
    let tinyhumans = engines["engines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "tinyhumans")
        .unwrap();
    assert_eq!(tinyhumans["hosted"], json!(true));
    assert_eq!(tinyhumans["fetch_modes"], json!(["hybrid"]));

    let engine = f.ok("openhuman.memory_engine_get", json!({})).await;
    assert_eq!(engine["engine"], json!("tinyhumans"));
    assert_eq!(engine["status"], json!("off"));
    assert_eq!(engine["has_key"], json!(false));
    assert_eq!(engine["fetch_modes"], json!([]));
    assert!(
        engine["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("sign in"),
        "the reason explains why memory is off: {engine}"
    );

    // Everything that needs an engine answers MEMORY_OFF.
    let off_calls = [
        ("openhuman.memory_recall", json!({ "question": "anything" })),
        ("openhuman.memory_fetch", json!({ "query": "anything" })),
        ("openhuman.memory_learn", json!({ "text": "a fact" })),
        ("openhuman.memory_forget", json!({ "ids": ["x"] })),
        ("openhuman.memory_items_list", json!({})),
        ("openhuman.memory_explore", json!({ "facet": "kind" })),
        ("openhuman.memory_items_get", json!({ "ids": ["x"] })),
        (
            "openhuman.memory_conversations_backfill_start",
            json!({ "consent": true }),
        ),
        ("openhuman.memory_sources_sync", json!({})),
        (
            "openhuman.memory_pack_preview",
            json!({ "query": "anything" }),
        ),
        ("openhuman.memory_agents_list", json!({})),
        ("openhuman.memory_brain_sources", json!({})),
        ("openhuman.memory_brain_ingest", json!({ "text": "a doc" })),
        ("openhuman.memory_jobs_run", json!({})),
        ("openhuman.memory_import_start", json!({ "consent": true })),
        ("openhuman.memory_erase_all", json!({ "confirm": true })),
    ];
    for (method, params) in off_calls {
        assert_eq!(f.code(method, params).await, "MEMORY_OFF", "{method}");
    }

    // Local settings and state still answer while memory is off.
    let policy = f.ok("openhuman.memory_policy_get", json!({})).await;
    assert_eq!(policy["log_conversations"], json!(true));
    assert_eq!(
        f.ok("openhuman.memory_sources_list", json!({})).await["sources"],
        json!([])
    );
    assert_eq!(
        f.ok("openhuman.memory_jobs_list", json!({})).await["pending"],
        json!([])
    );
    assert_eq!(
        f.ok("openhuman.memory_import_scan", json!({})).await["found"],
        json!(false)
    );
    assert_eq!(
        f.ok("openhuman.memory_import_status", json!({})).await["state"]["phase"],
        json!("idle")
    );

    // Nothing reached the backend's memory proxy.
    let paths = f.mock.request_paths().await;
    assert!(
        !paths.iter().any(|p| p.contains("/memory/")),
        "signed-out memory must not call the backend: {paths:?}"
    );
}

#[tokio::test]
async fn signing_in_turns_memory_on() {
    let f = Fixture::new(false).await;
    assert_eq!(
        f.code(
            "openhuman.memory_learn",
            json!({ "text": "before sign in" })
        )
        .await,
        "MEMORY_OFF"
    );
    f.sign_in().await;
    let engines = f.ok("openhuman.memory_engines_list", json!({})).await;
    assert_eq!(engines["active"], json!("tinyhumans"));
    let id = f.learn("after sign in").await;
    assert!(!id.is_empty());
}

// ---------------------------------------------------------------------------
// Engines
// ---------------------------------------------------------------------------

#[tokio::test]
async fn engines_list_get_and_set() {
    let f = Fixture::new(true).await;

    let engines = f.ok("openhuman.memory_engines_list", json!({})).await;
    assert_eq!(engines["active"], json!("tinyhumans"));
    for engine in engines["engines"].as_array().expect("engines") {
        for field in [
            "id",
            "label",
            "description",
            "hosted",
            "needs_endpoint",
            "needs_key",
            "fetch_modes",
        ] {
            assert!(
                engine.get(field).is_some(),
                "descriptor has `{field}`: {engine}"
            );
        }
    }

    let engine = f.ok("openhuman.memory_engine_get", json!({})).await;
    assert_eq!(engine["engine"], json!("tinyhumans"));
    assert_eq!(engine["status"], json!("ok"), "{engine}");
    assert_eq!(engine["has_key"], json!(true));
    assert_eq!(engine["fetch_modes"], json!(["hybrid"]));
    assert_eq!(engine["endpoint"], json!(f.mock.origin));

    // The health probe reached the hosted proxy with the session bearer.
    let paths = f.mock.request_paths().await;
    assert!(
        paths.iter().any(|p| p.starts_with("GET /memory/scopes")),
        "engine_get probes /memory/scopes: {paths:?}"
    );

    // Refused selections carry INVALID_REQUEST.
    for (params, why) in [
        (json!({ "engine": "nope" }), "unknown engine"),
        (
            json!({ "engine": "tinyhumans", "api_key": "k" }),
            "tinyhumans takes no key",
        ),
        (
            json!({ "engine": "cortexdb", "endpoint": "not a url" }),
            "malformed endpoint",
        ),
        (
            json!({ "engine": "cortexdb", "endpoint": "ftp://example.com" }),
            "non-http endpoint",
        ),
    ] {
        assert_eq!(
            f.code("openhuman.memory_engine_set", params).await,
            "INVALID_REQUEST",
            "{why}"
        );
    }
    // A refused set changed nothing.
    assert_eq!(
        f.ok("openhuman.memory_engine_get", json!({})).await["engine"],
        json!("tinyhumans")
    );

    // Selecting CortexDB without a key is accepted but leaves memory off, and
    // the view says why.
    let selected = f
        .ok(
            "openhuman.memory_engine_set",
            json!({ "engine": "cortexdb" }),
        )
        .await;
    assert_eq!(selected["engine"], json!("cortexdb"));
    assert_eq!(selected["status"], json!("off"));
    assert_eq!(selected["has_key"], json!(false));
    assert!(
        selected["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("API key"),
        "{selected}"
    );
    assert_eq!(
        f.code("openhuman.memory_learn", json!({ "text": "no key yet" }))
            .await,
        "MEMORY_OFF"
    );
    assert!(f.ok("openhuman.memory_engines_list", json!({})).await["active"].is_null());

    // A key (kept in the credential store, never echoed) makes it a candidate;
    // the mock only speaks the hosted wire, so the direct engine is not `ok`.
    let keyed = f
        .ok(
            "openhuman.memory_engine_set",
            json!({ "engine": "cortexdb", "endpoint": f.mock.origin, "api_key": "ctx-test-key" }),
        )
        .await;
    assert_eq!(keyed["engine"], json!("cortexdb"));
    assert_eq!(keyed["has_key"], json!(true));
    assert_eq!(keyed["endpoint"], json!(f.mock.origin));
    assert_ne!(
        keyed["status"],
        json!("ok"),
        "no /v1 surface on the mock: {keyed}"
    );
    assert!(
        !keyed.to_string().contains("ctx-test-key"),
        "the key is never echoed: {keyed}"
    );

    // Clearing the key (empty string) and going back to the hosted engine.
    let cleared = f
        .ok(
            "openhuman.memory_engine_set",
            json!({ "engine": "cortexdb", "api_key": "" }),
        )
        .await;
    assert_eq!(cleared["has_key"], json!(false));
    let back = f
        .ok(
            "openhuman.memory_engine_set",
            json!({ "engine": "tinyhumans" }),
        )
        .await;
    assert_eq!(back["status"], json!("ok"), "{back}");
    assert_eq!(
        f.ok("openhuman.memory_engines_list", json!({})).await["active"],
        json!("tinyhumans")
    );
}

#[tokio::test]
async fn engine_failures_surface_as_structured_errors() {
    let f = Fixture::new(true).await;

    f.mock.set_behavior("memoryForceStatus", "401").await;
    assert_eq!(
        f.code("openhuman.memory_learn", json!({ "text": "rejected" }))
            .await,
        "UNAUTHORIZED"
    );

    f.mock.set_behavior("memoryForceStatus", "503").await;
    let down = f.ok("openhuman.memory_engine_get", json!({})).await;
    assert_ne!(
        down["status"],
        json!("ok"),
        "an unavailable engine is not ok: {down}"
    );

    f.mock.set_behavior("memoryForceStatus", "").await;
    assert_eq!(
        f.ok("openhuman.memory_engine_get", json!({})).await["status"],
        json!("ok")
    );
}

// ---------------------------------------------------------------------------
// learn -> items_list -> fetch -> recall -> forget
// ---------------------------------------------------------------------------

#[tokio::test]
async fn learn_list_fetch_recall_and_forget_round_trip() {
    let f = Fixture::new(true).await;

    // Empty to begin with.
    let empty = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["learning"] } }),
        )
        .await;
    assert_eq!(empty["items"], json!([]));

    // learn: validated, then stored as a learning.
    assert_eq!(
        f.code(
            "openhuman.memory_learn",
            json!({ "text": "x", "confidence": 2.0 })
        )
        .await,
        "INVALID_REQUEST"
    );
    assert_eq!(
        f.code("openhuman.memory_learn", json!({ "text": "   " }))
            .await,
        "INVALID_REQUEST"
    );
    let coffee = f
        .learn("Alice prefers dark roast coffee in the morning")
        .await;
    let tea = f.learn("Bob drinks green tea after lunch").await;
    assert_ne!(coffee, tea);
    // Identical content resolves to the same id: a replay, not a duplicate.
    assert_eq!(
        f.learn("Alice prefers dark roast coffee in the morning")
            .await,
        coffee
    );

    // items_list: kind filter + shape.
    let listed = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["learning"] } }),
        )
        .await;
    let mut ids = ids_of(&listed, "items");
    ids.sort();
    let mut expected = vec![coffee.clone(), tea.clone()];
    expected.sort();
    assert_eq!(ids, expected, "{listed}");
    let item = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == json!(coffee))
        .unwrap();
    assert_eq!(item["kind"], json!("learning"));
    assert!(item["text"].as_str().unwrap().contains("dark roast coffee"));
    assert!(item["meta"].is_object());
    // No documents or conversations were stored.
    let docs = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["document"] } }),
        )
        .await;
    assert_eq!(docs["items"], json!([]));
    // Pagination: a page of one.
    let page = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["learning"] }, "limit": 1 }),
        )
        .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert!(
        page["next_cursor"].is_string(),
        "a second page exists: {page}"
    );

    // fetch: hybrid is the one declared mode; the others are UNSUPPORTED.
    let fetched = f
        .ok(
            "openhuman.memory_fetch",
            json!({ "query": "coffee", "mode": "hybrid", "filter": { "kinds": ["learning"] } }),
        )
        .await;
    assert_eq!(ids_of(&fetched, "hits"), vec![coffee.clone()], "{fetched}");
    let hit = &fetched["hits"][0];
    assert_eq!(hit["kind"], json!("learning"));
    assert!(hit["score"].is_number());
    assert!(hit["text"].as_str().unwrap().contains("coffee"));
    // An unset mode uses the engine's first declared mode.
    let default_mode = f
        .ok("openhuman.memory_fetch", json!({ "query": "tea" }))
        .await;
    assert_eq!(ids_of(&default_mode, "hits"), vec![tea.clone()]);
    assert_eq!(
        f.code(
            "openhuman.memory_fetch",
            json!({ "query": "coffee", "mode": "keyword" })
        )
        .await,
        "UNSUPPORTED"
    );
    assert_eq!(
        f.code(
            "openhuman.memory_fetch",
            json!({ "query": "coffee", "mode": "vector" })
        )
        .await,
        "UNSUPPORTED"
    );
    let nothing = f
        .ok("openhuman.memory_fetch", json!({ "query": "submarine" }))
        .await;
    assert_eq!(nothing["hits"], json!([]));

    // recall: an answer with citations drawn from the stored items.
    let recalled = f
        .ok(
            "openhuman.memory_recall",
            json!({ "question": "What coffee does Alice prefer?", "limit": 5 }),
        )
        .await;
    assert!(
        recalled["answer"]
            .as_str()
            .unwrap_or_default()
            .contains("grounded answer"),
        "{recalled}"
    );
    assert!(
        recalled["answer"].as_str().unwrap().contains("dark roast"),
        "the answer rests on the matching learning: {recalled}"
    );
    let cited = ids_of(&recalled, "citations");
    assert!(
        cited.contains(&coffee),
        "citations include the matching item: {recalled}"
    );
    let citation = &recalled["citations"][0];
    assert!(citation["snippet"].is_string() && citation.get("text").is_none());
    assert_eq!(
        f.code("openhuman.memory_recall", json!({ "question": "" }))
            .await,
        "INVALID_REQUEST"
    );

    // The hosted proxy saw the whole flow with the session bearer.
    let paths = f.mock.request_paths().await;
    for needle in [
        "POST /memory/experience",
        "GET /memory/events",
        "POST /memory/recall",
        "POST /memory/answer",
    ] {
        assert!(
            paths.iter().any(|p| p.starts_with(needle)),
            "expected a {needle} call: {paths:?}"
        );
    }

    // forget: by id; counts items; the item is gone everywhere.
    assert_eq!(
        f.code("openhuman.memory_forget", json!({ "ids": [] }))
            .await,
        "INVALID_REQUEST"
    );
    let forgotten = f
        .ok(
            "openhuman.memory_forget",
            json!({ "ids": [coffee.clone()] }),
        )
        .await;
    assert_eq!(forgotten["forgotten"], json!(1));
    let after = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["learning"] } }),
        )
        .await;
    assert_eq!(ids_of(&after, "items"), vec![tea.clone()]);
    let refetch = f
        .ok("openhuman.memory_fetch", json!({ "query": "coffee" }))
        .await;
    assert_eq!(refetch["hits"], json!([]));
    // Forgetting an unknown id forgets nothing.
    let none = f
        .ok(
            "openhuman.memory_forget",
            json!({ "ids": ["no-such-item"] }),
        )
        .await;
    assert_eq!(none["forgotten"], json!(0));
}

/// The `/memory/*` requests the mock logged after the first `skip`, as
/// `METHOD path` lines.
async fn memory_requests_since(f: &Fixture, skip: usize) -> Vec<String> {
    f.mock
        .request_rows()
        .await
        .into_iter()
        .skip(skip)
        .filter_map(|row| {
            let url = row["url"].as_str()?.to_string();
            url.starts_with("/memory/").then(|| {
                let method = row["method"].as_str().unwrap_or("?");
                format!("{method} {}", url.split('?').next().unwrap_or(&url))
            })
        })
        .collect()
}

/// The memory calls made after the one experience write in `calls`.
fn after_the_write(calls: &[String]) -> &[String] {
    let writes: Vec<usize> = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| call.as_str() == "POST /memory/experience")
        .map(|(index, _)| index)
        .collect();
    assert_eq!(writes.len(), 1, "exactly one experience write: {calls:?}");
    &calls[writes[0] + 1..]
}

#[tokio::test]
async fn the_agent_learn_returns_on_accept_and_recall_finds_it() {
    use std::sync::Arc;
    use tinytools::Tool;

    let f = Fixture::new(true).await;
    let config = openhuman_core::config::load_config_with_timeout()
        .await
        .expect("the signed-in config");

    // Control: `memory_learn` over RPC returns once the write is readable, so
    // the hosted engine reads the store back after its write.
    let before = f.mock.request_rows().await.len();
    f.learn("Carol keeps her standup notes in a paper notebook")
        .await;
    let rpc_calls = memory_requests_since(&f, before).await;
    assert!(
        after_the_write(&rpc_calls)
            .iter()
            .any(|call| call == "GET /memory/events"),
        "the RPC learn must read its write back before returning: {rpc_calls:?}"
    );

    // The agent's tool returns on accept: nothing follows its one write (the
    // read before it is the engine's replay lookup), and the model is told
    // recall may lag.
    openhuman_core::core::bus::init().await.expect("bus init");
    let mut events = openhuman_core::core::bus::BUS
        .get()
        .expect("bus initialized")
        .receiver();
    let before = f.mock.request_rows().await.len();
    let tool = openhuman_core::memory::MemoryTool::new(Arc::new(config.clone()));
    let learned = tool
        .execute(json!({
            "action": "learn",
            "text": "Dave takes his coffee with oat milk"
        }))
        .await
        .expect("memory tool execution");
    assert!(!learned.is_error, "{}", learned.text());
    let view: Value = serde_json::from_str(&learned.text()).expect("learn result json");
    let id = view["id"].as_str().expect("learn keeps the receipt id");
    assert_eq!(view["status"], json!("queued; not yet saved to memory"));

    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match events.recv().await {
                Some(openhuman_core::core::events::DomainEvent::MemoryStored { key, .. })
                    if key == id =>
                {
                    break;
                }
                Some(_) => continue,
                None => panic!("memory event bus closed before the queued write completed"),
            }
        }
    })
    .await
    .expect("queued store completed and published MemoryStored");

    let tool_calls = memory_requests_since(&f, before).await;
    assert!(
        after_the_write(&tool_calls).is_empty(),
        "the agent learn must return right after its write, with no visibility reads: {tool_calls:?}"
    );

    // Shortly after, the learning is readable like any other.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let listed = f
            .ok(
                "openhuman.memory_items_list",
                json!({ "filter": { "kinds": ["learning"] } }),
            )
            .await;
        if ids_of(&listed, "items").iter().any(|known| known == id) {
            break;
        }
        assert!(Instant::now() < deadline, "never listed: {listed}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
async fn erase_all_needs_confirmation_and_erases_the_whole_hosted_memory() {
    let f = Fixture::new(true).await;
    f.learn("a fact about tea").await;
    f.learn("a fact about coffee").await;
    // A second hosted-memory category: a document, so a learnings-only
    // erase cannot pass this test.
    f.ok(
        "openhuman.memory_brain_ingest",
        json!({ "text": "a document about espresso machines" }),
    )
    .await;
    let docs_before = f
        .ok_until(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["document"] } }),
            |v| !ids_of(v, "items").is_empty(),
        )
        .await;
    assert!(
        !ids_of(&docs_before, "items").is_empty(),
        "the document was stored: {docs_before}"
    );
    let before = f
        .ok_until(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["learning"] } }),
            |v| ids_of(v, "items").len() == 2,
        )
        .await;
    assert_eq!(ids_of(&before, "items").len(), 2, "{before}");

    // Without its interlock nothing is sent and nothing is erased.
    let skip = f.mock.request_rows().await.len();
    assert_eq!(
        f.code("openhuman.memory_erase_all", json!({})).await,
        "INVALID_REQUEST"
    );
    assert_eq!(
        f.code("openhuman.memory_erase_all", json!({ "confirm": false }))
            .await,
        "INVALID_REQUEST"
    );
    let deletes = |paths: Vec<String>| {
        paths
            .into_iter()
            .filter(|p| p.starts_with("DELETE /memory"))
            .count()
    };
    assert_eq!(deletes(f.mock.request_paths().await[skip..].to_vec()), 0);

    let erased = f
        .ok("openhuman.memory_erase_all", json!({ "confirm": true }))
        .await;
    assert!(
        erased["erased_scopes"].as_u64().is_some_and(|n| n >= 1),
        "{erased}"
    );
    assert_eq!(deletes(f.mock.request_paths().await[skip..].to_vec()), 1);
    let after = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["learning"] } }),
        )
        .await;
    assert_eq!(after["items"], json!([]), "{after}");
    // Another account's memory is untouched, and a repeated erase is a no-op.
    let second = f
        .call(
            "openhuman.auth_store_session",
            json!({ "token": format!("{MOCK_TOKEN}-second"), "user_id": "second-user" }),
        )
        .await;
    assert!(second.get("error").is_none(), "{second}");
    f.learn("a fact that belongs to the second account").await;
    f.sign_in().await;
    f.ok("openhuman.memory_erase_all", json!({ "confirm": true }))
        .await;
    let again = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["learning"] } }),
        )
        .await;
    assert_eq!(again["items"], json!([]), "{again}");
    let second = f
        .call(
            "openhuman.auth_store_session",
            json!({ "token": format!("{MOCK_TOKEN}-second"), "user_id": "second-user" }),
        )
        .await;
    assert!(second.get("error").is_none(), "{second}");
    let kept = f
        .ok_until(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["learning"] } }),
            |v| !ids_of(v, "items").is_empty(),
        )
        .await;
    assert_eq!(ids_of(&kept, "items").len(), 1, "{kept}");
    f.sign_in().await;
    let docs_after = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["document"] } }),
        )
        .await;
    assert_eq!(docs_after["items"], json!([]), "{docs_after}");
    let refetch = f
        .ok("openhuman.memory_fetch", json!({ "query": "coffee" }))
        .await;
    assert_eq!(refetch["hits"], json!([]));
}

#[tokio::test]
async fn memory_is_isolated_per_account() {
    let f = Fixture::new(true).await;
    let id = f.learn("a private fact about account one").await;
    assert!(!id.is_empty());

    // A second session (another bearer) sees an empty store.
    let response = f
        .call(
            "openhuman.auth_store_session",
            json!({ "token": format!("{MOCK_TOKEN}-second"), "user_id": "second-user" }),
        )
        .await;
    assert!(response.get("error").is_none(), "{response}");
    let other = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["learning"] } }),
        )
        .await;
    assert_eq!(other["items"], json!([]), "{other}");
}

// ---------------------------------------------------------------------------
// Conversations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn policy_get_and_set_round_trip() {
    let f = Fixture::new(true).await;

    let defaults = f.ok("openhuman.memory_policy_get", json!({})).await;
    assert_eq!(defaults["log_conversations"], json!(true));
    assert_eq!(defaults["recall"]["enabled"], json!(true));
    assert_eq!(defaults["recall"]["budget_tokens"], json!(1200));
    // Team recall defaults to three entries; setting zero below disables it.
    assert_eq!(defaults["recall"]["team_limit"], json!(3));
    assert_eq!(defaults["root"], json!("root"));
    assert_eq!(defaults["host_bound"], json!(false));

    let updated = f
        .ok(
            "openhuman.memory_policy_set",
            json!({ "log_conversations": false, "budget_tokens": 600, "team_limit": 0 }),
        )
        .await;
    assert_eq!(updated["log_conversations"], json!(false));
    assert_eq!(updated["recall"]["budget_tokens"], json!(600));
    assert_eq!(updated["recall"]["team_limit"], json!(0));

    // Persisted: a fresh read sees it, and a partial set keeps the rest.
    let partial = f
        .ok(
            "openhuman.memory_policy_set",
            json!({ "log_conversations": true }),
        )
        .await;
    assert_eq!(partial["log_conversations"], json!(true));
    assert_eq!(partial["recall"]["budget_tokens"], json!(600));
    let again = f.ok("openhuman.memory_policy_get", json!({})).await;
    assert_eq!(again["recall"]["team_limit"], json!(0));

    for bad in [
        json!({ "budget_tokens": 1 }),
        json!({ "brain_limit": 1000 }),
        json!({ "pre_turn_timeout_ms": 0 }),
        json!({ "batch_turns": 4 }),
    ] {
        assert_eq!(
            f.code("openhuman.memory_policy_set", bad.clone()).await,
            "INVALID_REQUEST",
            "{bad}"
        );
    }
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

async fn wait_for_source(f: &Fixture, id: &str, want_items: u64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let listed = f.ok("openhuman.memory_sources_list", json!({})).await;
        let source = listed["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == json!(id))
            .unwrap_or_else(|| panic!("source {id} listed: {listed}"))
            .clone();
        if source["status"] != json!("syncing") && source["last_sync_at"].is_string() {
            assert!(
                source["items"].as_u64().unwrap_or(0) >= want_items,
                "sync stored the folder's files: {source}"
            );
            return source;
        }
        assert!(Instant::now() < deadline, "sync did not finish: {source}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn write_folder(root: &Path) -> PathBuf {
    let dir = root.join("notes");
    std::fs::create_dir_all(&dir).expect("mkdir notes");
    std::fs::write(
        dir.join("launch.md"),
        "# Launch plan\n\nThe launch is scheduled for the first Tuesday of March.\n",
    )
    .expect("write launch.md");
    std::fs::write(
        dir.join("budget.txt"),
        "The quarterly budget for the platform team is forty thousand dollars.\n",
    )
    .expect("write budget.txt");
    dir
}

#[tokio::test]
async fn sources_add_sync_list_and_remove() {
    let f = Fixture::new(true).await;
    let folder = write_folder(f.home.path());
    let target = folder.to_string_lossy().to_string();

    assert_eq!(
        f.ok("openhuman.memory_sources_list", json!({})).await["sources"],
        json!([])
    );

    // Validation.
    for (params, why) in [
        (
            json!({ "kind": "carrier-pigeon", "target": "x" }),
            "unknown kind",
        ),
        (json!({ "kind": "folder", "target": "  " }), "blank target"),
        (
            json!({ "kind": "rss", "target": "not a url" }),
            "rss needs a URL",
        ),
        (
            json!({ "kind": "github", "target": "only-one-part" }),
            "github needs owner/repo",
        ),
        (
            json!({ "kind": "folder", "target": target, "schedule_mins": 5 }),
            "schedule below the minimum",
        ),
    ] {
        assert_eq!(
            f.code("openhuman.memory_sources_add", params).await,
            "INVALID_REQUEST",
            "{why}"
        );
    }

    // add
    let added = f
        .ok(
            "openhuman.memory_sources_add",
            json!({ "kind": "folder", "target": target, "label": "Team notes", "schedule_mins": 60 }),
        )
        .await;
    let source = &added["source"];
    let id = source["id"].as_str().expect("source id").to_string();
    assert_eq!(source["kind"], json!("folder"));
    assert_eq!(source["label"], json!("Team notes"));
    assert_eq!(source["schedule_mins"], json!(60));
    assert_eq!(source["status"], json!("idle"));
    assert_eq!(source["items"], json!(0));
    assert!(source["last_sync_at"].is_null());
    // The same source twice is refused.
    assert_eq!(
        f.code(
            "openhuman.memory_sources_add",
            json!({ "kind": "folder", "target": target })
        )
        .await,
        "INVALID_REQUEST"
    );
    // A link and a GitHub repo register without syncing.
    let link = f
        .ok(
            "openhuman.memory_sources_add",
            json!({ "kind": "link", "target": "https://example.com/docs" }),
        )
        .await;
    assert_eq!(link["source"]["label"], json!("https://example.com/docs"));
    let repo = f
        .ok(
            "openhuman.memory_sources_add",
            json!({ "kind": "github", "target": "acme/widgets" }),
        )
        .await;
    assert_eq!(
        repo["source"]["target"],
        json!("https://github.com/acme/widgets")
    );
    let listed = f.ok("openhuman.memory_sources_list", json!({})).await;
    assert_eq!(listed["sources"].as_array().unwrap().len(), 3);
    // Drop the two network sources so a sync-all below only reads the folder.
    for extra in [&link["source"]["id"], &repo["source"]["id"]] {
        let removed = f
            .ok("openhuman.memory_sources_remove", json!({ "id": extra }))
            .await;
        assert_eq!(removed["removed"], json!(true));
    }

    // sync: unknown id is refused; a real one starts, then finishes.
    assert_eq!(
        f.code(
            "openhuman.memory_sources_sync",
            json!({ "id": "src-missing" })
        )
        .await,
        "INVALID_REQUEST"
    );
    let started = f
        .ok("openhuman.memory_sources_sync", json!({ "id": id }))
        .await;
    assert_eq!(started["started"], json!([id]));
    let synced = wait_for_source(&f, &id, 2).await;
    assert_eq!(synced["status"], json!("idle"), "{synced}");
    assert!(synced["error"].is_null(), "{synced}");

    // The folder's files are now documents tagged with the source.
    let docs = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["document"] } }),
        )
        .await;
    let items = docs["items"].as_array().unwrap();
    assert!(items.len() >= 2, "both files stored: {docs}");
    assert!(items.iter().all(|i| i["kind"] == json!("document")));
    assert!(
        items.iter().all(|i| i["meta"]["source"]["id"] == json!(id)),
        "documents carry their source: {docs}"
    );
    let texts: String = items.iter().filter_map(|i| i["text"].as_str()).collect();
    assert!(texts.contains("first Tuesday of March") && texts.contains("forty thousand"));
    assert!(
        items.iter().any(|i| i["meta"]["file_path"]
            .as_str()
            .is_some_and(|p| p.ends_with("launch.md"))),
        "meta.file_path is filled: {docs}"
    );

    // The documents are searchable.
    let found = f
        .ok(
            "openhuman.memory_fetch",
            json!({ "query": "launch Tuesday", "filter": { "kinds": ["document"] } }),
        )
        .await;
    assert!(!found["hits"].as_array().unwrap().is_empty(), "{found}");
    let by_source = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["document"], "source_id": id } }),
        )
        .await;
    assert_eq!(
        by_source["items"].as_array().unwrap().len(),
        items.len(),
        "the source filter keeps the source's items"
    );

    // Syncing again is a replay: nothing is duplicated.
    f.ok("openhuman.memory_sources_sync", json!({})).await;
    let resynced = wait_for_source(&f, &id, 2).await;
    let after = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["document"] } }),
        )
        .await;
    assert_eq!(
        after["items"].as_array().unwrap().len(),
        items.len(),
        "{resynced}"
    );

    // remove, keeping then forgetting items.
    let kept = f
        .ok(
            "openhuman.memory_sources_remove",
            json!({ "id": id, "forget_items": false }),
        )
        .await;
    assert_eq!(kept["removed"], json!(true));
    assert_eq!(
        f.ok("openhuman.memory_sources_list", json!({})).await["sources"],
        json!([])
    );
    let still = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["document"] } }),
        )
        .await;
    assert_eq!(still["items"].as_array().unwrap().len(), items.len());
    let gone = f
        .ok(
            "openhuman.memory_sources_remove",
            json!({ "id": id, "forget_items": true }),
        )
        .await;
    assert_eq!(gone["removed"], json!(false), "already removed");

    // Re-add and remove with forget_items: the source's documents are forgotten.
    let again = f
        .ok(
            "openhuman.memory_sources_add",
            json!({ "kind": "folder", "target": target }),
        )
        .await;
    let again_id = again["source"]["id"].as_str().unwrap().to_string();
    f.ok("openhuman.memory_sources_sync", json!({ "id": again_id }))
        .await;
    wait_for_source(&f, &again_id, 2).await;
    let removed = f
        .ok(
            "openhuman.memory_sources_remove",
            json!({ "id": again_id, "forget_items": true }),
        )
        .await;
    assert_eq!(removed["removed"], json!(true));
    let empty = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["document"], "source_id": again_id } }),
        )
        .await;
    assert_eq!(
        empty["items"],
        json!([]),
        "forget_items removed the source's documents: {empty}"
    );
    // The first source's documents were kept on purpose and stay.
    let kept_docs = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["document"], "source_id": id } }),
        )
        .await;
    assert_eq!(kept_docs["items"].as_array().unwrap().len(), items.len());
}

// ---------------------------------------------------------------------------
// context.md
// ---------------------------------------------------------------------------

#[tokio::test]
async fn local_files_and_text_file_under_the_files_source() {
    let f = Fixture::new(true).await;
    let path = f.home.path().join("handbook.md");
    std::fs::write(&path, "# Handbook\n\nExpenses are filed by the fifth.").expect("write");

    // Neither names a source: a file of any format, and pasted text, are
    // local files, not a per-format source.
    let file = f
        .ok(
            "openhuman.memory_brain_ingest",
            json!({ "path": path.display().to_string() }),
        )
        .await;
    assert_eq!(file["source"], json!("files"), "{file}");
    let text = f
        .ok(
            "openhuman.memory_brain_ingest",
            json!({ "text": "Payroll runs on the last Friday", "title": "Payroll" }),
        )
        .await;
    assert_eq!(text["source"], json!("files"), "{text}");

    let sources = f
        .ok_until("openhuman.memory_brain_sources", json!({}), |v| {
            v["sources"][0]["documents"] == json!(2)
        })
        .await;
    let listed: Vec<(String, u64)> = sources["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|s| {
            (
                s["source"].as_str().unwrap_or_default().to_string(),
                s["documents"].as_u64().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(listed, [("files".to_string(), 2)], "{sources}");

    let found = f
        .ok_until(
            "openhuman.memory_brain_search",
            json!({ "query": "expenses", "source": "files" }),
            |v| !v["hits"].as_array().is_none_or(Vec::is_empty),
        )
        .await;
    assert_eq!(found["hits"].as_array().unwrap().len(), 1, "{found}");
}

#[tokio::test]
async fn a_document_at_an_old_per_format_node_stays_listed_searchable_and_forgettable() {
    let f = Fixture::new(true).await;
    // Documents stored before the brain was filed by connector sit at a
    // per-format node (`source:pdf`); they stay visible beside `files`.
    let old = f
        .ok(
            "openhuman.memory_brain_ingest",
            json!({ "text": "The old vendor code is PV-7023", "source": "pdf" }),
        )
        .await;
    assert_eq!(old["source"], json!("pdf"), "{old}");
    f.ok(
        "openhuman.memory_brain_ingest",
        json!({ "text": "The new vendor code is PV-9000" }),
    )
    .await;
    let sources = f
        .ok_until("openhuman.memory_brain_sources", json!({}), |v| {
            v["sources"].as_array().is_some_and(|s| s.len() == 2)
        })
        .await;
    let mut listed: Vec<String> = sources["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|s| s["source"].as_str().unwrap_or_default().to_string())
        .collect();
    listed.sort();
    assert_eq!(listed, ["files", "pdf"], "{sources}");
    let found = f
        .ok_until(
            "openhuman.memory_brain_search",
            json!({ "query": "vendor code" }),
            |v| v["hits"].as_array().is_some_and(|h| h.len() == 2),
        )
        .await;
    assert_eq!(
        found["hits"].as_array().unwrap().len(),
        2,
        "an unscoped search reads both: {found}"
    );
    let gone = f
        .ok("openhuman.memory_brain_forget", json!({ "source": "pdf" }))
        .await;
    assert_eq!(gone["forgotten"], json!(1), "{gone}");
}

#[tokio::test]
async fn composio_is_no_longer_a_memory_source_kind() {
    let f = Fixture::new(true).await;
    let folder = write_folder(f.home.path());
    assert_eq!(
        f.code(
            "openhuman.memory_sources_add",
            json!({ "kind": "composio", "target": "gmail" })
        )
        .await,
        "INVALID_REQUEST"
    );
    for (kind, target) in [
        ("folder", folder.to_string_lossy().to_string()),
        ("file", format!("{}/launch.md", folder.to_string_lossy())),
        ("link", "https://example.com/docs".to_string()),
        ("github", "acme/widgets".to_string()),
        ("rss", "https://example.com/feed.xml".to_string()),
    ] {
        let added = f
            .ok(
                "openhuman.memory_sources_add",
                json!({ "kind": kind, "target": target }),
            )
            .await;
        assert_eq!(added["source"]["kind"], json!(kind));
    }
}

#[tokio::test]
async fn composio_sync_is_no_longer_a_method() {
    let f = Fixture::new(true).await;
    let response = f
        .call("openhuman.composio_sync", json!({ "connection_id": "c-1" }))
        .await;
    let message = response["error"]["message"].as_str().unwrap_or_default();
    assert!(
        response.get("error").is_some() && message.contains("unknown method"),
        "composio_sync must be an unknown method: {response}"
    );
}

#[tokio::test]
async fn a_stale_config_with_a_composio_source_loads_without_it() {
    let f = Fixture::new(true).await;
    let stale = r#"
[[memory.sources]]
id = "src-composio"
kind = "composio"
target = "gmail"

[[memory.sources]]
id = "src-notes"
kind = "folder"
target = "/tmp/stale-notes"

[[memory.sources]]
id = "src-feed"
kind = "rss"
target = "https://example.com/feed.xml"
"#;
    for dir in [
        f.home.path().join(".openhuman"),
        f.home
            .path()
            .join(".openhuman")
            .join("users")
            .join(MOCK_USER_ID),
    ] {
        let path = dir.join("config.toml");
        let mut text = std::fs::read_to_string(&path).expect("read config.toml");
        text.push_str(stale);
        std::fs::write(&path, text).expect("write config.toml");
    }
    let listed = f.ok("openhuman.memory_sources_list", json!({})).await;
    let mut ids: Vec<String> = listed["sources"]
        .as_array()
        .expect("sources array")
        .iter()
        .filter_map(|s| s["id"].as_str().map(str::to_string))
        .collect();
    ids.sort();
    assert_eq!(ids, ["src-feed", "src-notes"], "{listed}");
}

#[tokio::test]
async fn brain_pack_preview_and_jobs_round_trip() {
    let f = Fixture::new(true).await;

    let filed = f
        .ok(
            "openhuman.memory_brain_ingest",
            json!({ "text": "Standup is at nine every weekday", "source": "notion", "title": "Rituals" }),
        )
        .await;
    assert_eq!(filed["source"], json!("notion"));
    assert_eq!(
        f.code(
            "openhuman.memory_brain_ingest",
            json!({ "text": "x", "path": "/also" })
        )
        .await,
        "INVALID_REQUEST"
    );

    let sources = f.ok("openhuman.memory_brain_sources", json!({})).await;
    assert_eq!(
        sources["sources"][0]["source"],
        json!("notion"),
        "{sources}"
    );
    let found = f
        .ok(
            "openhuman.memory_brain_search",
            json!({ "query": "standup", "source": "notion" }),
        )
        .await;
    assert_eq!(found["hits"].as_array().unwrap().len(), 1, "{found}");

    // The pack a turn about standups would get carries the brain document.
    f.learn("The user prefers short answers").await;
    let preview = f
        .ok(
            "openhuman.memory_pack_preview",
            json!({ "query": "when is standup?" }),
        )
        .await;
    assert_eq!(preview["mode"], json!("turn"));
    let markdown = preview["pack"]["markdown"].as_str().unwrap_or_default();
    assert!(markdown.contains("nine"), "{preview}");
    let session = f.ok("openhuman.memory_pack_preview", json!({})).await;
    assert_eq!(session["mode"], json!("session"));

    // The ingest queued its source's belief build; the hosted engine builds on
    // its own schedule, so running it sends nothing and reports `scheduled`.
    let jobs = f.ok("openhuman.memory_jobs_list", json!({})).await;
    assert_eq!(jobs["pending"].as_array().unwrap().len(), 1, "{jobs}");
    let ran = f.ok("openhuman.memory_jobs_run", json!({})).await;
    assert_eq!(ran["runs"][0]["outcome"], json!("scheduled"), "{ran}");
    let after = f.ok("openhuman.memory_jobs_list", json!({})).await;
    assert_eq!(after["pending"], json!([]));
    assert_eq!(after["history"][0]["outcome"], json!("scheduled"));
    assert_eq!(
        f.code("openhuman.memory_jobs_run", json!({ "id": "nope" }))
            .await,
        "INVALID_REQUEST"
    );

    let gone = f
        .ok(
            "openhuman.memory_brain_forget",
            json!({ "source": "notion" }),
        )
        .await;
    assert_eq!(gone["forgotten"], json!(1), "{gone}");
    let agents = f.ok("openhuman.memory_agents_list", json!({})).await;
    assert_eq!(agents["root"], json!("root"));
}

/// Waits until some logged mock request satisfies `found`.
async fn wait_for_request(f: &Fixture, what: &str, found: impl Fn(&str, &str) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if f.mock
            .request_bodies()
            .await
            .iter()
            .any(|(url, body)| found(url, body))
        {
            return;
        }
        if Instant::now() >= deadline {
            let log: Vec<String> = f
                .mock
                .request_bodies()
                .await
                .into_iter()
                .map(|(url, body)| format!("{url} {}", body.chars().take(600).collect::<String>()))
                .collect();
            panic!("never saw {what}; mock log:\n{}", log.join("\n"));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
async fn a_later_chat_turn_carries_cached_memory_and_is_logged() {
    // Chat sessions resolve their hosted authority from the agent definition
    // registry, which the full runtime initialises at startup.
    openhuman_core::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins(
    )
    .expect("builtin agent definitions");
    let f = Fixture::new(true).await;
    f.learn("The user prefers launch events in Lisbon").await;

    let accepted = f
        .ok(
            "openhuman.channel_web_chat",
            json!({
                "client_id": "memory-v2-e2e",
                "thread_id": "launch-thread",
                "message": "Where should the launch event be held?",
                "model_override": "e2e-mock-model",
            }),
        )
        .await;
    assert_eq!(accepted["accepted"], json!(true), "{accepted}");

    // A cold turn starts without waiting for memory. Its background lookup
    // supplies a completed pack to a later turn on this same conversation.
    let idle = f
        .ok_until_with_timeout(
            "openhuman.channel_web_queue_status",
            json!({ "thread_id": "launch-thread" }),
            Duration::from_secs(60),
            |status| status["active"] == json!(false),
        )
        .await;
    assert_eq!(
        idle["active"],
        json!(false),
        "the cold turn finished: {idle}"
    );
    let first = f.mock.request_bodies().await;
    let inference = first
        .iter()
        .find(|(url, _)| url.contains("chat/completions"))
        .expect("the cold turn reached inference");
    assert!(
        !inference.1.contains("launch events in Lisbon"),
        "a cold turn has no cached pack"
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let before = f.mock.request_bodies().await.len();
        let accepted = f
            .ok(
                "openhuman.channel_web_chat",
                json!({
                    "client_id": "memory-v2-e2e",
                    "thread_id": "launch-thread",
                    "message": "Where should the launch event be held?",
                    "model_override": "e2e-mock-model",
                }),
            )
            .await;
        assert_eq!(accepted["accepted"], json!(true), "{accepted}");
        let idle = f
            .ok_until(
                "openhuman.channel_web_queue_status",
                json!({ "thread_id": "launch-thread" }),
                |status| status["active"] == json!(false),
            )
            .await;
        assert_eq!(
            idle["active"],
            json!(false),
            "the later turn finished: {idle}"
        );
        let requests = f.mock.request_bodies().await;
        if requests.iter().skip(before).any(|(url, body)| {
            url.contains("chat/completions")
                && body.contains("memory-context")
                && body.contains("launch events in Lisbon")
                && body.contains("completed background lookup for an earlier turn")
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "later turns never received the completed background memory pack"
        );
    }
    // Pre-turn logs the user's message, post-turn the reply, each as one turn
    // of the thread under the answering agent's node.
    wait_for_request(&f, "the user turn logged", |url, body| {
        url.contains("/memory/experience")
            && body.contains("Where should the launch event be held?")
            && body.contains("launch-thread")
    })
    .await;
    wait_for_request(&f, "the reply logged", |url, body| {
        url.contains("/memory/experience")
            && body.contains("launch-thread")
            && body.contains("\"assistant\"")
    })
    .await;

    // Every memory call carries the host's attribution, as every other
    // backend call does.
    let memory_calls: Vec<Value> = f
        .mock
        .request_rows()
        .await
        .into_iter()
        .filter(|row| {
            row["url"]
                .as_str()
                .is_some_and(|url| url.starts_with("/memory/"))
        })
        .collect();
    assert!(!memory_calls.is_empty());
    for row in &memory_calls {
        assert!(
            row["headers"]["x-sdk-name"].is_string(),
            "a memory call without x-sdk-name: {row}"
        );
    }

    // The pack is ephemeral: the committed transcript holds only what was
    // said (the reply is logged after the durable commit, so it exists).
    let transcript = f
        .ok(
            "openhuman.threads_transcript_get",
            json!({ "thread_id": "launch-thread" }),
        )
        .await;
    assert!(
        transcript
            .to_string()
            .contains("Where should the launch event be held?"),
        "the turn was committed: {transcript}"
    );
    assert!(
        !transcript.to_string().contains("memory-context"),
        "the pack never reaches the transcript: {transcript}"
    );
}

#[tokio::test]
async fn past_conversations_backfill_needs_consent_and_reports_status() {
    let f = Fixture::new(true).await;
    let status = f
        .ok("openhuman.memory_conversations_backfill_status", json!({}))
        .await;
    assert_eq!(status["state"]["phase"], json!("idle"), "{status}");
    assert_eq!(status["pending_threads"], json!(0));
    assert_eq!(
        f.code(
            "openhuman.memory_conversations_backfill_start",
            json!({ "consent": false })
        )
        .await,
        "INVALID_REQUEST"
    );
    let started = f
        .ok(
            "openhuman.memory_conversations_backfill_start",
            json!({ "consent": true }),
        )
        .await;
    assert_eq!(started["pending_turns"], json!(0), "{started}");
}

// ---------------------------------------------------------------------------
// Explorer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn explore_drills_down_and_items_get_reads_whole() {
    let f = Fixture::new(true).await;
    let folder = write_folder(f.home.path());
    let target = folder.to_string_lossy().to_string();
    let first = f.learn("The standup moved to ten").await;
    let second = f.learn("Invoices go out on the first").await;
    let added = f
        .ok(
            "openhuman.memory_sources_add",
            json!({ "kind": "folder", "target": target }),
        )
        .await;
    let source_id = added["source"]["id"].as_str().unwrap().to_string();
    f.ok("openhuman.memory_sources_sync", json!({ "id": source_id }))
        .await;
    wait_for_source(&f, &source_id, 2).await;

    // Root: by kind.
    let kinds = f
        .ok("openhuman.memory_explore", json!({ "facet": "kind" }))
        .await;
    assert_eq!(kinds["facet"], json!("kind"));
    let count = |page: &Value, value: &str| {
        page["buckets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["value"] == json!(value))
            .and_then(|b| b["count"].as_u64())
    };
    assert_eq!(count(&kinds, "learning"), Some(2), "{kinds}");
    assert_eq!(count(&kinds, "document"), Some(2), "{kinds}");
    assert_eq!(kinds["total"], json!(4));
    assert_eq!(kinds["truncated"], json!(false));

    // Drill: documents by source id, then by file.
    let path = json!([{ "facet": "kind", "value": "document" }]);
    let sources = f
        .ok(
            "openhuman.memory_explore",
            json!({ "facet": "source_id", "path": path }),
        )
        .await;
    assert_eq!(count(&sources, &source_id), Some(2), "{sources}");
    let files = f
        .ok(
            "openhuman.memory_explore",
            json!({
                "facet": "file_path",
                "path": [
                    { "facet": "kind", "value": "document" },
                    { "facet": "source_id", "value": source_id },
                ],
            }),
        )
        .await;
    assert_eq!(files["buckets"].as_array().unwrap().len(), 2, "{files}");

    // The same path lists the items there.
    let listed = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "path": [{ "facet": "kind", "value": "learning" }] }),
        )
        .await;
    let mut listed_ids = ids_of(&listed, "items");
    listed_ids.sort();
    let mut learned = vec![first.clone(), second.clone()];
    learned.sort();
    assert_eq!(listed_ids, learned);

    // Read whole, in the order asked, unknown ids left out.
    let got = f
        .ok(
            "openhuman.memory_items_get",
            json!({ "ids": [second, "no-such-item", first] }),
        )
        .await;
    assert_eq!(ids_of(&got, "items"), vec![second.clone(), first.clone()]);
    assert!(got["items"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Invoices go out on the first"));

    // Bad facets and paths are invalid requests.
    for (method, params) in [
        ("openhuman.memory_explore", json!({ "facet": "colour" })),
        (
            "openhuman.memory_explore",
            json!({ "facet": "kind", "path": [{ "facet": "kind", "value": "memo" }] }),
        ),
        (
            "openhuman.memory_explore",
            json!({ "facet": "kind", "limit": 0 }),
        ),
        ("openhuman.memory_items_get", json!({ "ids": [] })),
        (
            "openhuman.memory_items_list",
            json!({ "path": [{ "facet": "workspace", "value": " " }] }),
        ),
    ] {
        assert_eq!(
            f.code(method, params.clone()).await,
            "INVALID_REQUEST",
            "{params}"
        );
    }
}

// ---------------------------------------------------------------------------
// v1 import gate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn memory_nodes_keep_memory_apart_over_rpc() {
    let f = Fixture::new(true).await;
    let learn_at = |text: &'static str, namespace: &'static str| {
        let f = &f;
        async move {
            f.ok(
                "openhuman.memory_learn",
                json!({ "text": text, "kind": "fact", "meta": { "namespace": namespace } }),
            )
            .await["id"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let shared = f.learn("The user is called Sam").await;
    let researcher = learn_at("Prefer arxiv for papers", "agent:researcher").await;
    let writer = learn_at("Draft in British English", "agent:writer").await;

    // The namespace facet counts each node.
    let nodes = f
        .ok("openhuman.memory_explore", json!({ "facet": "namespace" }))
        .await;
    let values: Vec<&str> = nodes["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["value"].as_str().unwrap())
        .collect();
    for node in ["root", "agent:researcher", "agent:writer"] {
        assert!(values.contains(&node), "{nodes}");
    }

    // A researcher's reach: its node and the root, never the writer's.
    let listed = f
        .ok(
            "openhuman.memory_items_list",
            json!({ "filter": { "reach": { "at": "agent:researcher" } }, "limit": 50 }),
        )
        .await;
    let ids = ids_of(&listed, "items");
    assert!(
        ids.contains(&shared) && ids.contains(&researcher),
        "{listed}"
    );
    assert!(
        !ids.contains(&writer),
        "a sibling is out of reach: {listed}"
    );

    // `get` and `forget` within a reach leave the sibling's item alone.
    let got = f
        .ok(
            "openhuman.memory_items_get",
            json!({ "ids": [writer], "reach": { "at": "agent:researcher" } }),
        )
        .await;
    assert_eq!(got["items"], json!([]));
    let forgot = f
        .ok(
            "openhuman.memory_forget",
            json!({ "ids": [writer], "reach": { "at": "agent:researcher" } }),
        )
        .await;
    assert_eq!(forgot["forgotten"], json!(0));

    // A malformed node is refused.
    let refused = f
        .call(
            "openhuman.memory_items_list",
            json!({ "filter": { "reach": { "at": "not a node" } } }),
        )
        .await;
    assert!(refused.get("error").is_some(), "{refused}");
}

#[tokio::test]
async fn import_scan_finds_nothing_and_start_needs_consent() {
    let f = Fixture::new(true).await;

    let scan = f.ok("openhuman.memory_import_scan", json!({})).await;
    assert_eq!(scan["found"], json!(false));
    assert!(scan.get("counts").is_none_or(Value::is_null), "{scan}");

    let status = f.ok("openhuman.memory_import_status", json!({})).await;
    assert_eq!(status["state"]["phase"], json!("idle"));
    assert_eq!(status["state"]["failed"], json!(0));

    // Nothing finished, nothing skipped: there is nothing to retry.
    assert_eq!(
        f.code("openhuman.memory_import_retry_failed", json!({}))
            .await,
        "INVALID_REQUEST"
    );

    // Importing uploads local data: refused without consent, signed in or not.
    assert_eq!(
        f.code("openhuman.memory_import_start", json!({ "consent": false }))
            .await,
        "INVALID_REQUEST"
    );
    assert_eq!(
        f.code("openhuman.memory_import_start", json!({})).await,
        "INVALID_REQUEST"
    );
    // With consent but no v1 store there is nothing to import.
    assert_eq!(
        f.code("openhuman.memory_import_start", json!({ "consent": true }))
            .await,
        "INVALID_REQUEST"
    );
    assert_eq!(
        f.ok("openhuman.memory_import_status", json!({})).await["state"]["phase"],
        json!("idle")
    );

    // And nothing was uploaded.
    let paths = f.mock.request_paths().await;
    assert!(
        !paths
            .iter()
            .any(|p| p.starts_with("POST /memory/experience")),
        "a refused import stores nothing: {paths:?}"
    );
}

// ---------------------------------------------------------------------------
// Surface hygiene
// ---------------------------------------------------------------------------

#[tokio::test]
async fn memory_v2_registers_exactly_the_documented_methods() {
    let f = Fixture::new(false).await;
    let schema = rpc_harness::schema(&f.rpc_base).await;
    let mut memory: Vec<String> = schema["methods"]
        .as_array()
        .expect("schema methods")
        .iter()
        .filter(|m| m["namespace"] == json!("memory"))
        .filter_map(|m| m["method"].as_str().map(str::to_string))
        .collect();
    memory.sort();
    let mut expected: Vec<String> = [
        "engines_list",
        "engine_get",
        "engine_set",
        "policy_get",
        "policy_set",
        "pack_preview",
        "recall",
        "fetch",
        "learn",
        "forget",
        "erase_all",
        "items_list",
        "explore",
        "items_get",
        "agents_list",
        "conversations_backfill_status",
        "conversations_backfill_start",
        "brain_sources",
        "brain_search",
        "brain_ingest",
        "brain_forget",
        "sources_list",
        "sources_add",
        "sources_remove",
        "sources_sync",
        "jobs_list",
        "jobs_run",
        "import_scan",
        "import_start",
        "import_status",
        "import_retry_failed",
        "migration_scan",
        "migration_start",
        "migration_status",
        "migration_retry",
    ]
    .iter()
    .map(|m| format!("openhuman.memory_{m}"))
    .collect();
    expected.sort();
    assert_eq!(memory, expected);

    // The retired v1 surface does not dispatch.
    for method in [
        "openhuman.memory_init",
        "openhuman.memory_tree_ingest",
        "openhuman.memory_tree_search",
        "openhuman.memory_goals_list",
        "openhuman.memory_doc_put",
        "openhuman.memory_query_namespace",
        "openhuman.memory_sync_status_list",
        "openhuman.people_list",
        "openhuman.tree_summarizer_run",
        "openhuman.memory_sources_get",
        "openhuman.memory_sources_update",
        "openhuman.memory_context_get",
        "openhuman.memory_context_refresh",
        "openhuman.memory_context_set",
        "openhuman.memory_conversations_get",
        "openhuman.memory_conversations_set",
    ] {
        let response = f.call(method, json!({})).await;
        let message = response["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("unknown method"),
            "{method} must be gone, got: {response}"
        );
    }
}
