//! JSON-RPC E2E tests for the embeddings domain.
//!
//! Spins up the core HTTP router against a temp workspace and exercises the
//! `openhuman.embeddings_*` controller surface end-to-end. No real Voyage /
//! OpenAI / Cohere calls are made — tests either use the "none" noop provider
//! or assert error shapes from providers that require live credentials.
//!
//! Run with: `cargo test --test embeddings_rpc_e2e`

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

// ── Auth / token setup ────────────────────────────────────────────────────────

const TEST_RPC_TOKEN: &str = "embeddings-e2e-test-token";
static E2E_AUTH_INIT: OnceLock<()> = OnceLock::new();

/// Serialises tests: env-var mutations (`HOME`, `OPENHUMAN_WORKSPACE`,
/// `OPENHUMAN_APP_ENV`) are process-global. The OnceLock+Mutex pattern mirrors
/// `json_rpc_e2e.rs` so tests don't race each other.
static EMBEDDINGS_E2E_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn embeddings_e2e_env_lock() -> std::sync::MutexGuard<'static, ()> {
    let mutex = EMBEDDINGS_E2E_ENV_LOCK.get_or_init(|| Mutex::new(()));
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn ensure_test_rpc_auth() {
    E2E_AUTH_INIT.get_or_init(|| {
        // SAFETY: runs exactly once inside OnceLock before any concurrent env
        // reads occur. Rust 1.81+ requires unsafe for set_var in multi-threaded
        // contexts; the OnceLock guard limits the mutation to a single call.
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN) };
        let token_dir = std::env::temp_dir().join("openhuman-embeddings-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token for embeddings_rpc_e2e");
    });
}

// ── Env-var guard (RAII restore) ──────────────────────────────────────────────

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, path.as_os_str()) };
        Self { key, old }
    }

    #[allow(dead_code)]
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value) };
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::remove_var(key) };
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

// ── Minimal config writer ─────────────────────────────────────────────────────

fn write_min_config(config_dir: &Path) {
    std::fs::create_dir_all(config_dir).expect("mkdir for embeddings e2e config");
    let cfg = r#"api_url = "http://127.0.0.1:1"
default_model = "e2e-mock-model"
default_temperature = 0.7
chat_onboarding_completed = true

[secrets]
encrypt = false
"#;
    std::fs::write(config_dir.join("config.toml"), cfg).expect("write embeddings e2e config.toml");
}

// ── HTTP server helpers ───────────────────────────────────────────────────────

async fn serve_on_ephemeral() -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    ensure_test_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let app = build_core_http_router(false);
    let handle = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, handle)
}

async fn post_json_rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("build reqwest client");
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    });
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {url}: {e}"));
    assert!(
        resp.status().is_success(),
        "HTTP error {} calling {}",
        resp.status(),
        method
    );
    resp.json::<Value>()
        .await
        .unwrap_or_else(|e| panic!("deserialize json for {method}: {e}"))
}

// ── Assertion helpers ─────────────────────────────────────────────────────────

fn assert_no_rpc_error<'a>(v: &'a Value, ctx: &str) -> &'a Value {
    if let Some(err) = v.get("error") {
        panic!("{ctx}: unexpected JSON-RPC error: {err}");
    }
    v.get("result")
        .unwrap_or_else(|| panic!("{ctx}: missing 'result' field in: {v}"))
}

// ── Test scaffolding: set up HOME + OPENHUMAN_WORKSPACE then start RPC server ─

