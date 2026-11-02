//! HTTP JSON-RPC integration tests against a real axum stack and a mock upstream API.
//!
//! Isolates config under a temp `HOME` so auth profiles and the OpenHuman provider resolve
//! the same state directory. Run a namespace with `cargo test -p openhuman-cli --test json_rpc_<namespace>_e2e`.

// This harness is compiled separately into each namespace test binary; each
// binary intentionally uses only the helpers and re-exports it needs.
#![allow(dead_code, unused_imports)]

#[path = "tinyhumans_boot.rs"]
pub mod tinyhumans_boot;

#[path = "env_guard.rs"]
pub mod env_guard;
#[path = "scripted_stack.rs"]
pub mod scripted_stack;
pub use env_guard::EnvVarGuard;
pub use scripted_stack::assert_no_jsonrpc_error;
pub use std::collections::VecDeque;
pub use std::net::SocketAddr;
pub use std::path::{Path, PathBuf};
pub use std::sync::{Arc, Mutex, OnceLock};
pub use std::time::Duration;

pub use axum::extract::State;
pub use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode, Uri};
pub use axum::routing::{get, post};
pub use axum::{Json, Router};
pub use futures_util::StreamExt;
pub use serde_json::{json, Value};
pub use tempfile::tempdir;
pub use tinyinference_llm::message::Message;
pub use tinyinference_llm::model::ModelRequest;

pub use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
pub use openhuman_core::platform::connectivity::rpc::pick_listen_port;
pub use openhuman_rpc::server::build_core_http_router;

pub(crate) const TEST_RPC_TOKEN: &str = "json-rpc-e2e-local-token";
pub(crate) static JSON_RPC_AUTH_INIT: OnceLock<()> = OnceLock::new();

/// Serializes tests in this binary: `HOME` / `OPENHUMAN_WORKSPACE` / backend URL overrides are
/// process-global, so parallel tests would clobber each other and hit the wrong `config.toml` or
/// inherited `VITE_BACKEND_URL`.
pub(crate) static JSON_RPC_E2E_ENV_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
pub(crate) static CHAT_COMPLETION_MODELS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
pub(crate) static CHAT_COMPLETION_REQUESTS: OnceLock<Mutex<Vec<Value>>> = OnceLock::new();

pub(crate) async fn json_rpc_e2e_env_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    env_guard::init_keyring_workspace();
    let mutex = JSON_RPC_E2E_ENV_LOCK.get_or_init(|| tokio::sync::Mutex::new(()));
    // A tokio mutex is never poisoned, so a panic in one test cannot cascade to the others.
    mutex.lock().await
}

pub(crate) fn run_json_rpc_e2e_on_agent_stack<F, Fut>(name: &str, future_factory: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(move || {
            // The lightweight HTTP router used by these E2E cases does not
            // execute the full core boot sequence. Hosted turns still need
            // the same built-in definition registry that boot initializes in
            // production, so install it before constructing the router.
            openhuman_core::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins()
                .expect("initialize built-in agent definitions for JSON-RPC E2E");
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
                .enable_all()
                .build()
                .expect("build json_rpc e2e runtime");
            rt.block_on(future_factory());
        })
        .expect("spawn json_rpc e2e thread")
        .join()
        .expect("json_rpc e2e thread should not panic");
}

pub(crate) fn with_chat_completion_models<T>(f: impl FnOnce(&mut Vec<String>) -> T) -> T {
    let mutex = CHAT_COMPLETION_MODELS.get_or_init(|| Mutex::new(Vec::new()));
    match mutex.lock() {
        Ok(mut guard) => f(&mut guard),
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            f(&mut guard)
        }
    }
}

pub(crate) fn with_chat_completion_requests<T>(f: impl FnOnce(&mut Vec<Value>) -> T) -> T {
    let mutex = CHAT_COMPLETION_REQUESTS.get_or_init(|| Mutex::new(Vec::new()));
    match mutex.lock() {
        Ok(mut guard) => f(&mut guard),
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            f(&mut guard)
        }
    }
}

