//! HTTP JSON-RPC integration tests against a real axum stack and a mock upstream API.
//!
//! Isolates config under a temp `HOME` so auth profiles and the OpenHuman provider resolve
//! the same state directory. Run with: `cargo test --test json_rpc_e2e`

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::cron::{add_shell_job, Schedule};
use openhuman_core::openhuman::skills::qjs_engine::RuntimeEngine;
use openhuman_core::openhuman::skills::set_global_engine;

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

    fn unset(key: &'static str) -> Self {
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

/// Serializes tests in this binary: `HOME` / `OPENHUMAN_WORKSPACE` / backend URL overrides are
/// process-global, so parallel tests would clobber each other and hit the wrong `config.toml` or
/// inherited `VITE_BACKEND_URL`.
static JSON_RPC_E2E_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn json_rpc_e2e_env_lock() -> std::sync::MutexGuard<'static, ()> {
    let mutex = JSON_RPC_E2E_ENV_LOCK.get_or_init(|| Mutex::new(()));
    // Recover from poison so that a panic in one test does not cascade to all others.
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn mock_upstream_router() -> Router {
    const GENERAL_TOKEN: &str = "e2e-test-jwt";
    const BILLING_TOKEN: &str = "e2e-billing-jwt";
    const TEAM_TOKEN: &str = "e2e-team-jwt";

    fn error_json(status: StatusCode, message: &str) -> (StatusCode, Json<Value>) {
        (
            status,
            Json(json!({
                "success": false,
                "error": message,
                "message": message,
            })),
        )
    }

    fn require_bearer(
        headers: &HeaderMap,
        expected_token: &str,
    ) -> Result<(), (StatusCode, Json<Value>)> {
        require_any_bearer(headers, &[expected_token])
    }

    fn require_any_bearer(
        headers: &HeaderMap,
        expected_tokens: &[&str],
    ) -> Result<(), (StatusCode, Json<Value>)> {
        let actual = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::trim);
        match actual {
            Some(value)
                if expected_tokens
                    .iter()
                    .any(|token| value == format!("Bearer {token}")) =>
            {
                Ok(())
            }
            Some(_) => Err(error_json(
                StatusCode::UNAUTHORIZED,
                "invalid Authorization bearer token",
            )),
            None => Err(error_json(
                StatusCode::UNAUTHORIZED,
                "missing Authorization bearer token",
            )),
        }
    }

    fn require_string_field<'a>(
        body: &'a Value,
        field: &str,
    ) -> Result<&'a str, (StatusCode, Json<Value>)> {
        body.get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                error_json(
                    StatusCode::BAD_REQUEST,
                    &format!("missing or invalid '{field}'"),
                )
            })
    }

    fn require_positive_f64_field(
        body: &Value,
        field: &str,
    ) -> Result<f64, (StatusCode, Json<Value>)> {
        body.get(field)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| {
                error_json(
                    StatusCode::BAD_REQUEST,
                    &format!("missing or invalid '{field}'"),
                )
            })
    }

    // Matches authenticated profile fetches used during session validation.
    async fn current_user(headers: HeaderMap) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_any_bearer(&headers, &[GENERAL_TOKEN, BILLING_TOKEN, TEAM_TOKEN])?;
        Ok(Json(json!({
            "success": true,
            "data": {
                "_id": "e2e-user-1",
                "username": "e2e"
            }
        })))
    }

    async fn chat_completions(Json(_body): Json<Value>) -> Json<Value> {
        Json(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hello from e2e mock agent"
                }
            }]
        }))
    }

    // ── Billing mock routes ──────────────────────────────────────────────────

    async fn stripe_current_plan(
        headers: HeaderMap,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_bearer(&headers, BILLING_TOKEN)?;
        Ok(Json(json!({
            "success": true,
            "data": {
                "plan": "PRO",
                "hasActiveSubscription": true,
                "planExpiry": "2030-01-01T00:00:00.000Z",
                "subscription": { "id": "sub_mock_123", "status": "active" }
            }
        })))
    }

    async fn stripe_purchase_plan(
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_bearer(&headers, BILLING_TOKEN)?;
        let plan = require_string_field(&body, "plan")?;
        if !matches!(plan, "basic" | "pro" | "BASIC" | "PRO") {
            return Err(error_json(
                StatusCode::BAD_REQUEST,
                "missing or invalid 'plan'",
            ));
        }

        let checkout_url = "http://127.0.0.1/mock-checkout";
        let session_id = "cs_mock_abc";
        if checkout_url.is_empty() || session_id.is_empty() {
            return Err(error_json(
                StatusCode::BAD_REQUEST,
                "missing checkoutUrl or sessionId",
            ));
        }

        Ok(Json(json!({
            "success": true,
            "data": { "checkoutUrl": checkout_url, "sessionId": session_id }
        })))
    }

    async fn stripe_portal(headers: HeaderMap) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_bearer(&headers, BILLING_TOKEN)?;
        let portal_url = "http://127.0.0.1/mock-portal";
        if portal_url.is_empty() {
            return Err(error_json(StatusCode::BAD_REQUEST, "missing portalUrl"));
        }

        Ok(Json(json!({
            "success": true,
            "data": { "portalUrl": portal_url }
        })))
    }

    async fn credits_top_up(
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_bearer(&headers, BILLING_TOKEN)?;
        let amount_usd = require_positive_f64_field(&body, "amountUsd")?;
        let gateway = require_string_field(&body, "gateway")?;
        if !matches!(gateway, "stripe" | "coinbase") {
            return Err(error_json(
                StatusCode::BAD_REQUEST,
                "missing or invalid 'gateway'",
            ));
        }

        Ok(Json(json!({
            "success": true,
            "data": {
                "url": "http://127.0.0.1/mock-topup",
                "gatewayTransactionId": "txn_mock_1",
                "amountUsd": amount_usd,
                "gateway": gateway
            }
        })))
    }

    async fn coinbase_charge(
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_bearer(&headers, BILLING_TOKEN)?;
        let plan = require_string_field(&body, "plan")?;
        let interval = body
            .get("interval")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("annual");
        if !matches!(plan, "basic" | "pro" | "BASIC" | "PRO") {
            return Err(error_json(
                StatusCode::BAD_REQUEST,
                "missing or invalid 'plan'",
            ));
        }
        if interval != "annual" {
            return Err(error_json(
                StatusCode::BAD_REQUEST,
                "missing or invalid 'interval'",
            ));
        }

        Ok(Json(json!({
            "success": true,
            "data": {
                "gatewayTransactionId": "coinbase_mock_1",
                "hostedUrl": "http://127.0.0.1/mock-coinbase",
                "status": "NEW",
                "expiresAt": "2030-01-01T01:00:00.000Z"
            }
        })))
    }

    // ── Team mock routes ─────────────────────────────────────────────────────

    async fn team_members(headers: HeaderMap) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_bearer(&headers, TEAM_TOKEN)?;
        Ok(Json(json!({
            "success": true,
            "data": [
                { "id": "user-1", "username": "alice", "role": "ADMIN" },
                { "id": "user-2", "username": "bob",   "role": "MEMBER" }
            ]
        })))
    }

    async fn team_invites_get(
        headers: HeaderMap,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_bearer(&headers, TEAM_TOKEN)?;
        Ok(Json(json!({
            "success": true,
            "data": [
                { "id": "inv-1", "code": "ALPHA1", "maxUses": 5, "usedCount": 1, "expiresAt": null }
            ]
        })))
    }

    async fn team_invites_post(
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_bearer(&headers, TEAM_TOKEN)?;

        let max_uses = body
            .get("maxUses")
            .and_then(Value::as_u64)
            .ok_or_else(|| error_json(StatusCode::BAD_REQUEST, "missing or invalid 'maxUses'"))?;
        let expires_in_days = body
            .get("expiresInDays")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                error_json(
                    StatusCode::BAD_REQUEST,
                    "missing or invalid 'expiresInDays'",
                )
            })?;
        if max_uses == 0 || expires_in_days == 0 {
            return Err(error_json(
                StatusCode::BAD_REQUEST,
                "invite payload values must be greater than zero",
            ));
        }

        Ok(Json(json!({
            "success": true,
            "data": { "id": "inv-new", "code": "NEWCODE", "maxUses": max_uses, "usedCount": 0, "expiresAt": null }
        })))
    }

    async fn team_member_delete(
        headers: HeaderMap,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_bearer(&headers, TEAM_TOKEN)?;
        Ok(Json(json!({ "success": true, "data": {} })))
    }

    async fn team_member_role_put(
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_bearer(&headers, TEAM_TOKEN)?;
        let role = require_string_field(&body, "role")?;
        if !matches!(role, "ADMIN" | "MEMBER" | "OWNER") {
            return Err(error_json(
                StatusCode::BAD_REQUEST,
                "missing or invalid 'role'",
            ));
        }
        Ok(Json(json!({ "success": true, "data": {} })))
    }

    async fn team_invite_delete(
        headers: HeaderMap,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_bearer(&headers, TEAM_TOKEN)?;
        Ok(Json(json!({ "success": true, "data": {} })))
    }

    Router::new()
        .route("/settings", get(current_user))
        .route("/auth/me", get(current_user))
        .route("/openai/v1/chat/completions", post(chat_completions))
        // billing
        .route("/payments/stripe/currentPlan", get(stripe_current_plan))
        .route("/payments/stripe/purchasePlan", post(stripe_purchase_plan))
        .route("/payments/stripe/portal", post(stripe_portal))
        .route("/payments/credits/top-up", post(credits_top_up))
        .route("/payments/coinbase/charge", post(coinbase_charge))
        // team
        .route("/teams/{team_id}/members", get(team_members))
        .route(
            "/teams/{team_id}/members/{user_id}",
            axum::routing::delete(team_member_delete),
        )
        .route(
            "/teams/{team_id}/members/{user_id}/role",
            axum::routing::put(team_member_role_put),
        )
        .route(
            "/teams/{team_id}/invites",
            get(team_invites_get).post(team_invites_post),
        )
        .route(
            "/teams/{team_id}/invites/{invite_id}",
            axum::routing::delete(team_invite_delete),
        )
}

