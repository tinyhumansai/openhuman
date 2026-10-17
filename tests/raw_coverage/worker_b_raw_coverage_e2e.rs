//! Raw-line oriented JSON-RPC E2E coverage for Worker B-owned domains:
//! inference, agent, tools, tool_registry, and approval.
//!
//! These tests use the real core JSON-RPC router and local mock HTTP services
//! to drive implementation branches that the controller reachability tests only
//! touch at validation boundaries.

use crate::env_guard::EnvVarGuard;
use crate::rpc_harness::{error_message, ok, payload};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::routing::{get, post};
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::agent::turn_origin::{self, AgentTurnOrigin};
use openhuman_core::config::Config;
use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::security::approval::gate::{
    ApprovalChatContext, ApprovalGate, ApprovalTurnContext,
};
use openhuman_core::security::approval::types::{ExecutionOutcome, GateOutcome};
use openhuman_core::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_rpc::server::build_core_http_router;

const TEST_RPC_TOKEN: &str = "worker-b-raw-coverage-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();
static ENV_LOCK: &OnceLock<tokio::sync::Mutex<()>> = &crate::SHARED_ENV_LOCK;

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<Value>>>,
}

struct TestHarness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    rpc_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

struct MockHarness {
    base: String,
    state: MockState,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    let mutex = ENV_LOCK.get_or_init(|| tokio::sync::Mutex::new(()));
    mutex.blocking_lock()
}

async fn env_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    let mutex = ENV_LOCK.get_or_init(|| tokio::sync::Mutex::new(()));
    mutex.lock().await
}

fn ensure_rpc_auth() {
    // The core carries no backend client of its own. This suite boots the core
    // in-process via `build_core_http_router`, so without the SDK-backed
    // transport every backend-touching call answers `BACKEND_UNAVAILABLE:` —
    // which is what quarantined the managed web-search case (#6387). The
    // aggregate target only DECLARES the module (`raw_coverage_all.rs:40-41`);
    // its doc at `:38` says each suite calls it from its own fixture, and the
    // siblings that reach the backend do.
    // Idempotent behind a `Once`, so the other suites in this binary calling it
    // too costs nothing.
    crate::tinyhumans_boot::boot();
    AUTH_INIT.get_or_init(|| {
        std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);
        let token_dir = std::env::temp_dir().join("openhuman-worker-b-raw-coverage-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
}

/// The bearer this process actually validates.
///
/// `core::auth::RPC_TOKEN` is a process-global `OnceLock` and `init_rpc_token`
/// returns early once it is set — deliberately, so a second call cannot 401 live
/// clients. Since `tests/raw_coverage/` is one aggregated binary, only the first
/// suite to reach `ensure_rpc_auth` pins its own `TEST_RPC_TOKEN`; every other
/// suite sending its literal gets a 401 and trips its own `assert_eq!` (#6112).
/// Ask the auth module what it settled on instead of assuming we won the race.
fn rpc_bearer() -> &'static str {
    ensure_rpc_auth();
    openhuman_core::core::auth::get_rpc_token()
        .expect("ensure_rpc_auth initialises the token subsystem on the line above")
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

async fn serve_mock() -> MockHarness {
    let state = MockState::default();
    let router = Router::new()
        .route("/v1/models", get(mock_models))
        .route("/v1/missing-models", get(mock_missing_models))
        .route("/v1/chat/completions", post(mock_chat_completions))
        .route("/agent-integrations/exa/search", post(mock_exa_search))
        .route(
            "/agent-integrations/gemini/models/{model}/generate-content",
            post(mock_gemini_generate),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock listener");
    let addr = listener.local_addr().expect("mock listener addr");
    let join = tokio::spawn(async move { axum::serve(listener, router).await });
    MockHarness {
        base: format!("http://{addr}"),
        state,
        join,
    }
}

async fn mock_models(State(state): State<MockState>) -> Json<Value> {
    state
        .requests
        .lock()
        .expect("requests lock")
        .push(json!({ "path": "/v1/models" }));
    Json(json!({
        "object": "list",
        "data": [
            { "id": "worker-b-chat", "object": "model", "created": 1, "owned_by": "e2e" },
            { "id": "worker-b-reasoning", "object": "model", "created": 2, "owned_by": "e2e" }
        ]
    }))
}

async fn mock_missing_models() -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn mock_chat_completions(
    State(state): State<MockState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    state
        .requests
        .lock()
        .expect("requests lock")
        .push(json!({ "path": "/v1/chat/completions", "body": body }));
    Json(json!({
        "id": "chatcmpl-worker-b",
        "object": "chat.completion",
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "mock provider reply from worker b"
                },
                "finish_reason": "stop"
            }
        ]
    }))
}