// ── Scripted chat-completion FIFO (in-process, node-mock-free) ───────────────
//
// The heuristic `chat_completions` mock can't script tool calls, so agent-driven
// flows (`flows_build` / `flows_discover`) and agent-node flow runs had no way to
// exercise a deterministic tool_call / planner-drafter arc in-process. This FIFO
// mirrors the node mock's `llmForcedResponses` (`scripts/mock-api/routes/llm.mjs`)
// without pulling in a node dependency: a test pushes OpenAI-shape response
// bodies, and the chat-completions handler pops the first *matching* one before
// falling back to its heuristic.
//
// Entries are optionally gated on a marker substring appearing in the serialized
// request body (`when_contains`). That gate is what makes the arc robust against
// the "FIFO-drain" hazard the plan flags: a builder/scout turn may emit side
// completions (memory summarization, etc.) that don't carry the marker, so they
// hit the heuristic fallback instead of consuming a scripted response meant for a
// different node. Matching entries are still served in strict FIFO order among
// themselves, so a two-turn tool loop (tool_call turn, then terminal-text turn)
// stays ordered.
pub(crate) static FORCED_CHAT_COMPLETIONS: OnceLock<Mutex<VecDeque<ForcedChatCompletion>>> =
    OnceLock::new();

#[derive(Clone)]
pub(crate) struct ForcedChatCompletion {
    /// When `Some`, only serve this entry if the serialized request body contains
    /// this marker. `None` matches any request (strict FIFO).
    when_contains: Option<String>,
    /// The OpenAI-shape chat-completion body to return verbatim.
    body: Value,
}

pub(crate) fn with_forced_chat_completions<T>(
    f: impl FnOnce(&mut VecDeque<ForcedChatCompletion>) -> T,
) -> T {
    let mutex = FORCED_CHAT_COMPLETIONS.get_or_init(|| Mutex::new(VecDeque::new()));
    match mutex.lock() {
        Ok(mut guard) => f(&mut guard),
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            f(&mut guard)
        }
    }
}

/// Drop every queued scripted completion. Call between test sections so a leftover
/// entry can never bleed into an unrelated arc.
#[allow(dead_code)]
pub(crate) fn clear_forced_chat_completions() {
    with_forced_chat_completions(|q| q.clear());
}

/// RAII guard that drains the process-global scripted-completion FIFO on drop —
/// including on an assertion panic mid-test. A trailing `clear_forced_chat_completions()`
/// call only runs on the happy path; if an `assert!` unwinds first, the leftover
/// scripted entries would otherwise be consumed by a later test sharing this
/// binary. Hold one for the whole body of any test that queues completions.
#[allow(dead_code)]
pub(crate) struct ScriptedFifoGuard;

impl Drop for ScriptedFifoGuard {
    fn drop(&mut self) {
        clear_forced_chat_completions();
    }
}

/// Queue an unconditional scripted completion (strict FIFO — matches any request).
#[allow(dead_code)]
pub(crate) fn push_forced_chat_completion(body: Value) {
    with_forced_chat_completions(|q| {
        q.push_back(ForcedChatCompletion {
            when_contains: None,
            body,
        })
    });
}

/// Queue a scripted completion gated on `marker` appearing in the request body.
#[allow(dead_code)]
pub(crate) fn push_forced_chat_completion_when(marker: &str, body: Value) {
    with_forced_chat_completions(|q| {
        q.push_back(ForcedChatCompletion {
            when_contains: Some(marker.to_string()),
            body,
        })
    });
}

/// Pop the first queued completion whose marker matches `request_body`
/// (unconditional entries always match), preserving FIFO order among matches.
pub(crate) fn take_forced_chat_completion(request_body: &Value) -> Option<Value> {
    let haystack = request_body.to_string();
    with_forced_chat_completions(|q| {
        let idx = q.iter().position(|entry| match &entry.when_contains {
            Some(marker) => haystack.contains(marker.as_str()),
            None => true,
        })?;
        q.remove(idx).map(|entry| entry.body)
    })
}

/// A scripted plain-text assistant completion in the OpenAI shape the managed
/// backend parses (mirrors the node mock's non-streaming `makeChoice`).
#[allow(dead_code)]
pub(crate) fn forced_text_completion(content: &str) -> Value {
    json!({
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 10, "total_tokens": 20 }
    })
}