async fn serve_on_ephemeral(
    app: Router,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, handle)
}

async fn post_json_rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("client");
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    });
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {url}: {e}"));
    assert!(
        resp.status().is_success(),
        "HTTP error {} for {}",
        resp.status(),
        method
    );
    resp.json::<Value>()
        .await
        .unwrap_or_else(|e| panic!("json for {method}: {e}"))
}

async fn read_first_sse_event(events_url: &str) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("client");
    let resp = client
        .get(events_url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {events_url}: {e}"));
    assert!(
        resp.status().is_success(),
        "SSE HTTP error {} for {}",
        resp.status(),
        events_url
    );

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    while let Some(item) = stream.next().await {
        let chunk = item.unwrap_or_else(|e| panic!("sse stream read failed: {e}"));
        let text = std::str::from_utf8(&chunk).unwrap_or("");
        buffer.push_str(text);
        while let Some(idx) = buffer.find("\n\n") {
            let block = buffer[..idx].to_string();
            buffer = buffer[idx + 2..].to_string();
            let mut data_lines = Vec::new();
            for line in block.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    data_lines.push(data.trim_start());
                }
            }
            if !data_lines.is_empty() {
                let payload = data_lines.join("\n");
                let value: Value = serde_json::from_str(&payload)
                    .unwrap_or_else(|e| panic!("invalid sse data json: {e}"));
                return value;
            }
        }
    }
    panic!("SSE stream ended before any event payload");
}

fn assert_no_jsonrpc_error<'a>(v: &'a Value, context: &str) -> &'a Value {
    if let Some(err) = v.get("error") {
        panic!("{context}: JSON-RPC error: {err}");
    }
    v.get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {v}"))
}

fn extract_string_outcome(result: &Value) -> String {
    if let Some(s) = result.as_str() {
        return s.to_string();
    }
    if let Some(inner) = result.get("result").and_then(Value::as_str) {
        return inner.to_string();
    }
    panic!("expected string or {{result: string}}, got {result}");
}

fn write_min_config(openhuman_dir: &Path, api_origin: &str) {
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "e2e-mock-model"
default_temperature = 0.7

[secrets]
encrypt = false
"#
    );
    std::fs::create_dir_all(openhuman_dir).expect("mkdir openhuman");
    let path = openhuman_dir.join("config.toml");
    std::fs::write(&path, &cfg).expect("write config");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(&cfg).expect("config toml must match Config schema");
}

#[cfg(target_os = "macos")]
fn write_min_config_with_local_ai_disabled(openhuman_dir: &Path, api_origin: &str) {
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "e2e-mock-model"
default_temperature = 0.7

[secrets]
encrypt = false

[local_ai]
enabled = false
"#
    );
    std::fs::create_dir_all(openhuman_dir).expect("mkdir openhuman");
    let path = openhuman_dir.join("config.toml");
    std::fs::write(&path, &cfg).expect("write config");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(&cfg).expect("config toml must match Config schema");
}

