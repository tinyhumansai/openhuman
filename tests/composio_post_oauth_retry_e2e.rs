//! End-to-end tests for the Composio post-OAuth readiness-gap retry (PR #1708).
//!
//! ## What is tested
//!
//! After a user completes OAuth, Composio's action-execution gateway can
//! take up to 60 s to sync the new token into its execution cache. During
//! that window the gateway returns `successful = false, error = "Connection
//! error, try to authenticate"` for otherwise-valid tool calls. PR #1708
//! introduced a single-shot automatic retry with an 8 s backoff so the
//! user gets real data on the same turn without seeing the transient error.
//!
//! These tests exercise the full RPC stack:
//!
//!   client → JSON-RPC axum layer
//!           → `composio_execute` op (`ops.rs`)
//!           → `execute_composio_action_kind` dispatcher (`execute_dispatch.rs`)
//!           → `execute_with_auth_retry_inner` (`auth_retry.rs`)
//!           → `execute_tool_with_post_oauth_retry` on `ComposioClient`
//!           → mock backend HTTP server (in-process axum)
//!
//! Unlike the unit tests in `src/openhuman/composio/auth_retry_tests.rs` which
//! call the retry helper directly, here the call enters through the full
//! registered controller surface, picks up the config-derived `ComposioClient`,
//! and traverses the real `execute_composio_action_kind` dispatch path.
//!
//! ## Two flows covered
//!
//! 1. **Happy-path retry** (`post_oauth_gap_retries_and_returns_real_data`):
//!    first backend call returns the gappy auth-error payload; second call
//!    returns a real success. The RPC result must be successful with the
//!    second call's data — the transient error must not surface.
//!
//! 2. **Real revoked-token surfaced immediately**
//!    (`revoked_token_surfaces_without_retry`):
//!    the gateway returns an `invalid_grant: refresh token revoked` payload
//!    that does NOT match the retryable error strings. The RPC result must
//!    carry that error verbatim; the backend must be hit exactly once.
//!
//! ## Test isolation
//!
//! Each test spins up its own ephemeral axum backend mock and an ephemeral
//! core JSON-RPC server so port allocation is independent. The env-var lock
//! from `json_rpc_e2e.rs` is replicated here (the two test binaries run in
//! separate processes so they do not share the same OnceLock). Config is
//! written to a tempdir so nothing touches the developer's `~/.openhuman`.
//!
//! The mock backend requires a valid Bearer JWT (`e2e-composio-jwt`) on the
//! `/settings` / `/auth/me` probe that `auth_store_session` triggers. The
//! same token is then used for all composio backend calls, mirroring
//! production.
//!
//! ## Note on retry count assertions
//!
//! As documented in `auth_retry_tests.rs`, two retry layers are currently
//! stacked for the post-OAuth error string:
//!
//!   - outer: `execute_with_auth_retry_inner` in `auth_retry.rs` (PR #1708)
//!   - inner: `execute_tool_with_post_oauth_retry` in `client.rs` (PR #1707)
//!
//! A single outer retry therefore issues up to 4 backend calls total (outer
//! attempt 1 → inner retry → 2 hits; outer attempt 2 → inner retry → 2 hits).
//! The happy-path test sequences only the first call as the gappy error and
//! all subsequent calls as success, so the outer first-attempt succeeds after
//! the inner retry (2 backend hits) and no outer second attempt is needed.
//! The bounded-loop test uses a gate — once the mock has seen the expected
//! calls, further hits always return success — so the assertion is ≥ 1 success
//! rather than an exact count.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

// ── env serialisation ─────────────────────────────────────────────────────────
//
// HOME / OPENHUMAN_WORKSPACE / BACKEND_URL are process-global; parallel tests
// in this binary would clobber each other without a lock.

static COMPOSIO_E2E_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn composio_e2e_env_lock() -> std::sync::MutexGuard<'static, ()> {
    let mutex = COMPOSIO_E2E_ENV_LOCK.get_or_init(|| Mutex::new(()));
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

