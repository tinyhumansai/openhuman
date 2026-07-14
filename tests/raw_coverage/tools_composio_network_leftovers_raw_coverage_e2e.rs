//! Round20 raw/E2E coverage for Composio tool leftovers and adjacent
//! network-tool branches. All HTTP traffic stays on loopback mocks.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::{to_bytes, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};

use openhuman_core::openhuman::composio::ops::{composio_authorize, composio_list_tools};
use openhuman_core::openhuman::config::{Config, PolymarketClobCredentials};
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::security::SecurityPolicy;
use openhuman_core::openhuman::tools::{
    ComposioAuthorizeTool, ComposioListConnectionsTool, ComposioListToolkitsTool,
    ComposioListToolsTool, ComposioTool, PolymarketTool, SpawnSubagentTool, Tool, ToolCallOptions,
};

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: Method,
    path: String,
    query: String,
    body: Value,
    api_key: Option<String>,
}

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    connections_fail: Arc<Mutex<bool>>,
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

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("target dir");
    Builder::new()
        .prefix("tools-composio-network-leftovers-round20-")
        .tempdir_in("target")
        .expect("round20 tempdir")
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
            "round20-session-token",
            HashMap::new(),
            true,
        )
        .expect("store app session token");
}

#[tokio::test]
async fn round20_backend_agent_tools_cover_markdown_filtering_and_errors() {
    let _lock = env_lock();
    let state = MockState::default();
    let base = start_loopback(
        Router::new()
            .fallback(any(composio_backend_handler))
            .with_state(state.clone()),
    )
    .await;
    let mut harness = setup_config().await;
    harness.config.api_url = Some(base);
    harness.config.save().await.expect("save backend config");
    store_session_token(&harness.config);

    let config = Arc::new(harness.config.clone());
    let list_toolkits = ComposioListToolkitsTool::new(config.clone());
    let toolkits = list_toolkits
        .execute(json!({}))
        .await
        .expect("list toolkits");
    assert!(!toolkits.is_error);
    assert!(toolkits.output().contains("gmail"));

    let connections_tool = ComposioListConnectionsTool::new(config.clone());
    let connections = connections_tool
        .execute(json!({}))
        .await
        .expect("list connections");
    assert!(!connections.is_error);
    assert!(connections.output().contains("conn-gmail"));
    assert!(!connections.output().contains("conn-pending"));

    let list_tools = ComposioListToolsTool::new(config.clone());
    let markdown = list_tools
        .execute_with_options(
            json!({
                "toolkits": ["gmail", "github"],
                "tags": ["readOnlyHint", "repos"],
                "include_unconnected": true
            }),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("list tools markdown");
    assert!(!markdown.is_error);
    assert!(markdown.output().contains("GMAIL_FETCH_EMAILS"));
    assert!(markdown
        .markdown_formatted
        .as_deref()
        .unwrap_or_default()
        .contains("# Composio tools"));

    let connected_only = list_tools
        .execute(json!({ "toolkits": ["gmail", "github"] }))
        .await
        .expect("list tools connected only");
    assert!(!connected_only.is_error);
    assert!(connected_only.output().contains("GMAIL_FETCH_EMAILS"));
    assert!(!connected_only.output().contains("GITHUB_STAR_REPOSITORY"));

    let unsupported = list_tools
        .execute(json!({
            "toolkits": ["totallycustom"],
            "include_unconnected": true
        }))
        .await
        .expect("unsupported toolkit empty list");
    assert!(unsupported.is_error);
    assert!(unsupported.output().contains("no agent-ready actions"));

    *state.connections_fail.lock().expect("connections flag") = true;
    let connection_error = list_tools
        .execute(json!({ "toolkits": ["gmail"] }))
        .await
        .expect("connection prefilter error");
    assert!(connection_error.is_error);
    assert!(connection_error
        .output()
        .contains("failed to fetch connections"));

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().any(|request| {
        request.method == Method::GET
            && request.path == "/agent-integrations/composio/tools"
            && request.query.contains("toolkits=gmail")
            && request.query.contains("github")
            && request.query.contains("tags=readOnlyHint")
            && request.query.contains("repos")
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::GET
            && request.path == "/agent-integrations/composio/tools"
            && request.query.contains("toolkits=gmail")
            && request.query.contains("github")
            && !request.query.contains("tags=")
    }));
}

