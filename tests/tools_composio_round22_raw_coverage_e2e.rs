//! Round22 raw coverage for high-miss tool and Composio branches.
//!
//! All outbound HTTP stays on loopback mocks. The tests drive public tool
//! surfaces so coverage lands on the same code paths used by agent/tool calls.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use async_trait::async_trait;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};

use openhuman_core::openhuman::config::{Config, DelegateAgentConfig};
use openhuman_core::openhuman::cron::DeliveryConfig;
use openhuman_core::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary};
use openhuman_core::openhuman::security::{AuditLogger, SecurityPolicy};
use openhuman_core::openhuman::tools::{
    all_tools, ComposioTool, CronAddTool, TodoTool, Tool, ToolCallOptions,
};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: Method,
    path: String,
    query: String,
    api_key: Option<String>,
}

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
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
    workspace: PathBuf,
    config: Config,
    _guards: Vec<EnvGuard>,
}

struct StubMemory;

#[async_trait]
impl Memory for StubMemory {
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
        _opts: openhuman_core::openhuman::memory::RecallOpts<'_>,
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

    fn name(&self) -> &str {
        "round22-memory"
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
        .prefix("tools-composio-round22-")
        .tempdir_in("target")
        .expect("round22 tempdir")
}

async fn setup() -> Harness {
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
        EnvGuard::unset("OPENHUMAN_BROWSER_ALLOW_ALL"),
    ];

    let mut config = Config {
        workspace_dir: workspace.clone(),
        config_path: root.join("config.toml"),
        ..Config::default()
    };
    config.node.enabled = false;
    config.secrets.encrypt = false;
    config.observability.analytics_enabled = false;
    config.save().await.expect("save config");

    Harness {
        _tmp: tmp,
        workspace,
        config,
        _guards: guards,
    }
}

#[tokio::test]
async fn round22_direct_composio_tool_covers_summary_and_validation_edges() {
    let _lock = env_lock();
    let state = MockState::default();
    let base = start_loopback(
        Router::new()
            .fallback(any(composio_direct_handler))
            .with_state(state.clone()),
    )
    .await;
    let tool = ComposioTool::new_with_base_urls_for_loopback(
        " ck_round22 ",
        Some(" entity-round22 "),
        Arc::new(SecurityPolicy::default()),
        format!("{base}/api/v2"),
        format!("{base}/api/v3"),
    )
    .expect("loopback composio tool");

    assert!(tool.external_effect_with_args(&json!({})));
    assert!(tool.external_effect_with_args(&json!({ "action": "wat" })));

    let listed = tool
        .execute(json!({ "action": "list", "app": "bulk" }))
        .await
        .expect("list many actions");
    assert!(!listed.is_error);
    assert!(listed.output().contains("Found 22 available actions"));
    assert!(listed.output().contains("... and 2 more"));
    assert!(listed.output().contains("bulk-action-00"));
    assert!(!listed.output().contains("bulk-action-21"));

    let missing_execute_slug = tool
        .execute(json!({ "action": "execute", "params": {} }))
        .await
        .expect_err("missing execute slug is an anyhow validation error");
    assert!(missing_execute_slug
        .to_string()
        .contains("Missing 'action_name'"));

    let missing_connect_target = tool
        .execute(json!({ "action": "connect" }))
        .await
        .expect_err("missing connect target is an anyhow validation error");
    assert!(missing_connect_target
        .to_string()
        .contains("Missing 'app' or 'auth_config_id'"));

    let bad_json = tool
        .list_actions(Some("bad-json"))
        .await
        .expect_err("bad json fails v3 and v2")
        .to_string();
    assert!(bad_json.contains("Failed to decode Composio v3 tools response"));
    assert!(bad_json.contains("v2 fallback"));

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().all(|request| {
        request.api_key.as_deref() == Some("ck_round22") || request.path == "/health"
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::GET
            && request.path == "/api/v3/tools"
            && request.query.contains("toolkits=bulk")
            && request.query.contains("toolkit_slug=bulk")
    }));
}

