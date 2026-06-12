//! Round25 focused raw coverage for large Composio direct-mode misses.
//!
//! The direct-mode factory normally pins Composio's production HTTPS API.
//! This test uses debug-only loopback base overrides and temp config stores so
//! no real network, keychain, or backend session is required.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};

use openhuman_core::openhuman::composio::ops::{
    cached_active_integrations, composio_authorize, composio_execute, composio_list_connections,
    composio_list_toolkits, composio_list_tools, fetch_connected_integrations_status,
};
use openhuman_core::openhuman::composio::{
    invalidate_connected_integrations_cache, FetchConnectedIntegrationsStatus,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::tools::{ComposioListToolsTool, Tool, ToolCallOptions};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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
}

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl AsRef<str>) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value.as_ref());
        Self { key, old }
    }

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

async fn setup_direct_config(base: &str) -> Harness {
    std::fs::create_dir_all("target").expect("target dir");
    let tmp = Builder::new()
        .prefix("tools-composio-large-round25-")
        .tempdir_in("target")
        .expect("round25 tempdir");
    let root = tmp.path().join("openhuman");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let guards = vec![
        EnvGuard::set_path("OPENHUMAN_WORKSPACE", &root),
        EnvGuard::set_path("HOME", tmp.path()),
        EnvGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvGuard::set(
            "OPENHUMAN_COMPOSIO_DIRECT_BASE_V2",
            format!("{base}/api/v2"),
        ),
        EnvGuard::set(
            "OPENHUMAN_COMPOSIO_DIRECT_BASE_V3",
            format!("{base}/api/v3"),
        ),
        EnvGuard::unset("BACKEND_URL"),
        EnvGuard::unset("VITE_BACKEND_URL"),
        EnvGuard::unset("OPENHUMAN_API_URL"),
        EnvGuard::unset("OPENHUMAN_CORE_RPC_URL"),
        EnvGuard::unset("OPENHUMAN_CORE_PORT"),
    ];

    let mut config = Config {
        workspace_dir: workspace,
        config_path: root.join("config.toml"),
        ..Config::default()
    };
    config.composio.mode = "direct".to_string();
    config.composio.api_key = Some(" ck_round25_direct ".to_string());
    config.composio.entity_id = " entity-round25 ".to_string();
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

#[tokio::test]
async fn round25_direct_mode_ops_use_loopback_factory_for_tools_connections_and_execute() {
    let _lock = env_lock();
    invalidate_connected_integrations_cache();

    let state = MockState::default();
    let base = start_loopback(
        Router::new()
            .fallback(any(composio_direct_handler))
            .with_state(state.clone()),
    )
    .await;
    let harness = setup_direct_config(&base).await;

    let toolkits = composio_list_toolkits(&harness.config)
        .await
        .expect("direct list toolkits")
        .value;
    assert!(toolkits.toolkits.is_empty());

    let connections = composio_list_connections(&harness.config)
        .await
        .expect("direct list connections")
        .value;
    assert_eq!(connections.connections.len(), 3);
    assert!(connections
        .connections
        .iter()
        .any(|conn| conn.id == "acct-gmail" && conn.normalized_toolkit() == "gmail"));

    let listed = composio_list_tools(
        &harness.config,
        None,
        Some(vec![" readOnlyHint ".into(), " ".into()]),
    )
    .await
    .expect("direct list tools")
    .value;
    assert!(listed
        .tools
        .iter()
        .any(|tool| tool.function.name == "GMAIL_FETCH_EMAILS"));
    assert!(!listed
        .tools
        .iter()
        .any(|tool| tool.function.name.is_empty()));

    let markdown = ComposioListToolsTool::new(Arc::new(harness.config.clone()))
        .execute_with_options(
            json!({ "toolkits": ["gmail"], "tags": ["readOnlyHint"], "include_unconnected": true }),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("direct list tools tool markdown");
    assert!(!markdown.is_error, "{}", markdown.output());
    assert_eq!(markdown.output(), "{\"tools\":[]}");

    let authorize = composio_authorize(
        &harness.config,
        " gmail ",
        Some(json!({ "ignored_in_direct": true })),
    )
    .await
    .expect("direct authorize")
    .value;
    assert_eq!(authorize.connection_id, "");
    assert_eq!(
        authorize.connect_url,
        "https://connect.example.test/round25"
    );

    let executed = composio_execute(
        &harness.config,
        "GMAIL_FETCH_EMAILS",
        Some(json!({ "query": "label:INBOX" })),
        None,
    )
    .await
    .expect("direct execute")
    .value;
    assert!(executed.successful, "{executed:?}");
    assert_eq!(
        executed.data.pointer("/messages/0/id"),
        Some(&json!("msg-round25"))
    );
    assert_eq!(executed.cost_usd, 0.0);

    let failed_execute = composio_execute(
        &harness.config,
        "GMAIL_SEND_EMAIL",
        Some(json!({ "to": "person@example.test" })),
        None,
    )
    .await
    .expect("direct execute provider failure")
    .value;
    assert!(!failed_execute.successful);
    assert!(failed_execute
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("provider rejected send"));

    match fetch_connected_integrations_status(&harness.config).await {
        FetchConnectedIntegrationsStatus::Authoritative(integrations) => {
            assert!(integrations
                .iter()
                .any(|item| item.toolkit == "gmail" && item.connected));
            assert!(integrations
                .iter()
                .any(|item| item.toolkit == "slack" && item.connected));
            assert!(!integrations
                .iter()
                .any(|item| item.toolkit == "github" && item.connected));
        }
        FetchConnectedIntegrationsStatus::Unavailable => {
            panic!("direct loopback integrations should be authoritative")
        }
    }
    assert!(cached_active_integrations(&harness.config).is_some());

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().all(|request| {
        request.api_key.as_deref() == Some("ck_round25_direct") || request.path == "/health"
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::GET
            && request.path == "/api/v3/connected_accounts"
            && request.query.contains("limit=200")
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::GET
            && request.path == "/api/v3/tools"
            && request.query.contains("toolkits=gmail%2Cslack")
            && request.query.contains("tags=readOnlyHint")
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::POST
            && request.path == "/api/v3/connected_accounts/link"
            && request.body["auth_config_id"] == "auth-round25"
            && request.body["user_id"] == "entity-round25"
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::POST
            && request.path == "/api/v3/tools/execute/GMAIL_FETCH_EMAILS"
            && request.body["arguments"]["query"] == "label:INBOX"
            && request.body["user_id"] == "entity-round25"
    }));
}

async fn start_loopback(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("loopback server");
    });
    format!("http://127.0.0.1:{}", addr.port())
}