#[tokio::test]
async fn round20_composio_ops_cover_authorize_scopes_and_direct_factory_edges() {
    let _lock = env_lock();
    let state = MockState::default();
    let base = start_loopback(
        Router::new()
            .fallback(any(composio_backend_handler))
            .with_state(state.clone()),
    )
    .await;
    let mut harness = setup_config().await;
    harness.config.api_url = Some(base);
    harness.config.save().await.expect("save backend config");
    store_session_token(&harness.config);

    let bad_extra = composio_authorize(
        &harness.config,
        "gmail",
        Some(json!({ "oauth_scopes": [123] })),
    )
    .await
    .expect_err("bad oauth scope entries rejected before network");
    assert!(bad_extra.contains("oauth_scopes"));

    let authorized = composio_authorize(
        &harness.config,
        " gmail ",
        Some(json!({ "waba_id": "waba-round20" })),
    )
    .await
    .expect("authorize with required gmail scope")
    .value;
    assert_eq!(authorized.connection_id, "conn-authorize");

    let listed = composio_list_tools(
        &harness.config,
        Some(vec!["gmail".into(), "github".into()]),
        Some(vec!["readOnlyHint".into()]),
    )
    .await
    .expect("ops list tools")
    .value;
    assert_eq!(listed.tools.len(), 2);

    let mut direct = harness.config.clone();
    direct.composio.mode = "direct".to_string();
    direct.composio.api_key = Some(" ck_round20_direct ".to_string());
    direct.save().await.expect("save direct config");

    let direct_toolkits = ComposioListToolkitsTool::new(Arc::new(direct.clone()))
        .execute(json!({}))
        .await
        .expect("direct list toolkits");
    assert!(!direct_toolkits.is_error);
    assert!(direct_toolkits.output().contains("\"toolkits\":[]"));

    let direct_authorize = ComposioAuthorizeTool::new(Arc::new(direct.clone()))
        .execute(json!({ "toolkit": "gmail" }))
        .await
        .expect("direct authorize tool");
    assert!(direct_authorize.is_error);
    assert!(direct_authorize.output().contains("direct mode is active"));

    let direct_list_tools = ComposioListToolsTool::new(Arc::new(direct))
        .execute_with_options(
            json!({ "include_unconnected": true }),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("direct list tools tool");
    assert!(!direct_list_tools.is_error);
    assert_eq!(direct_list_tools.output(), "{\"tools\":[]}");
    assert_eq!(
        direct_list_tools.markdown_formatted.as_deref(),
        Some("_No composio tools available._")
    );

    let requests = state.requests.lock().expect("requests").clone();
    let authorize_body = requests
        .iter()
        .find(|request| request.path == "/agent-integrations/composio/authorize")
        .expect("authorize request")
        .body
        .clone();
    assert_eq!(authorize_body["toolkit"], "gmail");
    assert_eq!(authorize_body["waba_id"], "waba-round20");
    assert!(authorize_body["oauth_scopes"]
        .as_array()
        .expect("oauth scopes array")
        .iter()
        .any(|scope| scope
            .as_str()
            .unwrap_or_default()
            .contains("gmail.readonly")));
}

#[tokio::test]
async fn round20_direct_composio_tool_covers_fallback_sanitizing_and_account_edges() {
    let _lock = env_lock();
    let state = MockState::default();
    let base = start_loopback(
        Router::new()
            .fallback(any(composio_direct_handler))
            .with_state(state.clone()),
    )
    .await;
    let tool = ComposioTool::new_with_base_urls_for_loopback(
        " ck_round20 ",
        Some(" entity-round20 "),
        Arc::new(SecurityPolicy::default()),
        format!("{base}/api/v2"),
        format!("{base}/api/v3"),
    )
    .expect("loopback direct composio tool");

    let insecure_base_error = match ComposioTool::new_with_base_urls_for_loopback(
        "ck",
        None,
        Arc::new(SecurityPolicy::default()),
        "http://example.invalid/api/v2".to_string(),
        format!("{base}/api/v3"),
    ) {
        Ok(_) => panic!("non-loopback http refused"),
        Err(error) => error.to_string(),
    };
    assert!(insecure_base_error.contains("loopback HTTP"));

    assert!(!tool.external_effect_with_args(&json!({ "action": "list" })));
    assert!(!tool.external_effect_with_args(&json!({ "action": "connect" })));
    assert!(tool.external_effect_with_args(&json!({ "action": "execute" })));

    let actions = tool
        .list_actions(Some(" gmail "))
        .await
        .expect("v3 list actions");
    assert_eq!(actions.len(), 2);

    let fallback_actions = tool
        .list_actions(Some("fallback"))
        .await
        .expect("v2 action fallback");
    assert_eq!(fallback_actions[0].name, "FALLBACK_V2");

    let failed_list = tool
        .list_actions(Some("broken"))
        .await
        .expect_err("v3 and v2 list fail")
        .to_string();
    assert!(failed_list.contains("v3"));
    assert!(failed_list.contains("v2 fallback"));

    let exec = tool
        .execute(json!({
            "action": "execute",
            "action_name": "GMAIL_FETCH_EMAILS",
            "params": { "query": "label:INBOX" },
            "connected_account_id": "acct-gmail"
        }))
        .await
        .expect("tool execute");
    assert!(!exec.is_error);
    assert!(exec.output().contains("msg-round20"));

    let failed_exec = tool
        .execute_action(
            "BROKEN_ACTION",
            json!({ "user_id": "secret-user", "connected_account_id": "secret-account" }),
            Some("secret-user"),
            Some("secret-account"),
        )
        .await
        .expect_err("execute failure redacts sensitive field names")
        .to_string();
    assert!(failed_exec.contains("[redacted]"));
    assert!(!failed_exec.contains("connected_account_id"));
    assert!(!failed_exec.contains("user_id"));

    let missing_auth_config = tool
        .get_connection_url(Some("missing"), None, "entity-round20")
        .await
        .expect_err("missing auth config");
    assert!(missing_auth_config
        .to_string()
        .contains("No auth config found"));

    let connected_accounts = tool
        .list_connected_accounts()
        .await
        .expect("connected accounts");
    assert_eq!(connected_accounts.len(), 3);
    assert_eq!(
        connected_accounts[0].toolkit_slug().as_deref(),
        Some("gmail")
    );
    assert_eq!(
        connected_accounts[1].toolkit_slug().as_deref(),
        Some("github")
    );
    assert_eq!(
        connected_accounts[2].toolkit_slug().as_deref(),
        Some("slack")
    );

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().all(|request| {
        request.api_key.as_deref() == Some("ck_round20") || request.path == "/health"
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::GET
            && request.path == "/api/v3/tools"
            && request.query.contains("toolkits=gmail")
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::POST
            && request.path == "/api/v3/tools/execute/GMAIL_FETCH_EMAILS"
            && request.body.pointer("/connected_account_id") == Some(&json!("acct-gmail"))
    }));
}

