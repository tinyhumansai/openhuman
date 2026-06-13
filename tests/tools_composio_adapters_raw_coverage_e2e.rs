//! Round19 raw/E2E coverage for tools-side Composio adapters and adjacent
//! network-tool registration paths.
//!
//! This stays on loopback mocks and temp config/workspaces. The goal is to
//! exercise the same public surfaces the desktop shell and agent registry use
//! without reaching real Composio or Polymarket endpoints.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use async_trait::async_trait;
use axum::body::{to_bytes, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use tempfile::{Builder, TempDir};

use openhuman_core::openhuman::config::{Config, PolymarketClobCredentials};
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::security::{AuditLogger, SecurityPolicy};
use openhuman_core::openhuman::tools::{
    all_tools, all_tools_registered_controllers, ComposioExecuteTool, PolymarketTool, Tool,
};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: Method,
    path: String,
    query: String,
    body: Value,
    poly_api_key: Option<String>,
}

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    market_failures_left: Arc<Mutex<usize>>,
}

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set_path(key: &'static str, path: &Path) -> Self {
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

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

struct Harness {
    _tmp: TempDir,
    config: Config,
    _guards: Vec<EnvGuard>,
}

struct StubMemory;

#[async_trait]
impl Memory for StubMemory {
    fn name(&self) -> &str {
        "round19-stub"
    }

    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(&self) -> Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("target dir");
    Builder::new()
        .prefix("tools-composio-adapters-round19-")
        .tempdir_in("target")
        .expect("round19 tempdir")
}

async fn setup_config() -> Harness {
    let tmp = tempdir();
    let root = tmp.path().join("openhuman");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let guards = vec![
        EnvGuard::set_path("OPENHUMAN_WORKSPACE", &root),
        EnvGuard::set_path("HOME", tmp.path()),
        EnvGuard::unset("BACKEND_URL"),
        EnvGuard::unset("VITE_BACKEND_URL"),
        EnvGuard::unset("OPENHUMAN_API_URL"),
        EnvGuard::unset("OPENHUMAN_CORE_RPC_URL"),
        EnvGuard::unset("OPENHUMAN_CORE_PORT"),
        EnvGuard::unset("OPENHUMAN_LSP_ENABLED"),
    ];

    let mut config = Config {
        workspace_dir: workspace,
        config_path: root.join("config.toml"),
        ..Config::default()
    };
    config.node.enabled = false;
    config.secrets.encrypt = false;
    config.observability.analytics_enabled = false;
    config.save().await.expect("save config");

    Harness {
        _tmp: tmp,
        config,
        _guards: guards,
    }
}

fn store_session_token(config: &Config) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round19-session-token",
            HashMap::new(),
            true,
        )
        .expect("store app session token");
}

fn tool_names(tools: &[Box<dyn Tool>]) -> Vec<String> {
    tools.iter().map(|tool| tool.name().to_string()).collect()
}

#[tokio::test]
async fn round19_all_tools_registers_composio_and_polymarket_only_when_adapters_are_available() {
    let _lock = env_lock();
    let harness = setup_config().await;
    let security = Arc::new(SecurityPolicy::default());
    let memory: Arc<dyn Memory> = Arc::new(StubMemory);

    let unsigned = all_tools(
        Arc::new(harness.config.clone()),
        &security,
        AuditLogger::disabled(),
        memory.clone(),
        &harness.config.browser,
        &harness.config.http_request,
        &harness.config.workspace_dir,
        &HashMap::new(),
        &harness.config,
    );
    let unsigned_names = tool_names(&unsigned);
    assert!(!unsigned_names.contains(&"composio_execute".to_string()));
    assert!(!unsigned_names.contains(&"polymarket".to_string()));

    store_session_token(&harness.config);
    let mut enabled = harness.config.clone();
    enabled.integrations.polymarket.enabled = true;
    enabled.integrations.polymarket.gamma_base_url = "http://127.0.0.1:1".into();
    enabled.integrations.polymarket.clob_base_url = "http://127.0.0.1:1".into();
    enabled.integrations.polymarket.polygon_rpc_url = "http://127.0.0.1:1".into();
    enabled.integrations.polymarket.derived_clob_credentials = Some(fixture_clob_credentials());

    let signed = all_tools(
        Arc::new(enabled.clone()),
        &security,
        AuditLogger::disabled(),
        memory,
        &enabled.browser,
        &enabled.http_request,
        &enabled.workspace_dir,
        &HashMap::new(),
        &enabled,
    );
    let names = tool_names(&signed);
    assert!(names.contains(&"composio_execute".to_string()));
    assert!(names.contains(&"composio_list_tools".to_string()));
    assert!(names.contains(&"composio_authorize".to_string()));
    assert!(names.contains(&"polymarket".to_string()));
}