/// A scripted single-tool-call assistant completion. `arguments` is serialized to
/// the JSON string the OpenAI tool-calling contract (and the harness) expects.
#[allow(dead_code)]
pub(crate) fn forced_tool_call_completion(tool_name: &str, arguments: Value) -> Value {
    json!({
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": format!("call_{tool_name}"),
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": arguments.to_string()
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 10, "total_tokens": 20 }
    })
}

pub(crate) fn mock_upstream_router() -> Router {
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

    async fn chat_completions(
        uri: Uri,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        if let Some(model) = body.get("model").and_then(Value::as_str) {
            with_chat_completion_models(|models| models.push(model.to_string()));
        }
        let auth_header = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let x_api_key = headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        with_chat_completion_requests(|requests| {
            requests.push(json!({
                "path": uri.path(),
                "model": body.get("model").and_then(Value::as_str),
                "stream": body.get("stream").and_then(Value::as_bool),
                "thread_id": body.get("thread_id").and_then(Value::as_str),
                "authorization": auth_header,
                "x_api_key": x_api_key,
                "body": body.clone(),
            }))
        });
        // A scripted response (tool_call or plain completion) wins over the
        // heuristic when one is queued and matches this request.
        if let Some(forced) = take_forced_chat_completion(&body) {
            return Json(forced);
        }
        let is_triage_turn = body
            .get("messages")
            .and_then(Value::as_array)
            .map(|messages| {
                messages.iter().any(|m| {
                    m.get("content")
                        .and_then(Value::as_str)
                        .is_some_and(|content| {
                            content.contains("SOURCE: ")
                                && content.contains("DISPLAY_LABEL: ")
                                && content.contains("PAYLOAD:")
                        })
                })
            })
            .unwrap_or(false);
        let content = if is_triage_turn {
            "{\"action\":\"react\",\"reason\":\"e2e triage mock\"}"
        } else {
            "Hello from e2e mock agent"
        };
        Json(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": format!("{content} via /openai/v1/chat/completions")
                }
            }]
        }))
    }

    async fn generic_chat_completions(
        uri: Uri,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        if let Some(model) = body.get("model").and_then(Value::as_str) {
            with_chat_completion_models(|models| models.push(model.to_string()));
        }
        let auth_header = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let x_api_key = headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        with_chat_completion_requests(|requests| {
            requests.push(json!({
                "path": uri.path(),
                "model": body.get("model").and_then(Value::as_str),
                "stream": body.get("stream").and_then(Value::as_bool),
                "thread_id": body.get("thread_id").and_then(Value::as_str),
                "authorization": auth_header,
                "x_api_key": x_api_key,
                "body": body.clone(),
            }))
        });
        Json(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": format!(
                        "Hello from custom provider {}",
                        body.get("model").and_then(Value::as_str).unwrap_or("unknown-model")
                    )
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
        // The deployed backend's `BillingPlan` values (the SDK's typed request).
        if !matches!(
            plan,
            "BASIC_MONTHLY" | "BASIC_YEARLY" | "PRO_MONTHLY" | "PRO_YEARLY"
        ) {
            return Err(error_json(
                StatusCode::BAD_REQUEST,
                "missing or invalid 'plan'",
            ));
        }

        let checkout_url = "http://127.0.0.1/mock-checkout";
        let session_id = "cs_mock_abc";

        Ok(Json(json!({
            "success": true,
            "data": { "checkoutUrl": checkout_url, "sessionId": session_id }
        })))
    }

    async fn stripe_portal(headers: HeaderMap) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        require_bearer(&headers, BILLING_TOKEN)?;
        let portal_url = "http://127.0.0.1/mock-portal";

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
        .route("/v1/chat/completions", post(generic_chat_completions))
        .route("/chat/completions", post(generic_chat_completions))
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

#[derive(Clone)]
pub(crate) struct MockWalletRpcState {
    raw_txs: Arc<Mutex<Vec<String>>>,
}

