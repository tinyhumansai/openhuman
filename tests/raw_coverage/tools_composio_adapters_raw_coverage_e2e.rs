//! Round19 raw/E2E coverage for tools-side Composio adapters and adjacent
//! network-tool registration paths.
//!
//! This stays on loopback mocks and temp config/workspaces. The goal is to
//! exercise the same public surfaces the desktop shell and agent registry use
//! without reaching real Composio endpoints.

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

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::security::{AuditLogger, SecurityPolicy};
use openhuman_core::openhuman::tools::{
    all_tools, all_tools_registered_controllers, ComposioExecuteTool, Tool,
};

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: Method,
    path: String,
    query: String,
    body: Value,
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
async fn round19_all_tools_registers_composio_only_when_adapters_are_available() {
    let _lock = env_lock();
    let harness = setup_config().await;
    let security = Arc::new(SecurityPolicy::default());

    let unsigned = all_tools(
        Arc::new(harness.config.clone()),
        &security,
        AuditLogger::disabled(),
        &harness.config.browser,
        &harness.config.http_request,
        &harness.config.workspace_dir,
        &HashMap::new(),
        &harness.config,
    );
    let unsigned_names = tool_names(&unsigned);
    assert!(!unsigned_names.contains(&"composio_execute".to_string()));

    store_session_token(&harness.config);
    let enabled = harness.config.clone();

    let signed = all_tools(
        Arc::new(enabled.clone()),
        &security,
        AuditLogger::disabled(),
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
    // The Polymarket tool was deleted with the `prediction-markets` feature.
    // Assert its absence so a revert cannot quietly re-register it.
    assert!(!names.contains(&"polymarket".to_string()));
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