#[tokio::test]
async fn round22_todo_tool_covers_crud_and_patch_error_branches() {
    let _lock = env_lock();
    let _harness = setup().await;
    let tool = TodoTool::new();

    let _ = tool.execute(json!({ "op": "clear" })).await;

    let added = tool
        .execute(json!({
            "op": "add",
            "content": "Ship round22 coverage",
            "status": "pending",
            "objective": "Raise coverage on tools",
            "plan": ["write test", "run validation"],
            "assignedAgent": "coverage_worker",
            "allowedTools": ["cargo", "composio"],
            "approvalMode": null,
            "acceptanceCriteria": ["tests pass"],
            "evidence": ["target/tools-composio-round22-focused-lcov.info"],
            "notes": "scratch board"
        }))
        .await
        .expect("add todo card");
    assert!(!added.is_error, "{}", added.output());
    let payload: Value = serde_json::from_str(&added.output()).expect("todo json");
    let id = payload["cards"][0]["id"]
        .as_str()
        .expect("todo id")
        .to_string();
    assert!(payload["markdown"]
        .as_str()
        .unwrap_or_default()
        .contains("Ship round22 coverage"));

    let edited = tool
        .execute(json!({
            "op": "edit",
            "id": id,
            "content": "Ship focused round22 coverage",
            "status": "blocked",
            "blocker": "waiting on validation",
            "approvalMode": "required"
        }))
        .await
        .expect("edit todo card");
    assert!(!edited.is_error, "{}", edited.output());
    assert!(edited.output().contains("waiting on validation"));

    let removed = tool
        .execute(json!({ "op": "remove", "id": id }))
        .await
        .expect("remove todo card");
    assert!(!removed.is_error, "{}", removed.output());

    let bad_array = tool
        .execute(json!({
            "op": "add",
            "content": "bad plan",
            "plan": ["ok", 42]
        }))
        .await
        .expect_err("non-string plan entries are validation errors");
    assert!(bad_array
        .to_string()
        .contains("`plan` must be an array of strings"));

    let bad_approval = tool
        .execute(json!({
            "op": "add",
            "content": "bad approval",
            "approvalMode": "sometimes"
        }))
        .await
        .expect_err("bad approval mode is validation error");
    assert!(bad_approval.to_string().contains("invalid approvalMode"));

    let invalid_replace = tool
        .execute(json!({ "op": "replace", "cards": [{ "id": 7 }] }))
        .await
        .expect_err("invalid replace bubbles validation error");
    assert!(invalid_replace.to_string().contains("invalid `cards`"));

    let unknown = tool
        .execute(json!({ "op": "sort" }))
        .await
        .expect("unknown op returns tool result");
    assert!(unknown.is_error);
    assert!(unknown.output().contains("unknown op"));

    let _ = tool.execute(json!({ "op": "clear" })).await;
}

