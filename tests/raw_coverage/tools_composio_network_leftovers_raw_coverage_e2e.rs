//! Round20 raw/E2E coverage for Composio tool leftovers and adjacent
//! network-tool branches. All HTTP traffic stays on loopback mocks.

use crate::env_guard::EnvVarGuard;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};

use openhuman_core::config::Config;
use openhuman_core::integrations::composio::ops::{composio_authorize, composio_list_tools};
use openhuman_core::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::tools::{
    ComposioListConnectionsTool, ComposioListToolkitsTool, ComposioListToolsTool,
};
use tinytools::{Tool, ToolCallOptions};

static ENV_LOCK: &OnceLock<tokio::sync::Mutex<()>> = &crate::SHARED_ENV_LOCK;

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

struct Harness {
    _tmp: TempDir,
    config: Config,
    _guards: Vec<EnvVarGuard>,
}

fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .blocking_lock()
}

async fn env_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock().await
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("target dir");
    Builder::new()
        .prefix("tools-composio-network-leftovers-round20-")
        .tempdir_in("target")
        .expect("round20 tempdir")
}

async fn setup_config() -> Harness {
    crate::tinyhumans_boot::boot();
    let tmp = tempdir();
    let root = tmp.path().join("openhuman");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let guards = vec![
        EnvVarGuard::set_path("OPENHUMAN_WORKSPACE", &root),
        EnvVarGuard::set_path("HOME", tmp.path()),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::unset("OPENHUMAN_CORE_RPC_URL"),
        EnvVarGuard::unset("OPENHUMAN_CORE_PORT"),
        EnvVarGuard::unset("OPENHUMAN_LSP_ENABLED"),
    ];

    let mut config = Config {
        workspace_dir: workspace,
        config_path: root.join("config.toml"),
        ..Config::default()
    };
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
    let _lock = env_lock_async().await;
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
async fn round20_composio_ops_cover_authorize_scopes() {
    let _lock = env_lock_async().await;
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

    let legacy_extra = composio_authorize(
        &harness.config,
        "gmail",
        Some(json!({ "oauth_scopes": [123] })),
    )
    .await
    .expect("module ignores non-string legacy OAuth scope entries")
    .value;
    assert_eq!(legacy_extra.connection_id, "conn-authorize");

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

    let requests = state.requests.lock().expect("requests").clone();
    let authorize_body = requests
        .iter()
        .rfind(|request| request.path == "/agent-integrations/composio/authorize")
        .expect("authorize request with the requested WABA id")
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