/// Returns `(rpc_base, tempdir, guards)`. The `guards` tuple keeps all
/// `EnvVarGuard` values alive for the duration of the test.
async fn setup_embeddings_test() -> (
    String,
    tempfile::TempDir,
    (EnvVarGuard, EnvVarGuard, EnvVarGuard, EnvVarGuard),
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let openhuman_home = home.join(".openhuman");

    write_min_config(&openhuman_home);
    // Also write user-local config so that post-login config loads succeed.
    write_min_config(&openhuman_home.join("users").join("local"));

    let home_guard = EnvVarGuard::set_to_path("HOME", &home);
    let workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let backend_guard = EnvVarGuard::unset("BACKEND_URL");
    let vite_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (addr, join) = serve_on_ephemeral().await;
    let rpc_base = format!("http://{addr}");

    (
        rpc_base,
        tmp,
        (home_guard, workspace_guard, backend_guard, vite_guard),
        join,
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn embeddings_get_settings_returns_catalog() {
    let _lock = embeddings_e2e_env_lock();
    let (rpc_base, _tmp, _guards, _join) = setup_embeddings_test().await;

    let resp = post_json_rpc(&rpc_base, 1, "openhuman.embeddings_get_settings", json!({})).await;
    let result = assert_no_rpc_error(&resp, "embeddings_get_settings");

    // Unwrap one more layer if the controller wraps in {result: ...}
    let inner = result.get("result").unwrap_or(result);

    // Required top-level fields
    assert!(
        inner.get("provider").is_some(),
        "get_settings result missing 'provider': {inner}"
    );
    assert!(
        inner.get("model").is_some(),
        "get_settings result missing 'model': {inner}"
    );
    assert!(
        inner.get("dimensions").is_some(),
        "get_settings result missing 'dimensions': {inner}"
    );

    // Provider catalog
    let providers = inner
        .get("providers")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("get_settings result missing 'providers' array: {inner}"));
    assert!(
        providers.len() >= 5,
        "expected at least 5 providers in catalog, got {}: {providers:?}",
        providers.len()
    );

    // Each entry has the required fields
    for entry in providers.iter() {
        for field in &["slug", "label", "requires_api_key", "models"] {
            assert!(
                entry.get(field).is_some(),
                "provider entry missing '{field}': {entry}"
            );
        }
    }

    // Managed provider should not require an API key
    let managed = providers
        .iter()
        .find(|e| e.get("slug").and_then(Value::as_str) == Some("managed"))
        .expect("catalog must include 'managed' provider");
    assert_eq!(
        managed.get("requires_api_key").and_then(Value::as_bool),
        Some(false),
        "managed provider must not require an API key"
    );

    // Voyage provider requires a key
    let voyage = providers
        .iter()
        .find(|e| e.get("slug").and_then(Value::as_str) == Some("voyage"))
        .expect("catalog must include 'voyage' provider");
    assert_eq!(
        voyage.get("requires_api_key").and_then(Value::as_bool),
        Some(true),
        "voyage provider must require an API key"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn embeddings_update_settings_switches_provider() {
    let _lock = embeddings_e2e_env_lock();
    let (rpc_base, _tmp, _guards, _join) = setup_embeddings_test().await;

    // Switch to "none" (noop) which has 0 dimensions — dimension change from
    // the default requires confirm_wipe. We pass it so the update goes through.
    let update = post_json_rpc(
        &rpc_base,
        2,
        "openhuman.embeddings_update_settings",
        json!({ "provider": "none", "confirm_wipe": true }),
    )
    .await;
    let update_result = assert_no_rpc_error(&update, "embeddings_update_settings");
    let inner = update_result.get("result").unwrap_or(update_result);

    assert_eq!(
        inner.get("provider").and_then(Value::as_str),
        Some("none"),
        "update_settings should return provider=none: {inner}"
    );

    // Subsequent get_settings should reflect the change
    let get = post_json_rpc(&rpc_base, 3, "openhuman.embeddings_get_settings", json!({})).await;
    let get_result = assert_no_rpc_error(&get, "embeddings_get_settings after update");
    let get_inner = get_result.get("result").unwrap_or(get_result);

    assert_eq!(
        get_inner.get("provider").and_then(Value::as_str),
        Some("none"),
        "get_settings after update should show provider=none: {get_inner}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn embeddings_update_settings_dimension_change_requires_wipe() {
    let _lock = embeddings_e2e_env_lock();
    let (rpc_base, _tmp, _guards, _join) = setup_embeddings_test().await;

    // First set provider to voyage (a provider that supports multiple dims)
    let _ = post_json_rpc(
        &rpc_base,
        10,
        "openhuman.embeddings_update_settings",
        json!({ "provider": "voyage", "model": "voyage-3-large", "dimensions": 1024, "confirm_wipe": true }),
    )
    .await;

    // Now try to change only dimensions without confirm_wipe — should get the
    // EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE sentinel in the result body.
    let resp = post_json_rpc(
        &rpc_base,
        11,
        "openhuman.embeddings_update_settings",
        json!({ "dimensions": 512 }),
    )
    .await;
    let result = assert_no_rpc_error(&resp, "update_settings no confirm_wipe");
    let inner = result.get("result").unwrap_or(result);

    assert_eq!(
        inner.get("error").and_then(Value::as_str),
        Some("EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE"),
        "expected EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE sentinel in response body: {inner}"
    );
    assert!(
        inner.get("old_dimensions").is_some(),
        "response should include old_dimensions: {inner}"
    );
    assert!(
        inner.get("new_dimensions").is_some(),
        "response should include new_dimensions: {inner}"
    );

    // With confirm_wipe=true the change should succeed
    let confirmed = post_json_rpc(
        &rpc_base,
        12,
        "openhuman.embeddings_update_settings",
        json!({ "dimensions": 512, "confirm_wipe": true }),
    )
    .await;
    let confirmed_result = assert_no_rpc_error(&confirmed, "update_settings with confirm_wipe");
    let confirmed_inner = confirmed_result.get("result").unwrap_or(confirmed_result);

    // The confirmed update should NOT carry the error sentinel
    assert!(
        confirmed_inner.get("error").is_none(),
        "confirmed update must not return an error sentinel: {confirmed_inner}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn embeddings_set_and_clear_api_key() {
    let _lock = embeddings_e2e_env_lock();
    let (rpc_base, _tmp, _guards, _join) = setup_embeddings_test().await;

    // Store a key for voyage
    let set_resp = post_json_rpc(
        &rpc_base,
        20,
        "openhuman.embeddings_set_api_key",
        json!({ "provider": "voyage", "api_key": "voy-test-1234" }),
    )
    .await;
    let set_result = assert_no_rpc_error(&set_resp, "embeddings_set_api_key");
    let set_inner = set_result.get("result").unwrap_or(set_result);
    assert_eq!(
        set_inner.get("stored").and_then(Value::as_bool),
        Some(true),
        "set_api_key should report stored=true: {set_inner}"
    );

    // get_settings should now show has_api_key=true for voyage
    let get_resp = post_json_rpc(
        &rpc_base,
        21,
        "openhuman.embeddings_get_settings",
        json!({}),
    )
    .await;
    let get_result = assert_no_rpc_error(&get_resp, "get_settings after set_api_key");
    let get_inner = get_result.get("result").unwrap_or(get_result);
    let providers = get_inner
        .get("providers")
        .and_then(Value::as_array)
        .expect("providers array missing");
    let voyage = providers
        .iter()
        .find(|e| e.get("slug").and_then(Value::as_str) == Some("voyage"))
        .expect("voyage provider missing from catalog");
    assert_eq!(
        voyage.get("has_api_key").and_then(Value::as_bool),
        Some(true),
        "voyage should report has_api_key=true after storing key: {voyage}"
    );

    // Clear the key
    let clear_resp = post_json_rpc(
        &rpc_base,
        22,
        "openhuman.embeddings_clear_api_key",
        json!({ "provider": "voyage" }),
    )
    .await;
    let clear_result = assert_no_rpc_error(&clear_resp, "embeddings_clear_api_key");
    let clear_inner = clear_result.get("result").unwrap_or(clear_result);
    assert_eq!(
        clear_inner.get("cleared").and_then(Value::as_bool),
        Some(true),
        "clear_api_key should report cleared=true: {clear_inner}"
    );

    // has_api_key should be false again
    let get2_resp = post_json_rpc(
        &rpc_base,
        23,
        "openhuman.embeddings_get_settings",
        json!({}),
    )
    .await;
    let get2_result = assert_no_rpc_error(&get2_resp, "get_settings after clear_api_key");
    let get2_inner = get2_result.get("result").unwrap_or(get2_result);
    let providers2 = get2_inner
        .get("providers")
        .and_then(Value::as_array)
        .expect("providers array missing after clear");
    let voyage2 = providers2
        .iter()
        .find(|e| e.get("slug").and_then(Value::as_str) == Some("voyage"))
        .expect("voyage missing after clear");
    assert_eq!(
        voyage2.get("has_api_key").and_then(Value::as_bool),
        Some(false),
        "voyage should report has_api_key=false after clearing key: {voyage2}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn embeddings_test_connection_with_none_provider() {
    let _lock = embeddings_e2e_env_lock();
    let (rpc_base, _tmp, _guards, _join) = setup_embeddings_test().await;

    // Switch to "none" so test_connection uses the noop provider (no network).
    let _ = post_json_rpc(
        &rpc_base,
        30,
        "openhuman.embeddings_update_settings",
        json!({ "provider": "none", "confirm_wipe": true }),
    )
    .await;

    let resp = post_json_rpc(
        &rpc_base,
        31,
        "openhuman.embeddings_test_connection",
        json!({}),
    )
    .await;
    let result = assert_no_rpc_error(&resp, "embeddings_test_connection none");
    let inner = result.get("result").unwrap_or(result);

    // The noop provider's embed() returns an empty vec, which causes the
    // test_connection result to show success=false with an "Empty embedding
    // result" error — that's acceptable. What we assert is that the call
    // completes without a JSON-RPC level error and returns a structured body.
    assert!(
        inner.get("success").is_some(),
        "test_connection should return a 'success' field: {inner}"
    );
    assert!(
        inner.get("provider").is_some(),
        "test_connection should return a 'provider' field: {inner}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn embeddings_embed_with_none_returns_empty_vectors() {
    let _lock = embeddings_e2e_env_lock();
    let (rpc_base, _tmp, _guards, _join) = setup_embeddings_test().await;

    // Switch to noop so embed() doesn't require network.
    let _ = post_json_rpc(
        &rpc_base,
        40,
        "openhuman.embeddings_update_settings",
        json!({ "provider": "none", "confirm_wipe": true }),
    )
    .await;

    let resp = post_json_rpc(
        &rpc_base,
        41,
        "openhuman.embeddings_embed",
        json!({ "inputs": ["hello", "world"] }),
    )
    .await;
    let result = assert_no_rpc_error(&resp, "embeddings_embed none");
    let inner = result.get("result").unwrap_or(result);

    // NoopEmbedding.embed() returns an empty vec — count and dimensions should both be 0.
    assert_eq!(
        inner.get("count").and_then(Value::as_u64),
        Some(0),
        "noop provider should return count=0: {inner}"
    );
    assert_eq!(
        inner.get("dimensions").and_then(Value::as_u64),
        Some(0),
        "noop provider should return dimensions=0: {inner}"
    );
    let vectors = inner
        .get("vectors")
        .and_then(Value::as_array)
        .expect("embed result must include 'vectors' array: {inner}");
    assert!(
        vectors.is_empty(),
        "noop provider must return empty vectors array: {vectors:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn legacy_alias_inference_embed_resolves() {
    let _lock = embeddings_e2e_env_lock();
    let (rpc_base, _tmp, _guards, _join) = setup_embeddings_test().await;

    // Set provider to none so the embed call itself doesn't fail on missing keys.
    let _ = post_json_rpc(
        &rpc_base,
        50,
        "openhuman.embeddings_update_settings",
        json!({ "provider": "none", "confirm_wipe": true }),
    )
    .await;

    // Call via the legacy alias — must NOT return an "unknown method" JSON-RPC
    // error; the alias table must rewrite it to openhuman.embeddings_embed.
    let resp = post_json_rpc(
        &rpc_base,
        51,
        "openhuman.inference_embed",
        json!({ "inputs": [] }),
    )
    .await;

    // If the alias resolution failed we'd get a JSON-RPC error with code -32601.
    if let Some(err) = resp.get("error") {
        let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
        assert_ne!(
            code, -32601,
            "legacy alias openhuman.inference_embed resolved to 'method not found' — alias table may be broken: {err}"
        );
    }

    // The resolved call should succeed (no JSON-RPC error) and return a result.
    let result = assert_no_rpc_error(&resp, "legacy inference_embed alias");
    let inner = result.get("result").unwrap_or(result);
    assert!(
        inner.get("vectors").is_some() || inner.get("count").is_some(),
        "legacy alias should resolve to embeddings_embed and return vector data: {inner}"
    );
}