async fn composio_direct_handler(State(state): State<MockState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let method = parts.method;
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().unwrap_or_default().to_string();
    let api_key = parts
        .headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let body_bytes = to_bytes(body, 1024 * 1024).await.expect("body bytes");
    let body_json = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or_else(|_| Value::Null)
    };

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
        (Method::GET, "/api/v3/connected_accounts") => Json(json!({
            "items": [
                {
                    "id": "acct-gmail",
                    "status": "ACTIVE",
                    "toolkit": { "slug": "gmail" },
                    "created_at": "2026-05-30T00:00:00Z"
                },
                {
                    "id": "acct-github",
                    "status": "INITIATED",
                    "toolkit": "github",
                    "createdAt": "2026-05-30T00:00:01Z"
                },
                {
                    "id": "acct-slack",
                    "status": "CONNECTED",
                    "appName": "slack",
                    "created_at": "2026-05-30T00:00:02Z"
                },
                {
                    "id": "   ",
                    "status": "ACTIVE",
                    "toolkit": "dropme"
                }
            ]
        }))
        .into_response(),
        (Method::GET, "/api/v3/tools") => Json(json!({
            "items": [
                {
                    "slug": "GMAIL_FETCH_EMAILS",
                    "description": "Fetch Gmail messages",
                    "toolkit": { "slug": "gmail" },
                    "input_parameters": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" }
                        }
                    }
                },
                {
                    "slug": "",
                    "description": "Malformed row should be dropped",
                    "toolkit": { "slug": "gmail" }
                },
                {
                    "name": "SLACK_FETCH_MESSAGES",
                    "description": "Fetch Slack messages",
                    "appName": "slack",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "channel": { "type": "string" }
                        }
                    }
                }
            ]
        }))
        .into_response(),
        (Method::GET, "/api/v3/auth_configs") => Json(json!({
            "items": [
                { "id": "auth-disabled", "status": "disabled", "enabled": false },
                { "id": "auth-round25", "status": "enabled", "enabled": true }
            ]
        }))
        .into_response(),
        (Method::POST, "/api/v3/connected_accounts/link") => Json(json!({
            "data": {
                "redirect_url": "https://connect.example.test/round25"
            }
        }))
        .into_response(),
        (Method::POST, "/api/v3/tools/execute/GMAIL_FETCH_EMAILS") => Json(json!({
            "successful": true,
            "data": {
                "messages": [
                    { "id": "msg-round25", "subject": "Coverage" }
                ]
            }
        }))
        .into_response(),
        (Method::POST, "/api/v3/tools/execute/GMAIL_SEND_EMAIL") => Json(json!({
            "successful": false,
            "error": "provider rejected send",
            "data": {
                "status": "blocked"
            }
        }))
        .into_response(),
        _ => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response(),
    }
}