pub(crate) async fn mock_wallet_evm_rpc(
    State(state): State<MockWalletRpcState>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    let method = payload
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = payload
        .get("params")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let result = match method {
        "eth_chainId" => Value::String("0x1".to_string()),
        "eth_getTransactionCount" => Value::String("0x7".to_string()),
        "eth_gasPrice" => Value::String("0x3b9aca00".to_string()),
        "eth_estimateGas" => Value::String("0x5208".to_string()),
        "eth_sendRawTransaction" => {
            if let Some(raw) = params.first().and_then(Value::as_str) {
                match state.raw_txs.lock() {
                    Ok(mut guard) => guard.push(raw.to_string()),
                    Err(poisoned) => poisoned.into_inner().push(raw.to_string()),
                }
            }
            Value::String(
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            )
        }
        "eth_getBalance" => Value::String("0x0".to_string()),
        "eth_blockNumber" => Value::String("0x14".to_string()),
        // Solana's liveness probe, so one mock can stand in for every chain.
        "getHealth" => Value::String("ok".to_string()),
        "eth_getTransactionByHash" => {
            json!({"hash": params.first().cloned().unwrap_or(Value::Null)})
        }
        "eth_getTransactionReceipt" => json!({
            "status": "0x1",
            "blockNumber": "0x10",
            "gasUsed": "0x5208",
            "effectiveGasPrice": "0x3b9aca00"
        }),
        _ => Value::Null,
    };
    Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
}

/// Esplora's chain-tip read, which the wallet's Bitcoin status probe uses.
pub(crate) async fn mock_wallet_btc_tip() -> &'static str {
    "850000"
}

/// TronGrid's latest-block read, which the wallet's Tron status probe uses.
pub(crate) async fn mock_wallet_tron_now_block() -> Json<Value> {
    Json(json!({"blockID": "0000000000000001", "block_header": {}}))
}

pub(crate) async fn start_mock_wallet_evm_rpc() -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    let raw_txs = Arc::new(Mutex::new(Vec::new()));
    let state = MockWalletRpcState {
        raw_txs: raw_txs.clone(),
    };
    let app = Router::new()
        .route("/", post(mock_wallet_evm_rpc))
        .route("/blocks/tip/height", get(mock_wallet_btc_tip))
        .route("/wallet/getnowblock", post(mock_wallet_tron_now_block))
        .with_state(state);
    let (addr, _join) = serve_on_ephemeral(app).await;
    (addr, raw_txs)
}

pub(crate) async fn serve_on_ephemeral(
    app: Router,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    ensure_test_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, handle)
}

pub(crate) async fn post_json_rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
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
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
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

#[allow(dead_code)]
pub(crate) async fn read_first_sse_event(events_url: &str) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("client");
    let resp = client
        .get(events_url)
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
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

/// Read SSE events until one matches the given `event` field value, skipping
/// progress events (inference_start, iteration_start, etc.) that precede the
/// terminal event.
pub(crate) async fn read_sse_event_by_type(events_url: &str, target_event: &str) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("client");
    let resp = client
        .get(events_url)
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
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
                if value.get("event").and_then(Value::as_str) == Some(target_event) {
                    return value;
                }
            }
        }
    }
    panic!("SSE stream ended before receiving '{target_event}' event");
}

/// Read SSE events until a terminal web-chat event arrives.
///
/// This prevents tests from timing out blindly when the turn actually
/// completed with `chat_error` rather than `chat_done`.
pub(crate) async fn read_terminal_web_chat_event(events_url: &str) -> Value {
    read_terminal_web_chat_event_with_ready(events_url, None, None).await
}

pub(crate) async fn read_terminal_web_chat_event_with_ready(
    events_url: &str,
    ready: Option<tokio::sync::oneshot::Sender<()>>,
    expected_request: Option<tokio::sync::oneshot::Receiver<String>>,
) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("client");
    let resp = client
        .get(events_url)
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {events_url}: {e}"));
    assert!(
        resp.status().is_success(),
        "SSE HTTP error {} for {}",
        resp.status(),
        events_url
    );
    if let Some(ready) = ready {
        let _ = ready.send(());
    }
    let expected_request_id = match expected_request {
        Some(receiver) => Some(receiver.await.expect("accepted request id should be sent")),
        None => None,
    };

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
                match value.get("event").and_then(Value::as_str) {
                    Some("chat_done") | Some("chat_error")
                        if expected_request_id.as_deref().is_none_or(|id| {
                            value.get("request_id").and_then(Value::as_str) == Some(id)
                        }) =>
                    {
                        return value;
                    }
                    _ => {}
                }
            }
        }
    }
    panic!("SSE stream ended before receiving terminal web-chat event");
}

