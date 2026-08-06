//! JSON-RPC E2E coverage for the `medulla_local` namespace (Flavor A draft).
//!
//! Boots the real Axum JSON-RPC router (same pattern as
//! `domain_modules_e2e.rs`) and exercises the `medulla_local` controllers
//! hermetically: no `medulla-serve` entry is configured, so `status` must
//! report a well-formed not-running snapshot without spawning Node or touching
//! the network, and `instruct` must fail cleanly with the actionable
//! "serve entry not configured" error. Run with:
//! `cargo test --test medulla_local_e2e`.
#![cfg(feature = "medulla-local")]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

const TEST_RPC_TOKEN: &str = "medulla-local-e2e-token";

/// The startup-failure needle the supervisor must surface with no serve entry
/// configured: the actionable configuration error on unix, and the typed
/// unsupported-platform error on targets without unix domain sockets (where
/// the serve transport is stubbed out).
#[cfg(unix)]
const STARTUP_UNAVAILABLE_NEEDLE: &str = "serve entry not configured";
#[cfg(not(unix))]
const STARTUP_UNAVAILABLE_NEEDLE: &str = "unavailable on this platform";

static AUTH_INIT: OnceLock<()> = OnceLock::new();
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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

/// Serializes tests in this binary: `HOME` and the serve-entry env override are
/// process-global, as is the medulla supervisor cache.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    let mutex = ENV_LOCK.get_or_init(|| Mutex::new(()));
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn ensure_rpc_auth() {
    AUTH_INIT.get_or_init(|| {
        // SAFETY: guarded by OnceLock and set once before the router for this
        // test binary is used concurrently.
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN) };
        let token_dir = std::env::temp_dir().join("openhuman-medulla-local-e2e-auth");
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
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "e2e-model"
default_temperature = 0.2

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
    std::fs::write(openhuman_dir.join("config.toml"), cfg).expect("write config.toml");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(cfg).expect("test config must match schema");
}

struct TestHarness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

async fn setup() -> TestHarness {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    write_min_config(&openhuman_home);

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        // The whole point of this suite: no serve entry anywhere, so the
        // supervisor must fail its spawn attempt with the actionable
        // configuration error instead of probing Node or the network.
        EnvVarGuard::unset("OPENHUMAN_MEDULLA_SERVE_ENTRY"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];

    let (addr, join) = serve_rpc().await;
    TestHarness {
        _tmp: tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        join,
    }
}

async fn rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("client");
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .unwrap_or_else(|err| panic!("POST {url} {method}: {err}"));
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "HTTP transport should accept {method}"
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
        .unwrap_or_else(|| panic!("{context}: expected JSON-RPC error, got: {value}"))
}

fn err_message(value: &Value, context: &str) -> String {
    err(value, context)
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error without string message: {value}"))
        .to_string()
}

/// Unwrap the CLI-compatible envelope: handlers that attach logs return
/// `{ "result": <payload>, "logs": [...] }`; otherwise the payload is bare.
fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    let result = ok(value, context);
    result.get("result").unwrap_or(result)
}

#[tokio::test]
async fn medulla_local_schema_catalog_exposes_status_and_instruct() {
    let _lock = env_lock();
    let harness = setup().await;

    let url = format!("{}/schema", harness.rpc_base.trim_end_matches('/'));
    let schema = reqwest::get(&url)
        .await
        .unwrap_or_else(|err| panic!("GET {url}: {err}"))
        .json::<Value>()
        .await
        .expect("schema json");
    let methods: Vec<String> = schema
        .get("methods")
        .and_then(Value::as_array)
        .expect("schema methods array")
        .iter()
        .map(|method| {
            method
                .get("method")
                .and_then(Value::as_str)
                .expect("method name")
                .to_string()
        })
        .collect();

    for method in [
        "openhuman.medulla_local_status",
        "openhuman.medulla_local_instruct",
    ] {
        assert!(
            methods.iter().any(|name| name == method),
            "schema catalog must expose {method}; got {} methods",
            methods.len()
        );
    }

    harness.join.abort();
}

#[tokio::test]
async fn medulla_local_status_and_instruct_round_trip_without_serve_child() {
    let _lock = env_lock();
    let harness = setup().await;

    // `status` with no serve entry configured: a well-formed idle snapshot,
    // never a spawn attempt against Node or the network. The failed startup is
    // folded into `message` rather than surfaced as an RPC error.
    let status_response = rpc(
        &harness.rpc_base,
        40_001,
        "openhuman.medulla_local_status",
        json!({}),
    )
    .await;
    let status = payload(&status_response, "medulla_local_status");
    assert_eq!(
        status.get("enabled").and_then(Value::as_bool),
        Some(true),
        "status.enabled: {status_response}"
    );
    assert_eq!(
        status.get("running").and_then(Value::as_bool),
        Some(false),
        "status.running must be false without a serve child: {status_response}"
    );
    assert!(
        status.get("serve_version").is_some_and(Value::is_null),
        "status.serve_version must be null when not connected: {status_response}"
    );
    assert!(
        status.get("session_id").is_some_and(Value::is_null),
        "status.session_id must be null when not connected: {status_response}"
    );
    assert_eq!(
        status
            .get("ports")
            .and_then(Value::as_array)
            .map(Vec::as_slice),
        Some(&[][..]),
        "status.ports must be an empty array when not connected: {status_response}"
    );
    let message = status
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        message.contains(STARTUP_UNAVAILABLE_NEEDLE),
        "status.message must carry the actionable startup error: {status_response}"
    );

    // `instruct` without the required `message` param fails at deserialization,
    // before any supervisor involvement.
    let missing_param = rpc(
        &harness.rpc_base,
        40_002,
        "openhuman.medulla_local_instruct",
        json!({}),
    )
    .await;
    let missing_message = err_message(&missing_param, "medulla_local_instruct missing message");
    assert!(
        missing_message.contains("message"),
        "instruct without params should name the missing `message` field: {missing_param}"
    );

    // `instruct` with a message but no configured serve entry fails cleanly
    // with the actionable configuration error (no hang, no Node spawn).
    let instruct_response = rpc(
        &harness.rpc_base,
        40_003,
        "openhuman.medulla_local_instruct",
        json!({ "message": "hello from the e2e suite" }),
    )
    .await;
    let instruct_error = err_message(&instruct_response, "medulla_local_instruct unconfigured");
    assert!(
        instruct_error.contains(STARTUP_UNAVAILABLE_NEEDLE),
        "instruct must surface the actionable startup error: {instruct_response}"
    );

    harness.join.abort();
}