#[tokio::test]
async fn round19_composio_agent_execute_tool_uses_backend_adapter_and_preserves_provider_errors() {
    let _lock = env_lock();
    let state = MockState::default();
    let base = start_loopback(
        Router::new()
            .fallback(any(composio_handler))
            .with_state(state.clone()),
    )
    .await;
    let mut harness = setup_config().await;
    harness.config.api_url = Some(base);
    harness.config.save().await.expect("save backend config");
    store_session_token(&harness.config);

    let tool = ComposioExecuteTool::new(Arc::new(harness.config.clone()));
    let ok = tool
        .execute(json!({
            "tool": "GMAIL_FETCH_EMAILS",
            "arguments": { "query": "from:round19" },
            "connection_id": "conn-gmail"
        }))
        .await
        .expect("execute ok");
    assert!(!ok.is_error);
    assert_eq!(ok.text(), "round19 markdown");

    let provider_error = tool
        .execute(json!({
            "tool": "GMAIL_SEND_EMAIL",
            "arguments": { "to": "nobody@example.test" }
        }))
        .await
        .expect("execute provider error");
    assert!(provider_error.text().contains("provider refused round19"));

    let bad_args = tool
        .execute(json!({ "tool": "GMAIL_FETCH_EMAILS", "arguments": [] }))
        .await
        .expect("bad args are tool result");
    assert!(bad_args.is_error);
    assert!(bad_args.text().contains("arguments"));

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().any(|request| {
        request.method == Method::POST
            && request.path == "/agent-integrations/composio/execute"
            && request.body.to_string().contains("GMAIL_FETCH_EMAILS")
    }));
}

#[tokio::test]
async fn round19_polymarket_controller_and_tool_cover_retry_signed_reads_and_validation() {
    let _lock = env_lock();
    let state = MockState::default();
    *state.market_failures_left.lock().expect("failures") = 2;
    let base = start_loopback(
        Router::new()
            .fallback(any(polymarket_handler))
            .with_state(state.clone()),
    )
    .await;
    let mut harness = setup_config().await;
    configure_polymarket(&mut harness.config, &base);
    harness.config.save().await.expect("save polymarket config");

    let tool = PolymarketTool::new(
        &harness.config.integrations.polymarket,
        Arc::new(SecurityPolicy::default()),
    );
    let retried = tool
        .execute(json!({ "action": "list_markets", "limit": 3, "active": true }))
        .await
        .expect("retried list markets");
    assert!(!retried.is_error);
    assert!(retried.output().contains("round19-market"));

    let signed = tool
        .execute(json!({
            "action": "get_balance",
            "user": "0x1111111111111111111111111111111111111111"
        }))
        .await
        .expect("signed balance");
    assert!(!signed.is_error);
    assert!(signed.output().contains("42.00"));

    let invalid = tool
        .execute(json!({ "action": "get_orderbook", "token_id": " " }))
        .await
        .expect("invalid token id");
    assert!(invalid.is_error);
    assert!(invalid.output().contains("token_id"));

    let controller = all_tools_registered_controllers()
        .into_iter()
        .find(|controller| controller.schema.function == "polymarket_execute")
        .expect("polymarket controller");
    let controller_result = (controller.handler)(Map::from_iter([
        ("action".to_string(), json!("get_price")),
        (
            "arguments".to_string(),
            json!({ "token_id": "token-round19", "side": "sell" }),
        ),
    ]))
    .await
    .expect("controller get_price");
    assert!(controller_result
        .pointer("/result/data")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .contains("get_price"));

    let bad_shape = (controller.handler)(Map::from_iter([
        ("action".to_string(), json!("get_price")),
        ("arguments".to_string(), json!("not-an-object")),
    ]))
    .await
    .expect_err("controller rejects non-object arguments");
    assert!(bad_shape.contains("arguments"));

    harness.config.integrations.polymarket.enabled = false;
    harness
        .config
        .save()
        .await
        .expect("save disabled polymarket");
    let disabled = (controller.handler)(Map::from_iter([(
        "action".to_string(),
        json!("list_markets"),
    )]))
    .await
    .expect_err("controller disabled");
    assert!(disabled.contains("disabled"));

    let requests = state.requests.lock().expect("requests").clone();
    let market_gets = requests
        .iter()
        .filter(|request| request.method == Method::GET && request.path == "/markets")
        .count();
    assert_eq!(
        market_gets, 3,
        "expected two retries then success: {requests:?}"
    );
    assert!(requests.iter().any(|request| {
        request.path == "/data/balance" && request.poly_api_key.as_deref() == Some("round19-key")
    }));
    assert!(requests.iter().any(|request| {
        request.path == "/price"
            && request.query.contains("token_id=token-round19")
            && request.query.to_ascii_uppercase().contains("SIDE=SELL")
    }));
}