#[tokio::test]
async fn round20_polymarket_covers_discovery_errors_rpc_allowance_and_write_gates() {
    let _lock = env_lock();
    let state = MockState::default();
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
    assert!(tool.is_concurrency_safe(&json!({ "action": "get_price" })));
    assert!(!tool.is_concurrency_safe(&json!({ "action": "place_order" })));
    assert!(!tool.is_concurrency_safe(&json!({ "action": "cancel_order" })));
    assert_eq!(tool.category().to_string(), "skill");
    assert!(tool.description().contains("Polymarket"));
    assert!(tool.parameters_schema()["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .contains(&json!("cancel_order")));

    let by_id = tool
        .execute(json!({ "action": "get_market", "market_id": "m-round20" }))
        .await
        .expect("get market by id");
    assert!(!by_id.is_error);
    assert!(by_id.output().contains("lookup"));

    let missing_slug = tool
        .execute(json!({ "action": "get_market", "slug": "empty-slug" }))
        .await
        .expect("missing slug returns tool error");
    assert!(missing_slug.is_error);
    assert!(missing_slug.output().contains("No Polymarket market found"));

    let event = tool
        .execute(json!({ "action": "list_events", "event_id": "evt-round20" }))
        .await
        .expect("get event by id");
    assert!(!event.is_error);
    assert!(event.output().contains("evt-round20"));

    let allowance = tool
        .execute(json!({
            "action": "get_usdc_allowance",
            "user": "0x1111111111111111111111111111111111111111"
        }))
        .await
        .expect("allowance");
    assert!(!allowance.is_error);
    assert!(allowance.output().contains("1000000"));

    let invalid_side = tool
        .execute(json!({ "action": "get_price", "token_id": "tok", "side": "hold" }))
        .await
        .expect("invalid side");
    assert!(invalid_side.is_error);
    assert!(invalid_side.output().contains("Invalid 'side'"));

    let unapproved_write = tool
        .execute(json!({
            "action": "cancel_order",
            "order_id": "order-round20",
            "user": "0x1111111111111111111111111111111111111111"
        }))
        .await
        .expect("unapproved write");
    assert!(unapproved_write.is_error);
    assert!(unapproved_write.output().contains("explicit user approval"));

    let invalid_request = tool
        .execute(json!({ "action": "not_real" }))
        .await
        .expect_err("invalid request shape returns error")
        .to_string();
    assert!(invalid_request.contains("Invalid polymarket request"));

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().any(|request| {
        request.method == Method::POST && (request.path.is_empty() || request.path == "/")
    }));
    assert!(requests
        .iter()
        .any(|request| request.method == Method::GET && request.path == "/events/evt-round20"));
}

#[tokio::test]
async fn round20_spawn_subagent_covers_validation_schema_and_disabled_worker_branch() {
    let _lock = env_lock();
    let tool = SpawnSubagentTool::new();

    assert_eq!(tool.name(), "spawn_subagent");
    assert_eq!(tool.permission_level().to_string(), "Execute");
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["toolkit"]
        .as_object()
        .expect("toolkit schema")
        .contains_key("description"));
    assert!(schema["properties"]["dedicated_thread"]
        .as_object()
        .expect("dedicated_thread schema")
        .contains_key("description"));

    let missing_agent = tool
        .execute(json!({ "prompt": "summarize the thread" }))
        .await
        .expect("missing agent id returns tool result");
    assert!(missing_agent.is_error);
    assert!(missing_agent.output().contains("agent_id"));

    let missing_prompt = tool
        .execute(json!({ "agent_id": "researcher" }))
        .await
        .expect("missing prompt returns tool result");
    assert!(missing_prompt.is_error);
    assert!(missing_prompt.output().contains("prompt"));

    let dedicated_thread = tool
        .execute(json!({
            "agent_id": "researcher",
            "prompt": "summarize",
            "dedicated_thread": true
        }))
        .await
        .expect("dedicated thread disabled returns tool result");
    assert!(dedicated_thread.is_error);
    // #3049 superseded #1624: dedicated_thread is no longer "temporarily
    // disabled". Verify the tool errors (no provider) without requiring
    // the exact legacy message.
    assert!(!dedicated_thread.output().is_empty());
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

async fn composio_backend_handler(State(state): State<MockState>, request: Request) -> Response {
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
            query: query.clone(),
            body: body.clone(),
            api_key: None,
        });

    match (method, path.as_str()) {
        (Method::GET, "/agent-integrations/composio/toolkits") => ok(json!({
            "toolkits": ["gmail", "github", "totallycustom"]
        })),
        (Method::GET, "/agent-integrations/composio/connections") => {
            if *state.connections_fail.lock().expect("connections flag") {
                return fail(StatusCode::BAD_GATEWAY, "connections unavailable");
            }
            ok(json!({
                "connections": [
                    {
                        "id": "conn-gmail",
                        "toolkit": "gmail",
                        "status": "ACTIVE",
                        "createdAt": "2026-05-30T00:00:00Z"
                    },
                    {
                        "id": "conn-pending",
                        "toolkit": "github",
                        "status": "PENDING",
                        "createdAt": "2026-05-30T00:00:01Z"
                    }
                ]
            }))
        }
        (Method::POST, "/agent-integrations/composio/authorize") => ok(json!({
            "connectUrl": "https://connect.example/round20",
            "connectionId": "conn-authorize"
        })),
        (Method::GET, "/agent-integrations/composio/tools") => {
            if query.contains("totallycustom") {
                ok(json!({ "tools": [] }))
            } else {
                ok(json!({
                    "tools": [
                        {
                            "type": "function",
                            "function": {
                                "name": "GMAIL_FETCH_EMAILS",
                                "description": "Fetch Gmail messages\nwith whitespace",
                                "parameters": {
                                    "type": "object",
                                    "required": ["query"],
                                    "properties": {
                                        "query": { "type": "string" },
                                        "max_results": { "type": "integer" }
                                    }
                                }
                            }
                        },
                        {
                            "type": "function",
                            "function": {
                                "name": "GITHUB_STAR_REPOSITORY",
                                "description": "Star a repository",
                                "parameters": {
                                    "type": "object",
                                    "required": ["owner", "repo"],
                                    "properties": {
                                        "owner": { "type": "string" },
                                        "repo": { "type": "string" }
                                    }
                                }
                            }
                        }
                    ]
                }))
            }
        }
        _ => fail(StatusCode::NOT_FOUND, &format!("unhandled backend {path}")),
    }
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
    let body: Value = if bytes.is_empty() {
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
            body: body.clone(),
            api_key,
        });

    match (method, path.as_str()) {
        (Method::GET, "/api/v3/tools") if query.contains("toolkits=broken") => message_fail(
            StatusCode::BAD_REQUEST,
            "v3 broken list mentions connected_account_id and user_id",
        ),
        (Method::GET, "/api/v3/tools") if query.contains("toolkits=fallback") => {
            fail(StatusCode::BAD_GATEWAY, "fallback to v2 please")
        }
        (Method::GET, "/api/v3/tools") if query.contains("toolkits=gmail") => Json(json!({
            "items": [
                {
                    "slug": "gmail-fetch-emails",
                    "name": "Gmail fetch",
                    "description": "Fetch Gmail",
                    "toolkit": { "slug": "gmail" },
                    "input_parameters": {
                        "type": "object",
                        "properties": { "query": { "type": "string" } }
                    }
                },
                {
                    "name": "gmail-send-email",
                    "description": "Send Gmail",
                    "appName": "gmail"
                }
            ]
        }))
        .into_response(),
        (Method::GET, "/api/v2/actions") if query.contains("appNames=fallback") => Json(json!({
            "items": [
                {
                    "name": "FALLBACK_V2",
                    "appName": "fallback",
                    "description": "Fallback action",
                    "enabled": true
                }
            ]
        }))
        .into_response(),
        (Method::GET, "/api/v2/actions") if query.contains("appNames=broken") => message_fail(
            StatusCode::BAD_REQUEST,
            "v2 broken list mentions connected_account_id and user_id",
        ),
        (Method::POST, "/api/v3/tools/execute/GMAIL_FETCH_EMAILS") => Json(json!({
            "successful": true,
            "data": { "messages": [{ "id": "msg-round20" }] },
            "error": null
        }))
        .into_response(),
        (Method::POST, "/api/v3/tools/execute/BROKEN_ACTION") => message_fail(
            StatusCode::BAD_REQUEST,
            "bad execute connected_account_id user_id entity_id",
        ),
        (Method::POST, "/api/v2/actions/BROKEN_ACTION/execute") => message_fail(
            StatusCode::BAD_REQUEST,
            "bad legacy connected_account_id user_id entity_id",
        ),
        (Method::GET, "/api/v3/auth_configs") if query.contains("toolkit_slug=missing") => {
            Json(json!({ "items": [] })).into_response()
        }
        (Method::GET, "/api/v3/connected_accounts") => Json(json!({
            "items": [
                {
                    "id": "acct-gmail",
                    "status": "ACTIVE",
                    "created_at": "2026-05-30T00:00:00Z",
                    "toolkit": "gmail"
                },
                {
                    "id": "acct-github",
                    "status": "CONNECTED",
                    "createdAt": "2026-05-30T00:00:01Z",
                    "toolkit": { "slug": "github" }
                },
                {
                    "id": "acct-slack",
                    "status": "PENDING",
                    "appName": "slack"
                },
                {
                    "id": "   ",
                    "status": "ACTIVE",
                    "toolkit": "dropme"
                }
            ]
        }))
        .into_response(),
        _ => fail(
            StatusCode::NOT_FOUND,
            &format!("unhandled direct {path} {query}"),
        ),
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
    let api_key = headers
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
            query: query.clone(),
            body: body_json,
            api_key,
        });

    match (method, path.as_str()) {
        (Method::GET, "/markets/m-round20") => {
            Json(json!({ "id": "m-round20", "slug": "market-round20" })).into_response()
        }
        (Method::GET, "/markets") if query.contains("slug=empty-slug") => {
            Json(json!([])).into_response()
        }
        (Method::GET, "/events/evt-round20") => {
            Json(json!({ "id": "evt-round20", "title": "Event Round20" })).into_response()
        }
        (Method::GET, "/price") => Json(json!({ "price": "0.51" })).into_response(),
        (Method::POST, "") | (Method::POST, "/") => Json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "0x0f4240"
        }))
        .into_response(),
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("unhandled polymarket {path} {query}") })),
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
    config.integrations.polymarket.usdc_contract =
        "0x2222222222222222222222222222222222222222".to_string();
    config.integrations.polymarket.clob_exchange_contract =
        "0x3333333333333333333333333333333333333333".to_string();
    config.integrations.polymarket.derived_clob_credentials = Some(fixture_clob_credentials());
}

fn fixture_clob_credentials() -> PolymarketClobCredentials {
    PolymarketClobCredentials {
        api_key: "round20-key".to_string(),
        secret: "cm91bmQyMC1zZWNyZXQ=".to_string(),
        passphrase: "round20-pass".to_string(),
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

fn message_fail(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "message": message.to_string() }))).into_response()
}