#[tokio::test]
async fn json_rpc_protocol_auth_and_agent_hello() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    // Always use the in-process Axum mock for /settings + /openai so this test does not pick up
    // BACKEND_URL/VITE_BACKEND_URL from the developer shell (e.g. mock-api that returns 401 for
    // the synthetic JWT used below).
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);

    write_min_config(&openhuman_home, &mock_origin);

    // Pre-create the user-scoped config directory so that when store_session
    // activates user "e2e-user" and reloads config, it finds the correct
    // api_url and secrets.encrypt=false (rather than defaults).
    let user_scoped_dir = openhuman_home.join("users").join("e2e-user");
    write_min_config(&user_scoped_dir, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // --- core.ping (baseline protocol) ---
    let ping = post_json_rpc(&rpc_base, 1, "core.ping", json!({})).await;
    let ping_result = assert_no_jsonrpc_error(&ping, "core.ping");
    assert_eq!(ping_result.get("ok"), Some(&json!(true)));

    // --- unknown method ---
    let unknown = post_json_rpc(&rpc_base, 2, "core.not_a_real_method", json!({})).await;
    assert!(
        unknown.get("error").is_some(),
        "expected error for unknown method: {unknown}"
    );

    // --- auth: session state (no JWT yet) ---
    let state_before = post_json_rpc(&rpc_base, 3, "openhuman.auth_get_state", json!({})).await;
    let state_outer = assert_no_jsonrpc_error(&state_before, "get_state");
    let state_body = state_outer.get("result").unwrap_or(state_outer);
    assert!(
        state_body.get("isAuthenticated").is_some() || state_body.get("is_authenticated").is_some(),
        "unexpected auth state shape: {state_body}"
    );

    // --- auth: store session (validates JWT via mock GET /auth/me) ---
    let store = post_json_rpc(
        &rpc_base,
        4,
        "openhuman.auth_store_session",
        json!({
            "token": "e2e-test-jwt",
            "user_id": "e2e-user"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    // --- agent: single chat turn (mock chat completions) ---
    let chat = post_json_rpc(
        &rpc_base,
        5,
        "openhuman.local_ai_agent_chat",
        json!({
            "message": "Hello",
        }),
    )
    .await;
    let chat_result = assert_no_jsonrpc_error(&chat, "agent_chat");
    let reply = extract_string_outcome(chat_result);
    assert!(
        reply.contains("e2e mock") || reply.contains("Hello"),
        "unexpected agent reply: {reply:?}"
    );

    // --- web channel RPC + SSE loop ---
    let client_id = "e2e-client-1";
    let thread_id = "thread-1";
    let events_url = format!("{}/events?client_id={}", rpc_base, client_id);
    let sse_task = tokio::spawn(async move { read_first_sse_event(&events_url).await });

    let web_chat = post_json_rpc(
        &rpc_base,
        6,
        "openhuman.channel_web_chat",
        json!({
            "client_id": client_id,
            "thread_id": thread_id,
            "message": "Hello from web channel",
            "model_override": "e2e-mock-model",
        }),
    )
    .await;
    let web_chat_result = assert_no_jsonrpc_error(&web_chat, "channel_web_chat");
    assert_eq!(
        web_chat_result
            .get("result")
            .and_then(|v| v.get("accepted")),
        Some(&json!(true))
    );

    let sse_event = sse_task.await.expect("sse task join should succeed");
    assert_eq!(
        sse_event.get("event").and_then(Value::as_str),
        Some("chat_done")
    );
    assert_eq!(
        sse_event.get("thread_id").and_then(Value::as_str),
        Some(thread_id)
    );
    assert!(
        sse_event
            .get("full_response")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .len()
            > 0,
        "expected non-empty chat_done response payload: {sse_event}"
    );

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_rejects_non_object_params_with_clear_error() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let invalid = post_json_rpc(
        &rpc_base,
        1001,
        "openhuman.auth_get_state",
        json!(["invalid", "params"]),
    )
    .await;
    let err_message = invalid
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        !err_message.is_empty(),
        "expected non-empty JSON-RPC error message: {invalid}"
    );

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_screen_intelligence_capture_test_returns_stable_shape() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let capture = post_json_rpc(
        &rpc_base,
        1002,
        "openhuman.screen_intelligence_capture_test",
        json!({}),
    )
    .await;
    let capture_outer = assert_no_jsonrpc_error(&capture, "screen_intelligence_capture_test");
    let capture_result = capture_outer.get("result").unwrap_or(capture_outer);

    assert!(
        capture_result.get("ok").and_then(Value::as_bool).is_some(),
        "expected bool ok field: {capture_result}"
    );
    assert!(
        matches!(
            capture_result.get("capture_mode").and_then(Value::as_str),
            Some("windowed" | "fullscreen")
        ),
        "expected capture_mode field: {capture_result}"
    );
    assert!(
        capture_result
            .get("timing_ms")
            .and_then(Value::as_u64)
            .is_some(),
        "expected timing_ms field: {capture_result}"
    );

    let ok = capture_result
        .get("ok")
        .and_then(Value::as_bool)
        .expect("ok should be bool");
    let image_ref = capture_result.get("image_ref").and_then(Value::as_str);
    let error = capture_result.get("error").and_then(Value::as_str);

    if ok {
        assert!(
            image_ref
                .map(|value| value.starts_with("data:image/png;base64,"))
                .unwrap_or(false),
            "successful capture should include a PNG data URL: {capture_result}"
        );
        assert!(
            error.is_none(),
            "successful capture should not include an error"
        );
    } else {
        assert!(
            image_ref.is_none(),
            "failed capture should not include image data"
        );
        assert!(
            error.is_some(),
            "failed capture should include an error message"
        );
    }

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_screen_intelligence_status_returns_stable_shape() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let status = post_json_rpc(
        &rpc_base,
        1003,
        "openhuman.screen_intelligence_status",
        json!({}),
    )
    .await;
    let result = assert_no_jsonrpc_error(&status, "screen_intelligence_status");
    let status_result = result.get("result").unwrap_or(result);

    // Required top-level fields
    assert!(
        status_result
            .get("platform_supported")
            .and_then(Value::as_bool)
            .is_some(),
        "expected bool platform_supported: {status_result}"
    );
    assert!(
        status_result
            .get("is_context_blocked")
            .and_then(Value::as_bool)
            .is_some(),
        "expected bool is_context_blocked: {status_result}"
    );

    // session block
    let session = status_result
        .get("session")
        .expect("expected session object");
    assert!(
        session.get("active").and_then(Value::as_bool).is_some(),
        "expected bool session.active: {status_result}"
    );
    assert_eq!(
        session.get("active").and_then(Value::as_bool),
        Some(false),
        "session should not be active without start_session: {status_result}"
    );
    assert!(
        session
            .get("capture_count")
            .and_then(Value::as_u64)
            .is_some(),
        "expected u64 session.capture_count: {status_result}"
    );
    assert!(
        session
            .get("vision_persist_count")
            .and_then(Value::as_u64)
            .is_some(),
        "expected u64 session.vision_persist_count: {status_result}"
    );
    assert!(
        session.get("last_vision_persist_error").is_some(),
        "expected nullable session.last_vision_persist_error: {status_result}"
    );

    // permissions block
    let perms = status_result
        .get("permissions")
        .expect("expected permissions object");
    assert!(
        perms
            .get("screen_recording")
            .and_then(Value::as_str)
            .is_some(),
        "expected string permissions.screen_recording: {status_result}"
    );

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_screen_intelligence_vision_recent_returns_empty_without_session() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let recent = post_json_rpc(
        &rpc_base,
        1004,
        "openhuman.screen_intelligence_vision_recent",
        json!({ "limit": 10 }),
    )
    .await;
    let result = assert_no_jsonrpc_error(&recent, "screen_intelligence_vision_recent");
    let recent_result = result.get("result").unwrap_or(result);

    let summaries = recent_result
        .get("summaries")
        .and_then(Value::as_array)
        .expect("expected summaries array: {recent_result}");
    assert!(
        summaries.is_empty(),
        "vision_recent should return empty list without an active session, got {} items",
        summaries.len()
    );

    mock_join.abort();
    rpc_join.abort();
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn json_rpc_autocomplete_runtime_settings_and_logs_flow() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config_with_local_ai_disabled(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let set_style = post_json_rpc(
        &rpc_base,
        2001,
        "openhuman.autocomplete_set_style",
        json!({
            "enabled": true,
            "debounce_ms": 180,
            "max_chars": 160,
            "accept_with_tab": false,
            "style_preset": "balanced",
            "style_examples": ["[mail] ...Can you share an update? → Can you share a quick update?"],
            "disabled_apps": []
        }),
    )
    .await;
    let set_style_outer = assert_no_jsonrpc_error(&set_style, "autocomplete_set_style");
    let set_style_payload = set_style_outer.get("result").unwrap_or(set_style_outer);
    let set_style_logs = set_style_outer
        .get("logs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        set_style_payload
            .get("config")
            .and_then(|v| v.get("debounce_ms"))
            .and_then(Value::as_u64),
        Some(180)
    );
    assert_eq!(
        set_style_payload
            .get("config")
            .and_then(|v| v.get("max_chars"))
            .and_then(Value::as_u64),
        Some(160)
    );
    assert!(
        set_style_logs.iter().any(|entry| {
            entry
                .as_str()
                .map(|s| s.contains("[autocomplete] set_style"))
                .unwrap_or(false)
        }),
        "expected structured set_style log line: {set_style_outer}"
    );

    let cfg = post_json_rpc(&rpc_base, 2002, "openhuman.config_get", json!({})).await;
    let cfg_outer = assert_no_jsonrpc_error(&cfg, "get_config");
    let cfg_payload = cfg_outer.get("result").unwrap_or(cfg_outer);
    let cfg_autocomplete = cfg_payload
        .get("config")
        .and_then(|v| v.get("autocomplete"))
        .expect("autocomplete config should exist");
    assert_eq!(
        cfg_autocomplete.get("debounce_ms").and_then(Value::as_u64),
        Some(180)
    );
    assert_eq!(
        cfg_autocomplete.get("max_chars").and_then(Value::as_u64),
        Some(160)
    );
    assert_eq!(
        cfg_autocomplete
            .get("accept_with_tab")
            .and_then(Value::as_bool),
        Some(false)
    );

    let start = post_json_rpc(
        &rpc_base,
        2003,
        "openhuman.autocomplete_start",
        json!({ "debounce_ms": 180 }),
    )
    .await;
    let start_outer = assert_no_jsonrpc_error(&start, "autocomplete_start");
    let start_logs = start_outer
        .get("logs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        start_logs.iter().any(|entry| {
            entry
                .as_str()
                .map(|s| s.contains("[autocomplete] start"))
                .unwrap_or(false)
        }),
        "expected structured start log line: {start_outer}"
    );

    let status_running =
        post_json_rpc(&rpc_base, 2004, "openhuman.autocomplete_status", json!({})).await;
    let status_running_outer = assert_no_jsonrpc_error(&status_running, "autocomplete_status");
    let status_running_payload = status_running_outer
        .get("result")
        .unwrap_or(status_running_outer);
    assert_eq!(
        status_running_payload
            .get("running")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        status_running_payload
            .get("enabled")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        status_running_payload
            .get("debounce_ms")
            .and_then(Value::as_u64),
        Some(180)
    );

    let current = post_json_rpc(
        &rpc_base,
        2005,
        "openhuman.autocomplete_current",
        json!({ "context": "Please review this changeset and" }),
    )
    .await;
    let current_outer = assert_no_jsonrpc_error(&current, "autocomplete_current");
    let current_payload = current_outer.get("result").unwrap_or(current_outer);
    let current_logs = current_outer
        .get("logs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        current_payload.get("context").and_then(Value::as_str),
        Some("Please review this changeset and")
    );
    assert!(
        current_logs.iter().any(|entry| {
            entry
                .as_str()
                .map(|s| s.contains("[autocomplete] current"))
                .unwrap_or(false)
        }),
        "expected structured current log line: {current_outer}"
    );

    let accept = post_json_rpc(
        &rpc_base,
        2006,
        "openhuman.autocomplete_accept",
        json!({
            "suggestion": " share your thoughts.",
            "skip_apply": true
        }),
    )
    .await;
    let accept_outer = assert_no_jsonrpc_error(&accept, "autocomplete_accept");
    let accept_payload = accept_outer.get("result").unwrap_or(accept_outer);
    let accept_logs = accept_outer
        .get("logs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        accept_payload.get("accepted").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        accept_payload.get("applied").and_then(Value::as_bool),
        Some(false)
    );
    assert!(
        accept_logs.iter().any(|entry| {
            entry
                .as_str()
                .map(|s| s.contains("[autocomplete] accept"))
                .unwrap_or(false)
        }),
        "expected structured accept log line: {accept_outer}"
    );

    let stop = post_json_rpc(
        &rpc_base,
        2007,
        "openhuman.autocomplete_stop",
        json!({ "reason": "json_rpc_e2e" }),
    )
    .await;
    let stop_outer = assert_no_jsonrpc_error(&stop, "autocomplete_stop");
    let stop_payload = stop_outer.get("result").unwrap_or(stop_outer);
    assert_eq!(
        stop_payload.get("stopped").and_then(Value::as_bool),
        Some(true)
    );

    let status_stopped =
        post_json_rpc(&rpc_base, 2008, "openhuman.autocomplete_status", json!({})).await;
    let status_stopped_outer = assert_no_jsonrpc_error(&status_stopped, "autocomplete_status");
    let status_stopped_payload = status_stopped_outer
        .get("result")
        .unwrap_or(status_stopped_outer);
    assert_eq!(
        status_stopped_payload
            .get("running")
            .and_then(Value::as_bool),
        Some(false)
    );

    mock_join.abort();
    rpc_join.abort();
}

// ---------------------------------------------------------------------------
// Skills registry E2E: fetch, search, install, list, uninstall
// ---------------------------------------------------------------------------

fn mock_skills_registry_router() -> Router {
    let manifest_json = json!({
        "id": "test-skill",
        "name": "Test Skill",
        "version": "1.0.0",
        "description": "A test skill for E2E",
        "runtime": "quickjs",
        "entry": "index.js"
    });
    let js_content = "function init() { console.log('test-skill'); }";

    // Compute checksum for the JS content
    let mut hasher = Sha256::new();
    hasher.update(js_content.as_bytes());
    let checksum = format!("{:x}", hasher.finalize());

    let registry = json!({
        "version": 1,
        "generated_at": "2026-03-30T12:00:00Z",
        "skills": {
            "core": [{
                "id": "test-skill",
                "name": "Test Skill",
                "version": "1.0.0",
                "description": "A test skill for E2E",
                "runtime": "quickjs",
                "entry": "index.js",
                "auto_start": false,
                "download_url": "__BASE__/skills/test-skill/index.js",
                "manifest_url": "__BASE__/skills/test-skill/manifest.json",
                "checksum_sha256": checksum
            }, {
                "id": "another-skill",
                "name": "Another Skill",
                "version": "2.0.0",
                "description": "Another skill for search testing",
                "runtime": "quickjs",
                "entry": "index.js",
                "download_url": "__BASE__/skills/another-skill/index.js",
                "manifest_url": "__BASE__/skills/another-skill/manifest.json"
            }],
            "third_party": []
        }
    });

    Router::new()
        .route(
            "/registry.json",
            get(move || {
                let r = registry.clone();
                async move { Json(r) }
            }),
        )
        .route(
            "/skills/test-skill/manifest.json",
            get(move || {
                let m = manifest_json.clone();
                async move { Json(m) }
            }),
        )
        .route(
            "/skills/test-skill/index.js",
            get(move || async move { js_content }),
        )
}

#[tokio::test]
async fn json_rpc_skills_registry_install_uninstall() {
    let _env_lock = json_rpc_e2e_env_lock();
    // 1. Setup: temp workspace, mock skills server, RPC server
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let workspace = openhuman_home.join("workspace");
    std::fs::create_dir_all(workspace.join("skills")).expect("create skills dir");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);

    // Start mock skills server
    let (skills_addr, skills_join) = serve_on_ephemeral(mock_skills_registry_router()).await;
    let skills_base = format!("http://{}", skills_addr);

    // Point registry URL at mock server and fix the __BASE__ placeholder
    let registry_url = format!("{}/registry.json", skills_base);
    let _registry_url_guard =
        EnvVarGuard::set_to_path("SKILLS_REGISTRY_URL", Path::new(&registry_url));

    // Also need a mock upstream for config loading
    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    // Start core RPC server
    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Sanity check
    let ping = post_json_rpc(&rpc_base, 1, "core.ping", json!({})).await;
    assert_no_jsonrpc_error(&ping, "core.ping");

    // Pre-populate the registry cache with correct URLs pointing at mock server.
    let cache_dir = workspace.join("skills");
    let now_rfc3339 = chrono::Utc::now().to_rfc3339();
    let js_content_bytes = b"function init() { console.log('test-skill'); }";
    let mut h = Sha256::new();
    h.update(js_content_bytes);
    let js_checksum = format!("{:x}", h.finalize());
    let dl_url = format!("{}/skills/test-skill/index.js", skills_base);
    let mf_url = format!("{}/skills/test-skill/manifest.json", skills_base);
    let dl_url2 = format!("{}/skills/another-skill/index.js", skills_base);
    let mf_url2 = format!("{}/skills/another-skill/manifest.json", skills_base);

    let registry_with_urls = json!({
        "fetched_at": now_rfc3339,
        "registry": {
            "version": 1,
            "generated_at": "2026-03-30T12:00:00Z",
            "skills": {
                "core": [{
                    "id": "test-skill",
                    "name": "Test Skill",
                    "version": "1.0.0",
                    "description": "A test skill for E2E",
                    "runtime": "quickjs",
                    "entry": "index.js",
                    "auto_start": false,
                    "download_url": dl_url,
                    "manifest_url": mf_url,
                    "checksum_sha256": js_checksum
                }, {
                    "id": "another-skill",
                    "name": "Another Skill",
                    "version": "2.0.0",
                    "description": "Another skill for search testing",
                    "runtime": "quickjs",
                    "entry": "index.js",
                    "download_url": dl_url2,
                    "manifest_url": mf_url2
                }],
                "third_party": []
            }
        }
    });
    std::fs::write(
        cache_dir.join(".registry-cache.json"),
        serde_json::to_string_pretty(&registry_with_urls).unwrap(),
    )
    .expect("write cache");

    // 2. skills_list_installed — should be empty initially
    let list = post_json_rpc(&rpc_base, 10, "openhuman.skills_list_installed", json!({})).await;
    let list_result = assert_no_jsonrpc_error(&list, "list_installed");
    assert!(
        list_result.as_array().unwrap().is_empty(),
        "expected empty installed list"
    );

    // 3. skills_search — find "test-skill"
    let search = post_json_rpc(
        &rpc_base,
        11,
        "openhuman.skills_search",
        json!({"query": "test"}),
    )
    .await;
    let search_result = assert_no_jsonrpc_error(&search, "search");
    let search_arr = search_result.as_array().expect("search result is array");
    assert!(
        search_arr.iter().any(|e| e["id"] == "test-skill"),
        "expected test-skill in search results: {search_result}"
    );

    // 4. skills_install — install test-skill
    let install = post_json_rpc(
        &rpc_base,
        12,
        "openhuman.skills_install",
        json!({"skill_id": "test-skill"}),
    )
    .await;
    let install_result = assert_no_jsonrpc_error(&install, "install");
    assert_eq!(install_result["success"], true);
    assert_eq!(install_result["skill_id"], "test-skill");

    // 5. Verify files exist on disk
    let installed_manifest = workspace.join("skills/test-skill/manifest.json");
    let installed_js = workspace.join("skills/test-skill/index.js");
    assert!(
        installed_manifest.exists(),
        "manifest.json should exist after install"
    );
    assert!(installed_js.exists(), "index.js should exist after install");

    // 6. skills_list_installed — should now show test-skill
    let list2 = post_json_rpc(&rpc_base, 13, "openhuman.skills_list_installed", json!({})).await;
    let list2_result = assert_no_jsonrpc_error(&list2, "list_installed_after");
    let list2_arr = list2_result.as_array().expect("list result is array");
    assert_eq!(list2_arr.len(), 1);
    assert_eq!(list2_arr[0]["id"], "test-skill");

    // 7. skills_list_available — test-skill should show installed=true
    let available =
        post_json_rpc(&rpc_base, 14, "openhuman.skills_list_available", json!({})).await;
    let available_result = assert_no_jsonrpc_error(&available, "list_available");
    let available_arr = available_result
        .as_array()
        .expect("available result is array");
    let test_entry = available_arr
        .iter()
        .find(|e| e["id"] == "test-skill")
        .expect("test-skill should be in available list");
    assert_eq!(test_entry["installed"], true);
    assert_eq!(test_entry["update_available"], false);

    // 8. skills_uninstall — remove test-skill
    let uninstall = post_json_rpc(
        &rpc_base,
        15,
        "openhuman.skills_uninstall",
        json!({"skill_id": "test-skill"}),
    )
    .await;
    let uninstall_result = assert_no_jsonrpc_error(&uninstall, "uninstall");
    assert_eq!(uninstall_result["success"], true);

    // 9. Verify directory removed
    assert!(
        !installed_manifest.exists(),
        "manifest should be gone after uninstall"
    );
    assert!(
        !installed_js.exists(),
        "index.js should be gone after uninstall"
    );

    // 10. skills_list_installed — should be empty again
    let list3 = post_json_rpc(&rpc_base, 16, "openhuman.skills_list_installed", json!({})).await;
    let list3_result = assert_no_jsonrpc_error(&list3, "list_installed_final");
    assert!(
        list3_result.as_array().unwrap().is_empty(),
        "should be empty after uninstall"
    );

    skills_join.abort();
    mock_join.abort();
    rpc_join.abort();
}