pub(crate) async fn spawn_ready_terminal_web_chat_event_for_request(
    events_url: &str,
) -> (
    tokio::task::JoinHandle<Value>,
    tokio::sync::oneshot::Sender<String>,
) {
    // The previous turn can emit a late cancellation on the same client SSE
    // stream. Subscribe before submitting, then match only the accepted turn.
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let events_url = events_url.to_owned();
    let task = tokio::spawn(async move {
        read_terminal_web_chat_event_with_ready(&events_url, Some(ready_tx), Some(request_rx)).await
    });
    tokio::time::timeout(Duration::from_secs(5), ready_rx)
        .await
        .expect("SSE subscription should become ready")
        .expect("SSE reader should signal readiness");
    (task, request_tx)
}

pub(crate) fn signal_accepted_web_chat_request_id(
    sender: tokio::sync::oneshot::Sender<String>,
    accepted: &Value,
) {
    let request_id = accepted
        .get("result")
        .and_then(|result| result.get("request_id"))
        .and_then(Value::as_str)
        .expect("accepted web chat request_id");
    sender
        .send(request_id.to_owned())
        .expect("SSE reader should receive request_id");
}

pub(crate) async fn wait_for_chat_completion_requests_len(expected_len: usize) -> Vec<Value> {
    for _ in 0..100 {
        let snapshot = with_chat_completion_requests(|requests| requests.clone());
        if snapshot.len() >= expected_len {
            return snapshot;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    with_chat_completion_requests(|requests| requests.clone())
}

pub(crate) async fn wait_for_chat_completion_request_with_message(message: &str) -> Value {
    for _ in 0..100 {
        if let Some(request) = with_chat_completion_requests(|requests| {
            requests
                .iter()
                .find(|request| {
                    request["body"]["messages"]
                        .as_array()
                        .is_some_and(|messages| {
                            messages.iter().any(|entry| {
                                entry["content"]
                                    .as_str()
                                    .is_some_and(|content| content.contains(message))
                            })
                        })
                })
                .cloned()
        }) {
            return request;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for chat completion containing {message:?}");
}

pub(crate) async fn encrypt_test_mnemonic() -> String {
    let config = openhuman_core::config::load_config_with_timeout()
        .await
        .expect("load config for encrypted test mnemonic");
    openhuman_core::security::secret_storage::init_workspace(&config.workspace_dir);
    openhuman_core::security::encryption::rpc::encrypt_secret(
        &config,
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .await
    .expect("encrypt test mnemonic")
    .value
}

pub(crate) fn assert_jsonrpc_error<'a>(v: &'a Value, context: &str) -> &'a Value {
    v.get("error")
        .unwrap_or_else(|| panic!("{context}: expected JSON-RPC error, got: {v}"))
}

pub(crate) fn assert_unknown_method(v: &Value, method: &str) {
    let error = assert_jsonrpc_error(v, method);
    let expected_message = format!("unknown method: {method}");
    assert_eq!(
        error.get("code").and_then(Value::as_i64),
        Some(-32000),
        "{method}: expected unknown-method server error: {v}"
    );
    assert_eq!(
        error.get("message").and_then(Value::as_str),
        Some(expected_message.as_str()),
        "{method}: expected unknown-method message: {v}"
    );
}

pub(crate) fn extract_string_outcome(result: &Value) -> String {
    if let Some(s) = result.as_str() {
        return s.to_string();
    }
    if let Some(inner) = result.get("result").and_then(Value::as_str) {
        return inner.to_string();
    }
    panic!("expected string or {{result: string}}, got {result}");
}

/// Peel the `{"result": inner, "logs": [...]}` envelope that
/// `Outcome::into_cli_compatible_json` adds when logs are present.
pub(crate) fn peel_logs_envelope(v: &Value) -> &Value {
    if v.get("logs").is_some() {
        v.get("result").unwrap_or(v)
    } else {
        v
    }
}

/// Extract the `is_error` flag from a tool_call response, tolerating both the
/// handler's snake_case `is_error` and the MCP protocol's camelCase `isError`.
pub(crate) fn tool_call_is_error(v: &Value) -> Option<bool> {
    v.get("is_error")
        .or_else(|| v.get("isError"))
        .and_then(Value::as_bool)
}

pub(crate) fn write_min_config(openhuman_dir: &Path, api_origin: &str) {
    // `chat_onboarding_completed = true` is retained for backward compatibility
    // with existing config.toml files. All chat turns now route to the
    // orchestrator directly regardless of this flag.
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "e2e-mock-model"
default_temperature = 0.7
chat_onboarding_completed = true

[secrets]
encrypt = false
"#
    );
    fn write_config_file(config_dir: &Path, cfg: &str) {
        std::fs::create_dir_all(config_dir).expect("mkdir openhuman");
        let path = config_dir.join("config.toml");
        std::fs::write(&path, cfg).expect("write config");
    }

    write_config_file(openhuman_dir, &cfg);

    // Runtime config resolution is user-scoped before login, so tests that seed
    // the root `~/.openhuman` directory also need the equivalent pre-login
    // config under `~/.openhuman/users/local`.
    if openhuman_dir
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new(".openhuman"))
    {
        write_config_file(&openhuman_dir.join("users").join("local"), &cfg);
    }

    let _: openhuman_core::config::Config =
        toml::from_str(&cfg).expect("config toml must match Config schema");
}

/// Seed a backend API key in the pre-login profile used by the RPC host.
/// `write_min_config` writes both root and `users/local` config files, but the
/// runtime resolves credentials from the latter before a user signs in.
pub(crate) fn store_local_profile_api_key(openhuman_dir: &Path, api_origin: &str) {
    let user_scoped_dir = openhuman_dir.join("users").join("local");
    let mut config = openhuman_core::config::Config {
        config_path: user_scoped_dir.join("config.toml"),
        workspace_dir: user_scoped_dir.join("workspace"),
        api_url: Some(api_origin.to_owned()),
        ..Default::default()
    };
    config.secrets.encrypt = false;
    std::fs::create_dir_all(&config.workspace_dir).expect("create test workspace");
    openhuman_core::security::credentials::api_key::store_api_key(
        &config,
        "th_live_rpc_e2e_test_key",
    )
    .expect("store test backend API key");
}

pub(crate) fn write_min_config_with_local_ai_disabled(openhuman_dir: &Path, api_origin: &str) {
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "e2e-mock-model"
default_temperature = 0.7
chat_onboarding_completed = true

[secrets]
encrypt = false

[local_ai]
enabled = false
"#
    );
    fn write_config_file(config_dir: &Path, cfg: &str) {
        std::fs::create_dir_all(config_dir).expect("mkdir openhuman");
        let path = config_dir.join("config.toml");
        std::fs::write(&path, cfg).expect("write config");
    }

    write_config_file(openhuman_dir, &cfg);

    if openhuman_dir
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new(".openhuman"))
    {
        write_config_file(&openhuman_dir.join("users").join("local"), &cfg);
    }

    let _: openhuman_core::config::Config =
        toml::from_str(&cfg).expect("config toml must match Config schema");
}

pub(crate) fn ensure_test_rpc_auth() {
    JSON_RPC_AUTH_INIT.get_or_init(|| {
        // The core carries no backend client; billing / team / composio RPCs
        // reach the mock backend through the `openhuman-tinyhumans` transport.
        openhuman_tinyhumans::install(openhuman_tinyhumans::InstallOptions::default())
            .expect("install the TinyHumans backend transport for json_rpc_e2e");
        // SAFETY: set_var is inside get_or_init so it runs exactly once across
        // all test threads. Rust 1.81+ requires unsafe for set_var in
        // multi-threaded contexts; the OnceLock guard limits the mutation to a
        // single call at init time, before any concurrent env reads occur.
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN) };
        let token_dir = std::env::temp_dir().join("openhuman-json-rpc-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token for json_rpc_e2e");
    });
}