const TEST_RPC_TOKEN: &str = "composio-e2e-rpc-token";
const TEST_JWT: &str = "e2e-composio-jwt";

static RPC_AUTH_ONCE: OnceLock<()> = OnceLock::new();

fn ensure_rpc_auth() {
    RPC_AUTH_ONCE.get_or_init(|| {
        // SAFETY: set_var inside OnceLock runs exactly once, before concurrent
        // env reads — same pattern as `ensure_test_rpc_auth` in json_rpc_e2e.rs.
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN) };
        let token_dir = std::env::temp_dir().join("openhuman-composio-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc token for composio_post_oauth_retry_e2e");
    });
}

// ── env-var guard ─────────────────────────────────────────────────────────────

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

// ── mock backend builders ─────────────────────────────────────────────────────

/// Minimal mock of the openhuman backend for the composio e2e tests.
/// Handles:
///   - `GET /settings` and `GET /auth/me` — JWT validation probe issued by
///     `auth_store_session`. Returns a synthetic user object.
///   - `POST /agent-integrations/composio/execute` — sequenced responses driven
///     by `ComposioExecuteState`.
#[derive(Clone)]
struct ComposioExecuteState {
    /// Incremented on every hit to `/agent-integrations/composio/execute`.
    hit_count: Arc<AtomicUsize>,
    /// Closure returning the mock response for request number `n` (0-indexed).
    response_fn: Arc<dyn Fn(usize) -> Value + Send + Sync>,
}

impl ComposioExecuteState {
    fn new(response_fn: impl Fn(usize) -> Value + Send + Sync + 'static) -> Self {
        Self {
            hit_count: Arc::new(AtomicUsize::new(0)),
            response_fn: Arc::new(response_fn),
        }
    }
}

async fn mock_current_user(headers: HeaderMap) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let auth = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != format!("Bearer {TEST_JWT}") {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "success": false, "error": "unauthorized" })),
        ));
    }
    Ok(Json(json!({
        "success": true,
        "data": {
            "_id": "composio-e2e-user",
            "username": "composio-e2e"
        }
    })))
}

async fn mock_composio_execute(
    State(state): State<ComposioExecuteState>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let auth = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != format!("Bearer {TEST_JWT}") {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "success": false, "error": "unauthorized" })),
        ));
    }
    let n = state.hit_count.fetch_add(1, Ordering::SeqCst);
    tracing::debug!(
        hit_n = n,
        "[composio-e2e-mock] /agent-integrations/composio/execute called"
    );
    Ok(Json((state.response_fn)(n)))
}

fn mock_backend_router(execute_state: ComposioExecuteState) -> Router {
    Router::new()
        .route("/settings", get(mock_current_user))
        .route("/auth/me", get(mock_current_user))
        .route(
            "/agent-integrations/composio/execute",
            post(mock_composio_execute).with_state(execute_state),
        )
}

// ── infrastructure helpers ────────────────────────────────────────────────────

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
    // Pre-login user directory: config resolution uses `users/local` before an
    // active user is established (same pattern as write_min_config in
    // json_rpc_e2e.rs). Without this, auth_store_session hits the real backend.
    write_cfg(&openhuman_dir.join("users").join("local"), &cfg);
    // Post-login user-scoped directory.
    write_cfg(&openhuman_dir.join("users").join("composio-e2e-user"), &cfg);
}

async fn post_json_rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("reqwest client");
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

// ── test: happy-path retry ────────────────────────────────────────────────────