// ---------------------------------------------------------------------------
// Skills runtime E2E: start, list_tools, call_tool, sync, stop
// ---------------------------------------------------------------------------

/// Create a minimal QuickJS skill on disk with one tool.
fn write_test_skill(workspace: &Path, skill_id: &str) {
    let skill_dir = workspace.join("skills").join(skill_id);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");

    let manifest = json!({
        "id": skill_id,
        "name": "E2E Runtime Skill",
        "version": "1.0.0",
        "description": "Minimal skill for runtime E2E tests",
        "runtime": "quickjs",
        "entry": "index.js",
        "auto_start": false
    });
    std::fs::write(
        skill_dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .expect("write manifest");

    // Minimal JS skill that exports one tool: "echo" and a deterministic onSync
    // payload so we can assert sync → working-memory extraction end to end.
    let js = r#"
        globalThis.__skill = {
            name: "E2E Runtime Skill",
            tools: [
                {
                    name: "echo",
                    description: "Echoes back the input message",
                    inputSchema: {
                        type: "object",
                        properties: {
                            message: { type: "string", description: "Message to echo" }
                        },
                        required: ["message"]
                    },
                    execute(args) {
                        return { type: "text", text: "echo: " + (args.message || "empty") };
                    }
                }
            ]
        };

        function init() {
            if (globalThis.__ops && globalThis.__ops.log) {
                globalThis.__ops.log("info", "e2e-runtime-skill initialized");
            }
        }

        async function onSync() {
            if (globalThis.state && typeof globalThis.state.set === "function") {
                globalThis.state.set("sync_payload", {
                    preferences: { writing_style: "prefers concise updates", language: "English" },
                    goals: ["Ship e2e integration"],
                    constraints: ["No meetings after 3pm"],
                    projects: [{ name: "Atlas" }]
                });
            }
            return { status: "ok", synced: true };
        }

        init();
    "#;
    std::fs::write(skill_dir.join("index.js"), js).expect("write index.js");
}

#[tokio::test]
async fn json_rpc_skills_runtime_start_tools_call_stop() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let workspace = openhuman_home.join("workspace");
    std::fs::create_dir_all(workspace.join("skills")).expect("create skills dir");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);
    // Ensure working-memory extraction is not disabled by an ambient env var so
    // the assertions below are deterministic regardless of the host environment.
    let _wm_guard = EnvVarGuard::unset("OPENHUMAN_SKILLS_WORKING_MEMORY_ENABLED");

    // Write a minimal skill to the workspace
    write_test_skill(&workspace, "e2e-runtime");

    // Mock upstream for config loading
    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    // Initialize and set the global RuntimeEngine
    let skills_data_dir = workspace.join("skills_data");
    std::fs::create_dir_all(&skills_data_dir).expect("create skills_data dir");
    let engine =
        std::sync::Arc::new(RuntimeEngine::new(skills_data_dir).expect("create RuntimeEngine"));
    engine.set_workspace_dir(workspace.clone());
    set_global_engine(engine);

    // Start RPC server
    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Sanity check
    let ping = post_json_rpc(&rpc_base, 1, "core.ping", json!({})).await;
    assert_no_jsonrpc_error(&ping, "core.ping");

    // 1. Start the skill
    let start = post_json_rpc(
        &rpc_base,
        20,
        "openhuman.skills_start",
        json!({"skill_id": "e2e-runtime"}),
    )
    .await;
    let start_result = assert_no_jsonrpc_error(&start, "skills_start");
    assert_eq!(
        start_result.get("skill_id").and_then(Value::as_str),
        Some("e2e-runtime"),
        "start should return correct skill_id: {start_result}"
    );
    let status_str = start_result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        status_str == "running" || status_str == "initializing",
        "skill should be running or initializing after start, got: {status_str}"
    );

    // Give the skill a moment to finish initializing
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 2. Get skill status
    let status = post_json_rpc(
        &rpc_base,
        21,
        "openhuman.skills_status",
        json!({"skill_id": "e2e-runtime"}),
    )
    .await;
    let status_result = assert_no_jsonrpc_error(&status, "skills_status");
    assert_eq!(
        status_result.get("skill_id").and_then(Value::as_str),
        Some("e2e-runtime")
    );

    let data_stats = post_json_rpc(
        &rpc_base,
        211,
        "openhuman.skills_data_stats",
        json!({"skill_id": "e2e-runtime"}),
    )
    .await;
    let ds = assert_no_jsonrpc_error(&data_stats, "skills_data_stats");
    assert_eq!(ds.get("exists"), Some(&json!(true)));
    assert!(ds.get("path").and_then(Value::as_str).is_some());
    assert!(ds.get("total_bytes").and_then(Value::as_u64).is_some());
    assert!(ds.get("file_count").and_then(Value::as_u64).is_some());

    // 3. List tools
    let tools = post_json_rpc(
        &rpc_base,
        22,
        "openhuman.skills_list_tools",
        json!({"skill_id": "e2e-runtime"}),
    )
    .await;
    let tools_result = assert_no_jsonrpc_error(&tools, "skills_list_tools");
    let tools_arr = tools_result
        .get("tools")
        .and_then(Value::as_array)
        .expect("tools should be an array");
    assert!(
        !tools_arr.is_empty(),
        "skill should expose at least one tool: {tools_result}"
    );
    let has_echo = tools_arr
        .iter()
        .any(|t| t.get("name").and_then(Value::as_str) == Some("echo"));
    assert!(has_echo, "should have 'echo' tool: {tools_result}");

    // 4. Call the echo tool
    let call = post_json_rpc(
        &rpc_base,
        23,
        "openhuman.skills_call_tool",
        json!({
            "skill_id": "e2e-runtime",
            "tool_name": "echo",
            "arguments": { "message": "hello from e2e" }
        }),
    )
    .await;
    let call_result = assert_no_jsonrpc_error(&call, "skills_call_tool");
    // Tool result has content array with text blocks
    let empty = vec![];
    let content = call_result
        .get("content")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let has_echo_text = content.iter().any(|c| {
        c.get("text")
            .and_then(Value::as_str)
            .map(|t| t.contains("hello from e2e"))
            .unwrap_or(false)
    });
    assert!(
        has_echo_text,
        "echo tool should return the message: {call_result}"
    );

    // 5. Trigger sync (routes to onSync via skill/sync RPC)
    let sync = post_json_rpc(
        &rpc_base,
        24,
        "openhuman.skills_sync",
        json!({"skill_id": "e2e-runtime"}),
    )
    .await;
    let _sync_result = assert_no_jsonrpc_error(&sync, "skills_sync");

    // 5a. Poll until the async memory worker has written working-memory docs into
    // the global namespace, instead of relying on a fixed sleep.
    let poll_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let (docs_result, docs_arr) = loop {
        let docs = post_json_rpc(
            &rpc_base,
            241,
            "openhuman.memory_list_documents",
            json!({"namespace":"global"}),
        )
        .await;
        let arr = {
            let result = assert_no_jsonrpc_error(&docs, "memory_list_documents");
            result
                .get("documents")
                .or_else(|| result.get("data").and_then(|d| d.get("documents")))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let has_summary = arr.iter().any(|doc| {
            doc.get("key").and_then(Value::as_str) == Some("working.user.e2e-runtime.summary")
        });
        if has_summary {
            break (docs, arr);
        }
        assert!(
            tokio::time::Instant::now() < poll_deadline,
            "Timeout waiting for working.user.e2e-runtime.summary to appear. docs={docs}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    let wm_keys: Vec<String> = docs_arr
        .iter()
        .filter_map(|doc| doc.get("key").and_then(Value::as_str))
        .filter(|key| key.starts_with("working.user.e2e-runtime."))
        .map(ToString::to_string)
        .collect();

    assert!(
        !wm_keys.is_empty(),
        "Expected working memory docs after skills_sync, found none. docs={docs_result}"
    );
    assert!(
        wm_keys
            .iter()
            .any(|key| key == "working.user.e2e-runtime.summary"),
        "Expected summary working-memory key. keys={wm_keys:?}"
    );

    // 6. Stop the skill
    let stop = post_json_rpc(
        &rpc_base,
        25,
        "openhuman.skills_stop",
        json!({"skill_id": "e2e-runtime"}),
    )
    .await;
    let stop_result = assert_no_jsonrpc_error(&stop, "skills_stop");
    assert_eq!(stop_result.get("success"), Some(&json!(true)));

    mock_join.abort();
    rpc_join.abort();
}

// ---------------------------------------------------------------------------
// Local AI device profile, presets, and apply preset
// ---------------------------------------------------------------------------

#[tokio::test]
async fn json_rpc_local_ai_device_profile_and_presets() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _tier_guard = EnvVarGuard::unset("OPENHUMAN_LOCAL_AI_TIER");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // --- device_profile ---
    let profile = post_json_rpc(
        &rpc_base,
        30,
        "openhuman.local_ai_device_profile",
        json!({}),
    )
    .await;
    let profile_result = assert_no_jsonrpc_error(&profile, "device_profile");
    assert!(
        profile_result
            .get("total_ram_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0,
        "expected positive RAM: {profile_result}"
    );
    assert!(
        profile_result
            .get("cpu_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0,
        "expected positive CPU count: {profile_result}"
    );

    // --- presets ---
    let presets = post_json_rpc(&rpc_base, 31, "openhuman.local_ai_presets", json!({})).await;
    let presets_result = assert_no_jsonrpc_error(&presets, "presets");
    let presets_arr = presets_result
        .get("presets")
        .and_then(Value::as_array)
        .expect("presets should be an array");
    assert_eq!(presets_arr.len(), 5, "expected 5 presets: {presets_result}");

    let recommended = presets_result
        .get("recommended_tier")
        .and_then(Value::as_str)
        .expect("should have recommended_tier");
    assert!(
        [
            "ram_1gb",
            "ram_2_4gb",
            "ram_4_8gb",
            "ram_8_16gb",
            "ram_16_plus_gb",
        ]
        .contains(&recommended),
        "unexpected recommended_tier: {recommended}"
    );

    let current = presets_result
        .get("current_tier")
        .and_then(Value::as_str)
        .expect("should have current_tier");
    // Default config uses gemma3:4b-it-qat which now maps to the 8-16 GB tier.
    assert_eq!(
        current, "ram_8_16gb",
        "default config should be the 8-16 GB tier"
    );

    // --- apply_preset (switch to 2-4 GB) ---
    let apply = post_json_rpc(
        &rpc_base,
        32,
        "openhuman.local_ai_apply_preset",
        json!({"tier": "ram_2_4gb"}),
    )
    .await;
    let apply_result = assert_no_jsonrpc_error(&apply, "apply_preset");
    assert_eq!(
        apply_result.get("applied_tier").and_then(Value::as_str),
        Some("ram_2_4gb")
    );
    assert_eq!(
        apply_result.get("chat_model_id").and_then(Value::as_str),
        Some("gemma3:1b-it-qat")
    );
    assert_eq!(
        apply_result.get("vision_mode").and_then(Value::as_str),
        Some("disabled")
    );

    // --- verify presets reflects the change ---
    let presets_after = post_json_rpc(&rpc_base, 33, "openhuman.local_ai_presets", json!({})).await;
    let presets_after_result = assert_no_jsonrpc_error(&presets_after, "presets_after");
    assert_eq!(
        presets_after_result
            .get("current_tier")
            .and_then(Value::as_str),
        Some("ram_2_4gb"),
        "current tier should now be 2-4 GB after apply"
    );

    // --- apply_preset with invalid tier should error ---
    let bad_apply = post_json_rpc(
        &rpc_base,
        34,
        "openhuman.local_ai_apply_preset",
        json!({"tier": "ultra"}),
    )
    .await;
    assert!(
        bad_apply.get("error").is_some(),
        "expected error for invalid tier: {bad_apply}"
    );

    mock_join.abort();
    rpc_join.abort();
}

// ── Billing & Team E2E tests ──────────────────────────────────────────────────

/// End-to-end test for billing RPC methods.
///
/// Spins up an in-process Axum mock backend and a real JSON-RPC server, stores a
/// session JWT, then exercises every billing controller through the RPC surface
/// exactly as the desktop app or a CI script would.
#[tokio::test]
async fn billing_rpc_e2e() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    // Pre-create the user-scoped config so store_session finds correct settings.
    let user_scoped_dir = openhuman_home.join("users").join("e2e-user");
    write_min_config(&user_scoped_dir, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Store a session first — all billing methods require it.
    let store = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.auth_store_session",
        json!({ "token": "e2e-billing-jwt", "user_id": "e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    // Helper: the RPC outcome wraps backend data in {result: ..., logs: [...]}.
    // We peel off the inner "result" field to get the actual backend payload.
    fn inner(outer: &Value, _ctx: &str) -> Value {
        outer
            .get("result")
            .cloned()
            .unwrap_or_else(|| outer.clone())
    }

    // --- billing_get_current_plan ---
    let plan = post_json_rpc(
        &rpc_base,
        2,
        "openhuman.billing_get_current_plan",
        json!({}),
    )
    .await;
    let plan_outer = assert_no_jsonrpc_error(&plan, "billing_get_current_plan");
    let plan_result = inner(plan_outer, "billing_get_current_plan");
    assert_eq!(
        plan_result.get("plan").and_then(Value::as_str),
        Some("PRO"),
        "expected PRO plan: {plan_result}"
    );
    assert_eq!(
        plan_result
            .get("hasActiveSubscription")
            .and_then(Value::as_bool),
        Some(true),
        "expected active subscription: {plan_result}"
    );

    // --- billing_purchase_plan ---
    let purchase = post_json_rpc(
        &rpc_base,
        3,
        "openhuman.billing_purchase_plan",
        json!({ "plan": "pro" }),
    )
    .await;
    let purchase_outer = assert_no_jsonrpc_error(&purchase, "billing_purchase_plan");
    let purchase_result = inner(purchase_outer, "billing_purchase_plan");
    assert!(
        purchase_result
            .get("checkoutUrl")
            .and_then(Value::as_str)
            .is_some(),
        "expected checkoutUrl: {purchase_result}"
    );

    // --- billing_create_portal_session ---
    let portal = post_json_rpc(
        &rpc_base,
        4,
        "openhuman.billing_create_portal_session",
        json!({}),
    )
    .await;
    let portal_outer = assert_no_jsonrpc_error(&portal, "billing_create_portal_session");
    let portal_result = inner(portal_outer, "billing_create_portal_session");
    assert!(
        portal_result
            .get("portalUrl")
            .and_then(Value::as_str)
            .is_some(),
        "expected portalUrl: {portal_result}"
    );

    // --- billing_top_up ---
    let top_up = post_json_rpc(
        &rpc_base,
        5,
        "openhuman.billing_top_up",
        json!({ "amountUsd": 10.0, "gateway": "stripe" }),
    )
    .await;
    let top_up_outer = assert_no_jsonrpc_error(&top_up, "billing_top_up");
    let top_up_result = inner(top_up_outer, "billing_top_up");
    assert_eq!(
        top_up_result.get("amountUsd").and_then(Value::as_f64),
        Some(10.0),
        "expected amountUsd 10.0: {top_up_result}"
    );

    // --- billing_create_coinbase_charge ---
    let charge = post_json_rpc(
        &rpc_base,
        6,
        "openhuman.billing_create_coinbase_charge",
        json!({ "plan": "pro" }),
    )
    .await;
    let charge_outer = assert_no_jsonrpc_error(&charge, "billing_create_coinbase_charge");
    let charge_result = inner(charge_outer, "billing_create_coinbase_charge");
    assert!(
        charge_result
            .get("hostedUrl")
            .and_then(Value::as_str)
            .is_some(),
        "expected hostedUrl: {charge_result}"
    );
    assert_eq!(
        charge_result.get("status").and_then(Value::as_str),
        Some("NEW"),
        "expected NEW status: {charge_result}"
    );

    mock_join.abort();
    rpc_join.abort();
}

/// End-to-end test for team RPC methods.
///
/// Spins up an in-process Axum mock backend and a real JSON-RPC server, stores a
/// session JWT, then exercises every team controller through the RPC surface.
#[tokio::test]
async fn team_rpc_e2e() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    // Pre-create the user-scoped config so store_session finds correct settings.
    let user_scoped_dir = openhuman_home.join("users").join("e2e-user");
    write_min_config(&user_scoped_dir, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Store a session first — all team methods require it.
    let store = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.auth_store_session",
        json!({ "token": "e2e-team-jwt", "user_id": "e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    // Helper: peel off the inner "result" field from the RPC outcome envelope.
    fn inner(outer: &Value, _ctx: &str) -> Value {
        outer
            .get("result")
            .cloned()
            .unwrap_or_else(|| outer.clone())
    }

    let team_id = "team-1";

    // --- team_list_members ---
    let members = post_json_rpc(
        &rpc_base,
        2,
        "openhuman.team_list_members",
        json!({ "teamId": team_id }),
    )
    .await;
    let members_outer = assert_no_jsonrpc_error(&members, "team_list_members");
    let members_result = inner(members_outer, "team_list_members");
    let members_arr = members_result
        .as_array()
        .expect("expected array of members");
    assert_eq!(members_arr.len(), 2, "expected 2 members: {members_result}");
    assert_eq!(
        members_arr[0].get("username").and_then(Value::as_str),
        Some("alice")
    );

    // --- team_create_invite ---
    let invite = post_json_rpc(
        &rpc_base,
        3,
        "openhuman.team_create_invite",
        json!({ "teamId": team_id, "maxUses": 3, "expiresInDays": 7 }),
    )
    .await;
    let invite_outer = assert_no_jsonrpc_error(&invite, "team_create_invite");
    let invite_result = inner(invite_outer, "team_create_invite");
    assert!(
        invite_result.get("code").and_then(Value::as_str).is_some(),
        "expected invite code: {invite_result}"
    );

    // --- team_list_invites ---
    let invites = post_json_rpc(
        &rpc_base,
        4,
        "openhuman.team_list_invites",
        json!({ "teamId": team_id }),
    )
    .await;
    let invites_outer = assert_no_jsonrpc_error(&invites, "team_list_invites");
    let invites_result = inner(invites_outer, "team_list_invites");
    let invites_arr = invites_result
        .as_array()
        .expect("expected array of invites");
    assert!(
        !invites_arr.is_empty(),
        "expected at least one invite: {invites_result}"
    );

    // --- team_revoke_invite (no payload to check, just assert no error) ---
    let revoke = post_json_rpc(
        &rpc_base,
        5,
        "openhuman.team_revoke_invite",
        json!({ "teamId": team_id, "inviteId": "inv-1" }),
    )
    .await;
    assert_no_jsonrpc_error(&revoke, "team_revoke_invite");

    // --- team_remove_member ---
    let remove = post_json_rpc(
        &rpc_base,
        6,
        "openhuman.team_remove_member",
        json!({ "teamId": team_id, "userId": "user-2" }),
    )
    .await;
    assert_no_jsonrpc_error(&remove, "team_remove_member");

    // --- team_change_member_role ---
    let role_change = post_json_rpc(
        &rpc_base,
        7,
        "openhuman.team_change_member_role",
        json!({ "teamId": team_id, "userId": "user-1", "role": "MEMBER" }),
    )
    .await;
    assert_no_jsonrpc_error(&role_change, "team_change_member_role");

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn about_app_rpc_list_lookup_and_search() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    tokio::time::sleep(Duration::from_millis(100)).await;

    fn inner(outer: &Value) -> Value {
        outer
            .get("result")
            .cloned()
            .unwrap_or_else(|| outer.clone())
    }

    let list = post_json_rpc(&rpc_base, 200, "openhuman.about_app_list", json!({})).await;
    let list_outer = assert_no_jsonrpc_error(&list, "about_app_list");
    let list_result = inner(list_outer);
    let capabilities = list_result
        .as_array()
        .expect("about_app list should return an array");
    assert!(
        capabilities.len() >= 40,
        "expected large capability catalog, got: {list_result}"
    );
    assert!(capabilities.iter().any(|capability| {
        capability.get("id").and_then(Value::as_str) == Some("local_ai.download_model")
    }));

    let filtered = post_json_rpc(
        &rpc_base,
        201,
        "openhuman.about_app_list",
        json!({ "category": "local_ai" }),
    )
    .await;
    let filtered_outer = assert_no_jsonrpc_error(&filtered, "about_app_list filtered");
    let filtered_result = inner(filtered_outer);
    let filtered_capabilities = filtered_result
        .as_array()
        .expect("filtered about_app list should return an array");
    assert!(
        !filtered_capabilities.is_empty(),
        "expected local_ai capabilities: {filtered_result}"
    );
    assert!(filtered_capabilities.iter().all(|capability| {
        capability.get("category").and_then(Value::as_str) == Some("local_ai")
    }));

    let lookup = post_json_rpc(
        &rpc_base,
        202,
        "openhuman.about_app_lookup",
        json!({ "id": "team.generate_invite_codes" }),
    )
    .await;
    let lookup_outer = assert_no_jsonrpc_error(&lookup, "about_app_lookup");
    let lookup_result = inner(lookup_outer);
    assert_eq!(
        lookup_result.get("id").and_then(Value::as_str),
        Some("team.generate_invite_codes")
    );
    assert_eq!(
        lookup_result.get("category").and_then(Value::as_str),
        Some("team")
    );

    let search = post_json_rpc(
        &rpc_base,
        203,
        "openhuman.about_app_search",
        json!({ "query": "invite" }),
    )
    .await;
    let search_outer = assert_no_jsonrpc_error(&search, "about_app_search");
    let search_result = inner(search_outer);
    let search_capabilities = search_result
        .as_array()
        .expect("about_app search should return an array");
    assert!(
        search_capabilities.iter().any(|capability| {
            capability.get("id").and_then(Value::as_str) == Some("team.join_via_invite_code")
        }),
        "expected invite-related capability in search results: {search_result}"
    );
    assert!(
        search_capabilities.iter().any(|capability| {
            capability.get("id").and_then(Value::as_str) == Some("team.generate_invite_codes")
        }),
        "expected invite generation capability in search results: {search_result}"
    );

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_automation_and_scheduling_spec_6x() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    tokio::time::sleep(Duration::from_millis(100)).await;

    fn inner(outer: &Value, context: &str) -> Value {
        let result = assert_no_jsonrpc_error(outer, context);
        result
            .get("result")
            .cloned()
            .unwrap_or_else(|| result.clone())
    }

    fn pick_task_id(payload: &Value) -> Option<String> {
        payload
            .get("task")
            .and_then(|task| task.get("id"))
            .and_then(Value::as_str)
            .or_else(|| payload.get("id").and_then(Value::as_str))
            .or_else(|| payload.get("task_id").and_then(Value::as_str))
            .map(ToString::to_string)
    }

    // 6.1.1 Task Creation
    let created = post_json_rpc(
        &rpc_base,
        3001,
        "openhuman.subconscious_tasks_add",
        json!({
            "title": "json-rpc e2e scheduled task",
            "source": "user"
        }),
    )
    .await;
    let created_payload = inner(&created, "subconscious_tasks_add");
    let task_id = pick_task_id(&created_payload)
        .unwrap_or_else(|| panic!("expected task id in tasks_add payload: {created_payload}"));

    // 6.1.2 Task Update
    let updated = post_json_rpc(
        &rpc_base,
        3002,
        "openhuman.subconscious_tasks_update",
        json!({
            "task_id": task_id,
            "title": "json-rpc e2e scheduled task updated",
            "enabled": true
        }),
    )
    .await;
    inner(&updated, "subconscious_tasks_update");

    let listed_after_update = post_json_rpc(
        &rpc_base,
        3003,
        "openhuman.subconscious_tasks_list",
        json!({}),
    )
    .await;
    let listed_payload = inner(&listed_after_update, "subconscious_tasks_list");
    let listed_text = listed_payload.to_string();
    assert!(
        listed_text.contains("json-rpc e2e scheduled task updated"),
        "expected updated task title in list payload: {listed_payload}"
    );

    // 6.1.3 Task Deletion
    let removed = post_json_rpc(
        &rpc_base,
        3004,
        "openhuman.subconscious_tasks_remove",
        json!({ "task_id": task_id }),
    )
    .await;
    inner(&removed, "subconscious_tasks_remove");

    let listed_after_remove = post_json_rpc(
        &rpc_base,
        3005,
        "openhuman.subconscious_tasks_list",
        json!({}),
    )
    .await;
    let listed_after_remove_payload = inner(&listed_after_remove, "subconscious_tasks_list");
    assert!(
        !listed_after_remove_payload.to_string().contains(&task_id),
        "expected removed task to be absent: {listed_after_remove_payload}"
    );

    // 6.2.1 Cron Expression Validation
    let cron_task = post_json_rpc(
        &rpc_base,
        3006,
        "openhuman.subconscious_tasks_add",
        json!({
            "title": "json-rpc e2e cron validation task",
            "source": "user"
        }),
    )
    .await;
    let cron_task_payload = inner(&cron_task, "subconscious_tasks_add cron");
    let cron_task_id = pick_task_id(&cron_task_payload).unwrap_or_else(|| {
        panic!("expected task id in cron validation task payload: {cron_task_payload}")
    });

    let invalid_update = post_json_rpc(
        &rpc_base,
        3007,
        "openhuman.subconscious_tasks_update",
        json!({
            "task_id": cron_task_id,
            "recurrence": "cron:not-a-valid-expression"
        }),
    )
    .await;
    let invalid_msg = invalid_update
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        invalid_update.get("error").is_some() && invalid_msg.contains("invalid cron expression"),
        "expected invalid cron recurrence error, got: {invalid_update}"
    );

    let cleanup_invalid_task = post_json_rpc(
        &rpc_base,
        3008,
        "openhuman.subconscious_tasks_remove",
        json!({ "task_id": cron_task_id }),
    )
    .await;
    inner(
        &cleanup_invalid_task,
        "subconscious_tasks_remove cron validation",
    );

    // 6.2.2 Recurring Execution + 6.2.3 Missed Execution Handling
    let trigger_1 =
        post_json_rpc(&rpc_base, 3009, "openhuman.subconscious_trigger", json!({})).await;
    inner(&trigger_1, "subconscious_trigger first");
    let trigger_2 =
        post_json_rpc(&rpc_base, 3010, "openhuman.subconscious_trigger", json!({})).await;
    inner(&trigger_2, "subconscious_trigger second");

    tokio::time::sleep(Duration::from_millis(250)).await;
    let logs = post_json_rpc(
        &rpc_base,
        3011,
        "openhuman.subconscious_log_list",
        json!({ "limit": 20 }),
    )
    .await;
    let logs_payload = inner(&logs, "subconscious_log_list");
    let logs_arr = logs_payload
        .as_array()
        .or_else(|| logs_payload.get("entries").and_then(Value::as_array))
        .unwrap_or_else(|| panic!("expected log array payload, got: {logs_payload}"));
    assert!(
        !logs_arr.is_empty(),
        "expected non-empty subconscious log entries after trigger: {logs_payload}"
    );

    // 6.3.1 Remote Agent Scheduling
    let cron_list = post_json_rpc(&rpc_base, 3012, "openhuman.cron_list", json!({})).await;
    inner(&cron_list, "cron_list");

    // 6.3.2 Execution Trigger Handling
    let missing_run = post_json_rpc(
        &rpc_base,
        3013,
        "openhuman.cron_run",
        json!({ "job_id": "missing-job-id-e2e" }),
    )
    .await;
    assert!(
        missing_run.get("error").is_some(),
        "expected explicit error for missing cron job id: {missing_run}"
    );

    // 6.3.3 Failure Retry Logic
    let config = openhuman_core::openhuman::config::load_config_with_timeout()
        .await
        .expect("load config for cron setup");
    let failing_job = add_shell_job(
        &config,
        Some("json-rpc e2e failing cron job".to_string()),
        Schedule::Every { every_ms: 60_000 },
        "this-command-should-not-exist-openhuman-e2e",
    )
    .expect("create failing cron job");

    let run_failure = post_json_rpc(
        &rpc_base,
        3014,
        "openhuman.cron_run",
        json!({ "job_id": failing_job.id }),
    )
    .await;
    let run_payload = inner(&run_failure, "cron_run failing job");
    let run_status = run_payload
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| {
            run_payload
                .get("result")
                .and_then(|v| v.get("status"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    assert_eq!(
        run_status, "error",
        "expected failing cron command to record error status: {run_payload}"
    );

    let run_history = post_json_rpc(
        &rpc_base,
        3015,
        "openhuman.cron_runs",
        json!({ "job_id": failing_job.id, "limit": 5 }),
    )
    .await;
    let run_history_payload = inner(&run_history, "cron_runs failing job");
    let run_history_arr = run_history_payload
        .as_array()
        .or_else(|| run_history_payload.get("runs").and_then(Value::as_array))
        .unwrap_or_else(|| panic!("expected cron run history array, got: {run_history_payload}"));
    assert!(
        !run_history_arr.is_empty(),
        "expected non-empty cron run history after failed run: {run_history_payload}"
    );
    assert!(
        run_history_arr
            .iter()
            .any(|entry| entry.get("status").and_then(Value::as_str) == Some("error")),
        "expected at least one failed run in history: {run_history_payload}"
    );

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn voice_status_returns_availability() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _whisper_guard = EnvVarGuard::unset("WHISPER_BIN");
    let _piper_guard = EnvVarGuard::unset("PIPER_BIN");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // voice_status does not require auth — it only checks filesystem availability
    let status = post_json_rpc(&rpc_base, 1, "openhuman.voice_status", json!({})).await;
    let result = assert_no_jsonrpc_error(&status, "voice_status");

    // Without whisper/piper installed in the test env, both should be unavailable
    assert!(
        result.get("stt_available").is_some(),
        "expected stt_available field: {result}"
    );
    assert!(
        result.get("tts_available").is_some(),
        "expected tts_available field: {result}"
    );
    assert!(
        result.get("stt_model_id").is_some(),
        "expected stt_model_id field: {result}"
    );
    assert!(
        result.get("tts_voice_id").is_some(),
        "expected tts_voice_id field: {result}"
    );

    // Verify that without binaries, availability is false
    assert_eq!(
        result.get("stt_available").and_then(Value::as_bool),
        Some(false),
        "stt should be unavailable without whisper binary"
    );
    assert_eq!(
        result.get("tts_available").and_then(Value::as_bool),
        Some(false),
        "tts should be unavailable without piper binary"
    );

    mock_join.abort();
    rpc_join.abort();
}