async fn start_loopback(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("loopback addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve loopback");
    });
    format!("http://127.0.0.1:{}", addr.port())
}

async fn composio_handler(State(state): State<MockState>, request: Request) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or_default().to_string();
    let bytes = to_bytes(request.into_body(), usize::MAX)
        .await
        .expect("request body");
    let body: Value = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).expect("json body")
    };
    state
        .requests
        .lock()
        .expect("requests")
        .push(RecordedRequest {
            method: method.clone(),
            path: path.clone(),
            query,
            body: body.clone(),
            poly_api_key: None,
        });

    match (method, path.as_str()) {
        (Method::POST, "/agent-integrations/composio/execute") => {
            match body.get("tool").and_then(Value::as_str) {
                Some("GMAIL_FETCH_EMAILS") => ok(json!({
                    "successful": true,
                    "data": { "messages": [{ "id": "round19-msg" }] },
                    "error": null,
                    "costUsd": 0.01,
                    "markdownFormatted": "round19 markdown"
                })),
                Some("GMAIL_SEND_EMAIL") => ok(json!({
                    "successful": false,
                    "data": {},
                    "error": "provider refused round19",
                    "costUsd": 0.0,
                    "markdownFormatted": null
                })),
                other => fail(
                    StatusCode::BAD_REQUEST,
                    &format!("unexpected composio tool: {other:?}"),
                ),
            }
        }
        _ => fail(StatusCode::NOT_FOUND, &format!("unhandled composio {path}")),
    }
}

async fn polymarket_handler(
    State(state): State<MockState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or_default().to_string();
    let body_text = String::from_utf8_lossy(&body);
    let body_json = serde_json::from_str::<Value>(&body_text).unwrap_or_else(|_| json!(body_text));
    let poly_api_key = headers
        .get("poly_api_key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    state
        .requests
        .lock()
        .expect("requests")
        .push(RecordedRequest {
            method: method.clone(),
            path: path.clone(),
            query,
            body: body_json,
            poly_api_key,
        });

    match (method, path.as_str()) {
        (Method::GET, "/markets") => {
            let mut failures = state.market_failures_left.lock().expect("failures");
            if *failures > 0 {
                *failures -= 1;
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "retry me round19" })),
                )
                    .into_response();
            }
            Json(json!([{ "id": "m-round19", "slug": "round19-market" }])).into_response()
        }
        (Method::GET, "/price") => Json(json!({ "price": "0.37" })).into_response(),
        (Method::GET, "/data/balance") => Json(json!({ "balance": "42.00" })).into_response(),
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("unhandled polymarket {path}") })),
        )
            .into_response(),
    }
}

fn configure_polymarket(config: &mut Config, base: &str) {
    config.integrations.polymarket.enabled = true;
    config.integrations.polymarket.gamma_base_url = base.to_string();
    config.integrations.polymarket.clob_base_url = base.to_string();
    config.integrations.polymarket.polygon_rpc_url = base.to_string();
    config.integrations.polymarket.timeout_secs = 2;
    config.integrations.polymarket.eoa_address =
        Some("0x1111111111111111111111111111111111111111".to_string());
    config.integrations.polymarket.derived_clob_credentials = Some(fixture_clob_credentials());
}

fn fixture_clob_credentials() -> PolymarketClobCredentials {
    PolymarketClobCredentials {
        api_key: "round19-key".to_string(),
        secret: "cm91bmQxOS1zZWNyZXQ=".to_string(),
        passphrase: "round19-pass".to_string(),
    }
}

fn ok(data: Value) -> Response {
    Json(json!({ "success": true, "data": data })).into_response()
}

fn fail(status: StatusCode, error: &str) -> Response {
    (
        status,
        Json(json!({ "success": false, "error": error.to_string() })),
    )
        .into_response()
}