#[tokio::test]
async fn round22_cron_add_tool_covers_validation_and_markdown_edges() {
    let _lock = env_lock();
    let mut harness = setup().await;
    let security = Arc::new(SecurityPolicy::from_config(
        &harness.config.autonomy,
        &harness.config.workspace_dir,
    ));

    harness.config.cron.enabled = false;
    let disabled = CronAddTool::new(Arc::new(harness.config.clone()), security.clone())
        .execute(json!({}))
        .await
        .expect("disabled cron");
    assert!(disabled.is_error);
    assert!(disabled.output().contains("cron is disabled"));

    harness.config.cron.enabled = true;
    let tool = CronAddTool::new(Arc::new(harness.config.clone()), security);

    let missing_schedule = tool
        .execute(json!({ "command": "echo ok" }))
        .await
        .expect("missing schedule");
    assert!(missing_schedule.is_error);
    assert!(missing_schedule.output().contains("Missing 'schedule'"));

    let invalid_job_type = tool
        .execute(json!({
            "name": "bad_type",
            "schedule": { "kind": "every", "every_ms": 60000 },
            "job_type": "timer",
            "command": "echo ok"
        }))
        .await
        .expect("invalid job type");
    assert!(invalid_job_type.is_error);
    assert!(invalid_job_type.output().contains("Invalid job_type"));

    let missing_command = tool
        .execute(json!({
            "name": "missing_command",
            "schedule": { "kind": "every", "every_ms": 60000 },
            "job_type": "shell"
        }))
        .await
        .expect("missing command");
    assert!(missing_command.is_error);
    assert!(missing_command.output().contains("Missing 'command'"));

    let invalid_delivery = tool
        .execute(json!({
            "name": "bad_delivery",
            "schedule": { "kind": "every", "every_ms": 60000 },
            "job_type": "agent",
            "prompt": "summarize",
            "delivery": { "mode": "announce", "channel": "telegram" }
        }))
        .await
        .expect("invalid delivery");
    assert!(invalid_delivery.is_error);
    assert!(invalid_delivery
        .output()
        .contains("delivery.to is required"));

    let created = tool
        .execute_with_options(
            json!({
                "name": "round22_agent_once",
                "schedule": { "kind": "at", "at": "2026-05-31T00:00:00Z" },
                "job_type": "agent",
                "prompt": "collect validation notes",
                "session_target": "main",
                "model": "test-model",
                "delivery": { "mode": "none" }
            }),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("create agent cron");
    assert!(!created.is_error, "{}", created.output());
    assert!(created
        .markdown_formatted
        .as_deref()
        .unwrap_or_default()
        .contains("Created cron job"));
    assert!(created.output().contains("\"enabled\": true"));

    let delivery: DeliveryConfig =
        serde_json::from_value(json!({ "mode": "none" })).expect("delivery defaults deserialize");
    assert_eq!(delivery.mode, "none");
}

#[tokio::test]
async fn round22_tool_registry_covers_config_gated_registration() {
    let _lock = env_lock();
    let mut harness = setup().await;
    harness.config.browser.enabled = true;
    harness.config.http_request.allowed_domains = vec![
        "*".to_string(),
        "example.com".to_string(),
        "docs.example.com".to_string(),
    ];
    harness.config.computer_control.enabled = true;
    harness.config.learning.enabled = true;
    harness.config.learning.tool_tracking_enabled = true;
    harness.config.gitbooks.enabled = true;
    harness.config.node.enabled = false;

    let security = Arc::new(SecurityPolicy::from_config(
        &harness.config.autonomy,
        &harness.workspace,
    ));
    let memory: Arc<dyn Memory> = Arc::new(StubMemory);
    let audit = AuditLogger::disabled();
    let agents: HashMap<String, DelegateAgentConfig> = HashMap::new();

    let tools = all_tools(
        Arc::new(harness.config.clone()),
        &security,
        audit,
        memory,
        &harness.config.browser,
        &harness.config.http_request,
        &harness.workspace,
        &agents,
        &harness.config,
    );
    let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();

    assert!(names.contains(&"browser_open"));
    assert!(names.contains(&"browser"));
    assert!(names.contains(&"http_request"));
    assert!(names.contains(&"web_fetch"));
    assert!(names.contains(&"curl"));
    assert!(names.contains(&"gitbooks_search"));
    assert!(names.contains(&"gitbooks_get_page"));
    assert!(names.contains(&"mouse"));
    assert!(names.contains(&"keyboard"));
    assert!(names.contains(&"tool_stats"));
    assert!(!names.contains(&"node_exec"));
    assert!(!names.contains(&"npm_exec"));
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

async fn composio_direct_handler(State(state): State<MockState>, request: Request) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or_default().to_string();
    let api_key = request
        .headers()
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = to_bytes(request.into_body(), usize::MAX)
        .await
        .expect("request body");
    let _body: Value = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)))
    };
    state
        .requests
        .lock()
        .expect("requests")
        .push(RecordedRequest {
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            api_key,
        });

    match (method, path.as_str()) {
        (Method::GET, "/api/v3/tools") if query.contains("toolkits=bulk") => {
            let items: Vec<Value> = (0..22)
                .map(|idx| {
                    json!({
                        "slug": format!("bulk-action-{idx:02}"),
                        "name": format!("Bulk action {idx:02}"),
                        "description": format!("Bulk description {idx:02}"),
                        "toolkit": { "slug": "bulk" },
                    })
                })
                .collect();
            ok(json!({ "items": items }))
        }
        (Method::GET, "/api/v3/tools") if query.contains("toolkits=bad-json") => {
            text(StatusCode::OK, "{not-json")
        }
        (Method::GET, "/api/v2/actions") if query.contains("bad-json") => {
            fail(StatusCode::BAD_GATEWAY, "v2 unavailable")
        }
        (Method::GET, "/api/v2/actions") => ok(json!({ "items": [] })),
        _ => fail(StatusCode::NOT_FOUND, &format!("unhandled {path}")),
    }
}

fn ok(value: Value) -> Response {
    Json(value).into_response()
}

fn fail(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": { "message": message } }))).into_response()
}

fn text(status: StatusCode, body: &str) -> Response {
    (status, body.to_string()).into_response()
}