async fn mock_gemini_generate(
    State(state): State<MockState>,
    axum::extract::Path(model): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    state.requests.lock().expect("requests lock").push(json!({
        "path": format!("/agent-integrations/gemini/models/{model}/generate-content"),
        "body": body
    }));
    Json(json!({
        "success": true,
        "data": {
            "modelVersion": model,
            "candidates": [{
                "content": {"role": "model", "parts": [{"text": "Worker B grounded answer."}]},
                "groundingMetadata": {
                    "groundingChunks": [
                        {"web": {"uri": "https://example.com/grounded", "title": "Grounded source"}}
                    ]
                }
            }],
            "costUsd": 0.002
        }
    }))
}

async fn mock_exa_search(State(state): State<MockState>, Json(body): Json<Value>) -> Json<Value> {
    state
        .requests
        .lock()
        .expect("requests lock")
        .push(json!({ "path": "/agent-integrations/exa/search", "body": body }));
    Json(json!({
        "success": true,
        "data": {
            "searchId": "exa-worker-b",
            "costUsd": 0.01,
            "results": [
                {
                    "url": "https://example.com/worker-b",
                    "title": "Worker B coverage",
                    "publish_date": "2026-05-29",
                    "excerpts": ["coverage result"]
                }
            ]
        }
    }))
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
    let _: Config = toml::from_str(cfg).expect("test config must match schema");
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
        EnvVarGuard::unset("OPENHUMAN_LM_STUDIO_BASE_URL"),
        EnvVarGuard::unset("LM_STUDIO_BASE_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];

    let _ = openhuman_core::agent::harness::host_definition::AgentDefinitionRegistry::init_global_builtins();

    let (addr, rpc_join) = serve_rpc().await;
    TestHarness {
        _tmp: tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        rpc_join,
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
        .header(AUTHORIZATION, format!("Bearer {}", rpc_bearer()))
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

async fn configure_mock_provider(rpc_base: &str, mock_base: &str) {
    let update = rpc(
        rpc_base,
        100,
        "openhuman.inference_update_model_settings",
        json!({
            "api_url": mock_base,
            "default_model": "worker-b-chat",
            "default_temperature": 0.25,
            "cloud_providers": [
                {
                    "slug": "mock",
                    "label": "Mock Provider",
                    "endpoint": format!("{mock_base}/v1"),
                    "auth_style": "none",
                    "default_model": "worker-b-chat"
                },
                {
                    "slug": "mock-404",
                    "label": "Mock Missing Models",
                    "endpoint": format!("{mock_base}/v1/missing-models"),
                    "auth_style": "none",
                    "default_model": "worker-b-chat"
                }
            ],
            "chat_provider": "mock:worker-b-chat"
        }),
    )
    .await;
    ok(&update, "inference_update_model_settings");
}

async fn seed_session_token() {
    let config = Config::load_or_init()
        .await
        .expect("load config for auth seed");
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "worker-b-session-token",
            HashMap::from([("user_id".to_string(), "worker-b-user".to_string())]),
            true,
        )
        .expect("seed session token");
}

#[tokio::test]
async fn inference_provider_success_paths_use_mock_models_and_chat() {
    let _lock = env_lock_async().await;
    let mock = serve_mock().await;
    let harness = setup().await;
    configure_mock_provider(&harness.rpc_base, &mock.base).await;
    seed_session_token().await;

    let models = rpc(
        &harness.rpc_base,
        101,
        "openhuman.inference_list_models",
        json!({ "provider_id": "mock" }),
    )
    .await;
    let model_ids = payload(&models, "inference_list_models")
        .get("models")
        .and_then(Value::as_array)
        .expect("models array")
        .iter()
        .filter_map(|model| model.get("id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(
        model_ids.contains(&"worker-b-chat"),
        "mock model should round-trip through provider /models: {models}"
    );

    // PR #2959 reverted the list_models 404 suppression: a 404 from /models
    // now surfaces as a JSON-RPC error instead of a synthetic `unsupported:
    // true` success, so the failure fires to Sentry for a root-cause fix.
    let models_404 = rpc(
        &harness.rpc_base,
        102,
        "openhuman.inference_list_models",
        json!({ "provider_id": "mock-404" }),
    )
    .await;
    assert!(
        error_message(&models_404, "inference_list_models 404").contains("provider returned 404"),
        "404 list_models should surface as an error: {models_404}"
    );

    let reply = rpc(
        &harness.rpc_base,
        103,
        "openhuman.inference_test_provider_model",
        json!({
            "workload": "chat",
            "provider": "mock:worker-b-chat",
            "prompt": "hello from raw coverage"
        }),
    )
    .await;
    assert_eq!(
        payload(&reply, "inference_test_provider_model")
            .get("reply")
            .and_then(Value::as_str),
        Some("mock provider reply from worker b")
    );

    let seen = mock.state.requests.lock().expect("requests lock").clone();
    assert!(
        seen.iter()
            .any(|entry| entry.get("path").and_then(Value::as_str) == Some("/v1/chat/completions")),
        "provider chat completion should hit the mock server: {seen:?}"
    );

    harness.rpc_join.abort();
    mock.join.abort();
}

#[tokio::test]
async fn tools_web_answer_uses_managed_gemini_grounding_and_returns_citations() {
    if openhuman_core::modules::local_artifact("tinysearch").is_none() {
        eprintln!(
            "SKIPPED (not run, not asserted): no tinysearch library. Run \
             `node scripts/modules/build.mjs tinysearch` and set OPENHUMAN_MODULE_SOURCE_DIR, \
             or use scripts/test-rust-with-mock.sh"
        );
        return;
    }
    let _lock = env_lock_async().await;
    let mock = serve_mock().await;
    let harness = setup().await;
    configure_mock_provider(&harness.rpc_base, &mock.base).await;
    seed_session_token().await;

    let settings = rpc(
        &harness.rpc_base,
        210,
        "openhuman.config_update_search_settings",
        json!({
            "enabled": true,
            "providers": {"gemini": {"enabled": true, "route": "managed"}},
            "roles": {"answer": ["gemini"]}
        }),
    )
    .await;
    let settings = payload(&settings, "config_update_search_settings");
    assert_eq!(
        settings
            .pointer("/effective_roles/answer/0")
            .and_then(Value::as_str),
        Some("gemini"),
        "managed Gemini should serve the answer role: {settings}"
    );

    let answer = rpc(
        &harness.rpc_base,
        211,
        "openhuman.tools_web_answer",
        json!({ "query": "who maintains worker b" }),
    )
    .await;
    let answer = payload(&answer, "tools_web_answer");
    assert_eq!(
        answer.get("provider").and_then(Value::as_str),
        Some("Gemini")
    );
    assert_eq!(answer.get("role").and_then(Value::as_str), Some("answer"));
    assert!(answer
        .get("answer")
        .and_then(Value::as_str)
        .is_some_and(|text| text.contains("Worker B grounded answer")));
    assert_eq!(
        answer.pointer("/citations/0/url").and_then(Value::as_str),
        Some("https://example.com/grounded")
    );

    let seen = mock.state.requests.lock().expect("requests lock").clone();
    let body = seen
        .iter()
        .find(|entry| {
            entry
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| path.starts_with("/agent-integrations/gemini/models/"))
        })
        .and_then(|entry| entry.get("body"))
        .expect("managed Gemini request body");
    assert!(
        body.pointer("/tools/0/googleSearch").is_some(),
        "grounded answers must request Google Search grounding: {body}"
    );

    harness.rpc_join.abort();
    mock.join.abort();
}

#[tokio::test]
async fn tools_web_search_success_path_uses_backend_session_and_shapes_results() {
    // The search RPC runs through the TinySearch module; CI and
    // scripts/test-rust-with-mock.sh build it and export its path.
    if openhuman_core::modules::local_artifact("tinysearch").is_none() {
        eprintln!(
            "SKIPPED (not run, not asserted): no tinysearch library. Run \
             `node scripts/modules/build.mjs tinysearch` and set OPENHUMAN_MODULE_SOURCE_DIR, \
             or use scripts/test-rust-with-mock.sh"
        );
        return;
    }
    let _lock = env_lock_async().await;
    let mock = serve_mock().await;
    let harness = setup().await;
    configure_mock_provider(&harness.rpc_base, &mock.base).await;
    seed_session_token().await;

    let settings = rpc(
        &harness.rpc_base,
        200,
        "openhuman.config_update_search_settings",
        json!({
            "enabled": true,
            "providers": {"exa": {"enabled": true, "route": "managed"}},
            "roles": {"search": ["exa"]}
        }),
    )
    .await;
    let settings = payload(&settings, "config_update_search_settings");
    assert_eq!(
        settings
            .pointer("/effective_roles/search/0")
            .and_then(Value::as_str),
        Some("exa"),
        "managed Exa should serve the search role once signed in: {settings}"
    );

    let search = rpc(
        &harness.rpc_base,
        201,
        "openhuman.tools_web_search",
        json!({
            "query": "worker b raw coverage",
            "max_results": 99
        }),
    )
    .await;
    let search = payload(&search, "tools_web_search");
    assert_eq!(search.get("provider").and_then(Value::as_str), Some("Exa"));
    let results = search
        .get("results")
        .and_then(Value::as_array)
        .expect("results array");
    assert_eq!(
        results[0].get("url").and_then(Value::as_str),
        Some("https://example.com/worker-b")
    );

    let seen = mock.state.requests.lock().expect("requests lock").clone();
    let body = seen
        .iter()
        .find(|entry| {
            entry.get("path").and_then(Value::as_str) == Some("/agent-integrations/exa/search")
        })
        .and_then(|entry| entry.get("body"))
        .expect("managed Exa search request body");
    assert_eq!(
        body.pointer("/searchQueries/0").and_then(Value::as_str),
        Some("worker b raw coverage")
    );
    assert!(
        body.get("mode").is_none(),
        "backend Exa rejects `mode`: {body}"
    );

    harness.rpc_join.abort();
    mock.join.abort();
}

#[tokio::test]
async fn approval_gate_rpc_decision_resumes_parked_tool_and_records_execution() {
    let _lock = env_lock_async().await;
    let harness = setup().await;
    let config = Config::load_or_init()
        .await
        .expect("load config for approval gate");
    let gate = ApprovalGate::init_global(config, format!("session-{}", uuid::Uuid::new_v4()));
    let gate_for_task = gate.clone();

    let approval_task = tokio::spawn(async move {
        // Scope a WebChat origin alongside the chat context — the gate now
        // requires an origin label or it fails closed on `Unknown`.
        turn_origin::with_origin(
            AgentTurnOrigin::WebChat {
                thread_id: "worker-b-thread".to_string(),
                client_id: "worker-b-client".to_string(),
                request_id: None,
            },
            async move {
                let turn = ApprovalTurnContext::chat(ApprovalChatContext {
                    thread_id: "worker-b-thread".to_string(),
                    client_id: "worker-b-client".to_string(),
                    // No turn in scope in this fixture; the field is documented as
                    // carried only when the caller has one (`gate.rs:91-95`).
                    request_id: None,
                });
                gate_for_task
                    .intercept_audited(
                        &turn,
                        "tools.web_search",
                        "search the web for coverage",
                        json!({ "query": "<redacted>", "max_results": 3 }),
                    )
                    .await
            },
        )
        .await
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    let request_id = loop {
        let pending = rpc(
            &harness.rpc_base,
            401,
            "openhuman.approval_list_pending",
            json!({}),
        )
        .await;
        let rows = payload(&pending, "approval_list_pending")
            .as_array()
            .expect("pending rows array");
        if let Some(row) = rows
            .iter()
            .find(|row| row.get("tool_name").and_then(Value::as_str) == Some("tools.web_search"))
        {
            break row
                .get("request_id")
                .and_then(Value::as_str)
                .expect("request_id")
                .to_string();
        }
        assert!(
            Instant::now() < deadline,
            "approval request did not appear before timeout"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    assert_eq!(
        gate.pending_for_thread("worker-b-thread").as_deref(),
        Some(request_id.as_str())
    );

    let decided = rpc(
        &harness.rpc_base,
        402,
        "openhuman.approval_decide",
        json!({
            "request_id": request_id,
            "decision": "approve_once"
        }),
    )
    .await;
    assert_eq!(
        payload(&decided, "approval_decide")
            .get("tool_name")
            .and_then(Value::as_str),
        Some("tools.web_search")
    );

    let (outcome, audit_id) = approval_task.await.expect("approval task join");
    assert!(matches!(outcome, GateOutcome::Allow));
    let audit_id = audit_id.expect("approved audited request id");
    gate.record_execution(&audit_id, ExecutionOutcome::Success, None);
    assert!(
        gate.pending_for_thread("worker-b-thread").is_none(),
        "thread mapping should be cleared after decision"
    );

    let recent = rpc(
        &harness.rpc_base,
        403,
        "openhuman.approval_list_recent_decisions",
        json!({ "limit": 10 }),
    )
    .await;
    let rows = payload(&recent, "approval_list_recent_decisions")
        .as_array()
        .expect("recent decisions array");
    assert!(
        rows.iter().any(|row| {
            row.get("request_id").and_then(Value::as_str) == Some(audit_id.as_str())
                && row.get("decision").and_then(Value::as_str) == Some("approve_once")
        }),
        "recent decisions should include the approved request: {recent}"
    );

    harness.rpc_join.abort();
}
