//! Memory v2 over JSON-RPC, end to end against the shared mock backend.
//!
//! Boots the core router in-process, points it at `scripts/mock-api-server.mjs`
//! (the in-memory TinyHumans `/memory/*` CortexDB proxy in
//! `scripts/mock-api/routes/memory.mjs`), signs in as the mock user and drives
//! every `openhuman.memory_*` method in `docs/specs/memory-v2.md`:
//!
//! - engines: list / get / set, the off state, and the structured error codes;
//! - learn -> items_list -> fetch -> recall -> forget on the hosted engine;
//! - conversations settings;
//! - sources: add a folder, sync it, read its items back, remove it;
//! - context.md: refresh / get / set;
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
use std::sync::MutexGuard;
use std::time::{Duration, Instant};

use env_guard::{env_lock_with_file_keyring, EnvVarGuard};
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
    _lock: MutexGuard<'static, ()>,
}

impl Fixture {
    /// Boots the stack; `signed_in` stores the mock session first.
    async fn new(signed_in: bool) -> Self {
        let lock = env_lock_with_file_keyring();
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
        ("openhuman.memory_sources_sync", json!({})),
        ("openhuman.memory_context_refresh", json!({})),
        ("openhuman.memory_import_start", json!({ "consent": true })),
    ];
    for (method, params) in off_calls {
        assert_eq!(f.code(method, params).await, "MEMORY_OFF", "{method}");
    }

    // Local settings and state still answer while memory is off.
    let conversations = f.ok("openhuman.memory_conversations_get", json!({})).await;
    assert_eq!(conversations["enabled"], json!(true));
    assert_eq!(
        f.ok("openhuman.memory_sources_list", json!({})).await["sources"],
        json!([])
    );
    assert_eq!(
        f.ok("openhuman.memory_context_get", json!({})).await["markdown"],
        json!("")
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
async fn conversations_get_and_set_round_trip() {
    let f = Fixture::new(true).await;

    let defaults = f.ok("openhuman.memory_conversations_get", json!({})).await;
    assert_eq!(defaults["enabled"], json!(true));
    assert_eq!(defaults["batch_turns"], json!(4));
    assert_eq!(defaults["idle_secs"], json!(120));
    assert_eq!(defaults["recent"], json!([]));

    let updated = f
        .ok(
            "openhuman.memory_conversations_set",
            json!({ "enabled": false, "batch_turns": 2, "idle_secs": 30 }),
        )
        .await;
    assert_eq!(updated["enabled"], json!(false));
    assert_eq!(updated["batch_turns"], json!(2));
    assert_eq!(updated["idle_secs"], json!(30));

    // Persisted: a fresh read sees it, and a partial set keeps the rest.
    let partial = f
        .ok(
            "openhuman.memory_conversations_set",
            json!({ "enabled": true }),
        )
        .await;
    assert_eq!(partial["enabled"], json!(true));
    assert_eq!(partial["batch_turns"], json!(2));
    let again = f.ok("openhuman.memory_conversations_get", json!({})).await;
    assert_eq!(again["idle_secs"], json!(30));

    for bad in [
        json!({ "batch_turns": 0 }),
        json!({ "batch_turns": 1000 }),
        json!({ "idle_secs": 0 }),
        json!({ "idle_secs": 10_000_000 }),
    ] {
        assert_eq!(
            f.code("openhuman.memory_conversations_set", bad.clone())
                .await,
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
async fn context_get_set_and_refresh() {
    let f = Fixture::new(true).await;

    let initial = f.ok("openhuman.memory_context_get", json!({})).await;
    assert_eq!(initial["enabled"], json!(true));
    assert_eq!(initial["interval_mins"], json!(360));
    assert_eq!(initial["budget_tokens"], json!(2000));
    assert_eq!(initial["markdown"], json!(""));
    assert!(initial["generated_at"].is_null());

    let set = f
        .ok(
            "openhuman.memory_context_set",
            json!({ "enabled": true, "interval_mins": 30, "budget_tokens": 500 }),
        )
        .await;
    assert_eq!(set["interval_mins"], json!(30));
    assert_eq!(set["budget_tokens"], json!(500));
    let partial = f
        .ok("openhuman.memory_context_set", json!({ "enabled": false }))
        .await;
    assert_eq!(partial["enabled"], json!(false));
    assert_eq!(partial["interval_mins"], json!(30));
    f.ok("openhuman.memory_context_set", json!({ "enabled": true }))
        .await;

    for bad in [
        json!({ "interval_mins": 1 }),
        json!({ "budget_tokens": 10 }),
        json!({ "budget_tokens": 1_000_000 }),
    ] {
        assert_eq!(
            f.code("openhuman.memory_context_set", bad.clone()).await,
            "INVALID_REQUEST",
            "{bad}"
        );
    }

    // refresh compiles a brief from what memory holds and persists it.
    f.learn("The team standup is at nine every weekday").await;
    let refreshed = f.ok("openhuman.memory_context_refresh", json!({})).await;
    assert!(refreshed["generated_at"].is_string(), "{refreshed}");
    assert!(refreshed["tokens"].as_u64().unwrap_or(0) > 0, "{refreshed}");
    let markdown = refreshed["markdown"].as_str().unwrap_or_default();
    assert!(
        !markdown.trim().is_empty(),
        "context.md has content: {refreshed}"
    );

    let read_back = f.ok("openhuman.memory_context_get", json!({})).await;
    assert_eq!(read_back["markdown"], refreshed["markdown"]);
    assert_eq!(read_back["generated_at"], refreshed["generated_at"]);
    assert_eq!(read_back["budget_tokens"], json!(500));
}

// ---------------------------------------------------------------------------
// v1 import gate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_scan_finds_nothing_and_start_needs_consent() {
    let f = Fixture::new(true).await;

    let scan = f.ok("openhuman.memory_import_scan", json!({})).await;
    assert_eq!(scan["found"], json!(false));
    assert!(scan.get("counts").map_or(true, Value::is_null), "{scan}");

    let status = f.ok("openhuman.memory_import_status", json!({})).await;
    assert_eq!(status["state"]["phase"], json!("idle"));

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
        "recall",
        "fetch",
        "learn",
        "forget",
        "items_list",
        "conversations_get",
        "conversations_set",
        "sources_list",
        "sources_add",
        "sources_remove",
        "sources_sync",
        "context_get",
        "context_refresh",
        "context_set",
        "import_scan",
        "import_start",
        "import_status",
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
    ] {
        let response = f.call(method, json!({})).await;
        let message = response["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("unknown method"),
            "{method} must be gone, got: {response}"
        );
    }
}