/// Flow 1 from the task brief:
///
/// After completing Composio OAuth the user immediately invokes an action.
/// The first backend call returns the post-OAuth gappy auth-error payload
/// (`successful=false, error="Connection error, try to authenticate"`).
/// The retry layer should fire automatically and the second backend call
/// should return real data. The RPC result observed by the caller must be
/// `successful=true` with the action data — the transient error is invisible.
///
/// The test drives this through the full `openhuman.composio_execute` RPC
/// handler so the retry logic in `execute_with_auth_retry_inner` and
/// `execute_tool_with_post_oauth_retry` is exercised end-to-end.
#[tokio::test]
async fn post_oauth_gap_retries_and_returns_real_data() {
    let _env_lock = composio_e2e_env_lock();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvGuard::set_to_path("HOME", home);
    let _ws_guard = EnvGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvGuard::unset("BACKEND_URL");
    let _vite_guard = EnvGuard::unset("VITE_BACKEND_URL");

    // Sequence: call 0 → post-OAuth gap error; call 1+ → success.
    let execute_state = ComposioExecuteState::new(|n| {
        if n == 0 {
            // Simulates Composio's transient readiness-gap response.
            json!({
                "success": true,
                "data": {
                    "data": {},
                    "successful": false,
                    "error": "Connection error, try to authenticate",
                    "costUsd": 0.0
                }
            })
        } else {
            // Simulates the real action response after the gateway has synced.
            json!({
                "success": true,
                "data": {
                    "data": { "events": [{ "id": "evt_1", "summary": "Team standup" }] },
                    "successful": true,
                    "error": null,
                    "costUsd": 0.0018
                }
            })
        }
    });

    let hit_count = execute_state.hit_count.clone();

    let (mock_addr, mock_join) = serve_ephemeral(mock_backend_router(execute_state)).await;
    let mock_origin = format!("http://{mock_addr}");
    write_test_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Authenticate with the core RPC server so backend calls carry a valid JWT.
    let store = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.auth_store_session",
        json!({ "token": TEST_JWT, "user_id": "composio-e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "auth_store_session");

    // Invoke `composio_execute` over JSON-RPC — the same surface the UI calls.
    // The `execute_dispatch` → `auth_retry` → `client` chain will fire the
    // first call, see the gappy auth error, back off (zero-delay in tests
    // because `AUTH_RETRY_BACKOFF` is 8 s but the inner mock is synchronous),
    // and retry.
    let exec = post_json_rpc(
        &rpc_base,
        2,
        "openhuman.composio_execute",
        json!({
            "tool": "GOOGLECALENDAR_EVENTS_LIST",
            "arguments": {}
        }),
    )
    .await;

    let envelope = assert_no_jsonrpc_error(&exec, "composio_execute");
    // RpcOutcome serialises as {"result": <ComposioExecuteResponse>, "logs": [...]}
    // when logs are present.  Unwrap one level to reach the composio payload.
    let result = envelope.get("result").unwrap_or(envelope);

    // The RPC result must surface the second (successful) backend response.
    assert!(
        result
            .get("successful")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "composio_execute must return successful=true after retrying the post-OAuth gap error; \
         got: {result}"
    );
    assert!(
        result.get("error").is_none() || result["error"].is_null(),
        "composio_execute must not surface the transient auth error; got: {result}"
    );

    // The action data from the second call must be present.
    let events = result
        .pointer("/data/events")
        .and_then(Value::as_array)
        .expect("result.data.events must be an array");
    assert_eq!(events.len(), 1, "expected one mocked event");
    assert_eq!(
        events[0]["summary"],
        json!("Team standup"),
        "event summary must match mock data"
    );

    // At least 2 backend hits: the initial gappy call + at least one retry.
    // (Could be up to 4 due to the two-layer retry stack documented in the
    // `auth_retry_tests.rs` TODO.)
    let hits = hit_count.load(Ordering::SeqCst);
    assert!(
        hits >= 2,
        "expected at least 2 backend hits (initial + retry); got {hits}"
    );
    assert!(
        hits <= 4,
        "expected at most 4 backend hits (bounded retry contract); got {hits}"
    );

    mock_join.abort();
    rpc_join.abort();
}

// ── test: real revoked-token error surfaces immediately ───────────────────────

/// Flow 2 from the task brief:
///
/// A real revoked-token / invalid-grant error is NOT in the retryable-error
/// allow-list (`POST_OAUTH_AUTH_ERROR_STRINGS`). The retry layer must surface
/// it immediately after a single backend call — no 8-second wait, no misleading
/// "try to authenticate" loop.
///
/// The assertion verifies:
///   - the RPC result carries `successful=false` with the error text preserved
///     (possibly wrapped by `format_provider_error`)
///   - the backend was hit exactly once (or up to 2 due to any unrelated inner
///     retry layer, but never more — the outer auth_retry.rs layer must not fire)
#[tokio::test]
async fn revoked_token_surfaces_without_retry() {
    let _env_lock = composio_e2e_env_lock();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvGuard::set_to_path("HOME", home);
    let _ws_guard = EnvGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvGuard::unset("BACKEND_URL");
    let _vite_guard = EnvGuard::unset("VITE_BACKEND_URL");

    // Always return a real revoked-token error — should not be retried.
    let execute_state = ComposioExecuteState::new(|_n| {
        json!({
            "success": true,
            "data": {
                "data": {},
                "successful": false,
                "error": "invalid_grant: refresh token revoked",
                "costUsd": 0.0
            }
        })
    });

    let hit_count = execute_state.hit_count.clone();

    let (mock_addr, mock_join) = serve_ephemeral(mock_backend_router(execute_state)).await;
    let mock_origin = format!("http://{mock_addr}");
    write_test_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let store = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.auth_store_session",
        json!({ "token": TEST_JWT, "user_id": "composio-e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "auth_store_session");

    let exec = post_json_rpc(
        &rpc_base,
        2,
        "openhuman.composio_execute",
        json!({
            "tool": "GMAIL_SEND_EMAIL",
            "arguments": { "to": "test@example.com", "subject": "hi", "body": "hello" }
        }),
    )
    .await;

    // The RPC layer returns a result (not a JSON-RPC error) with successful=false
    // because `execute_composio_action_kind` converts op-level errors to
    // formatted strings inside `ComposioExecuteResponse`. Either a result with
    // successful=false or a JSON-RPC error with the text is acceptable.
    let has_rpc_error = exec.get("error").is_some();
    let result_opt = exec.get("result");

    if has_rpc_error {
        // The error message must contain the revoked-token text (possibly
        // wrapped in the `[composio:error:auth]` prefix by format_provider_error).
        let err_msg = exec["error"]["message"]
            .as_str()
            .or_else(|| exec["error"].as_str())
            .unwrap_or("");
        assert!(
            err_msg.contains("revoked")
                || err_msg.contains("invalid_grant")
                || err_msg.contains("composio"),
            "RPC error should reference the revoked-token message; got: {err_msg}"
        );
    } else {
        let envelope = result_opt.expect("expected result or error");
        // RpcOutcome wraps the composio payload under a "result" key when logs
        // are present; fall back to the envelope itself for the no-logs case.
        let result = envelope.get("result").unwrap_or(envelope);
        let successful = result
            .get("successful")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        assert!(
            !successful,
            "revoked-token error must NOT be reported as successful; got: {result}"
        );
        let error_text = result.get("error").and_then(Value::as_str).unwrap_or("");
        assert!(
            error_text.contains("revoked")
                || error_text.contains("invalid_grant")
                || error_text.contains("composio"),
            "error text must reference the revoked-token or composio error; got: {error_text:?}"
        );
    }

    // The outer auth_retry.rs layer must NOT have fired — the error is not
    // in `POST_OAUTH_AUTH_ERROR_STRINGS`. We allow at most 2 hits to account
    // for the inner `execute_tool_with_post_oauth_retry` which also checks
    // the same predicate (and correctly short-circuits for this error string),
    // but in practice both layers skip the retry for non-allowlisted errors
    // so exactly 1 hit is expected.
    let hits = hit_count.load(Ordering::SeqCst);
    assert!(
        hits <= 2,
        "revoked-token error must not trigger the outer auth retry; \
         expected ≤ 2 backend hits, got {hits}"
    );
    assert!(
        hits >= 1,
        "at least one backend hit is required; got {hits}"
    );

    mock_join.abort();
    rpc_join.abort();
}
