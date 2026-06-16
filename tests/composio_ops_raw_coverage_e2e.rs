//! Focused raw integration coverage for Composio ops.
//!
//! This test binary stays on loopback mocks and temp stores. It drives the
//! public ops layer instead of unit-test-only helpers so coverage lands on the
//! RPC-facing paths used by controllers, tools, and prompt integration fetches.

use std::sync::{Arc, Mutex};

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::Map;
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::all::RegisteredController;
use openhuman_core::openhuman::composio::ops::{
    cached_active_integrations, composio_authorize, composio_create_trigger,
    composio_delete_connection, composio_disable_trigger, composio_enable_trigger,
    composio_execute, composio_get_mode, composio_list_agent_ready_toolkits,
    composio_list_available_triggers, composio_list_capabilities, composio_list_connections,
    composio_list_github_repos, composio_list_toolkits, composio_list_tools,
    composio_list_trigger_history, composio_list_triggers, composio_set_api_key, composio_sync,
    fetch_connected_integrations, fetch_connected_integrations_status,
    invalidate_connected_integrations_cache, FetchConnectedIntegrationsStatus,
};
use openhuman_core::openhuman::composio::{
    all_composio_controller_schemas, all_composio_registered_controllers,
};
use openhuman_core::openhuman::composio::{init_composio_trigger_history, ComposioActionTool};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::tools::{ComposioExecuteTool, Tool};

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    path: String,
    query: String,
    body: Value,
}

#[tokio::test]
async fn composio_ops_use_loopback_backend_for_happy_and_error_paths() {
    let state = MockState::default();
    let app = Router::new()
        .fallback(any(composio_backend_handler))
        .with_state(state.clone());
    let base = start_loopback_backend(app).await;

    let dir = tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().join("workspace"),
        config_path: dir.path().join("config.toml"),
        api_url: Some(base.clone()),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
    config.save().await.expect("save config snapshot");
    store_app_session_token(&config, "round14-session-token");

    invalidate_connected_integrations_cache();

    let toolkits = composio_list_toolkits(&config)
        .await
        .expect("list toolkits")
        .into_cli_compatible_json()
        .expect("toolkits json");
    assert_eq!(
        toolkits.pointer("/result/toolkits/0"),
        Some(&json!("gmail"))
    );

    let capabilities = composio_list_capabilities(&config)
        .await
        .expect("capabilities")
        .into_cli_compatible_json()
        .expect("capabilities json");
    assert!(capabilities
        .pointer("/result/capabilities")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty()));

    let ready = composio_list_agent_ready_toolkits()
        .await
        .expect("agent ready")
        .into_cli_compatible_json()
        .expect("ready json");
    assert!(ready
        .pointer("/result/toolkits")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item == "gmail")));

    let connections = composio_list_connections(&config)
        .await
        .expect("list connections")
        .into_cli_compatible_json()
        .expect("connections json");
    assert_eq!(
        connections.pointer("/result/connections/0/id"),
        Some(&json!("conn-gmail"))
    );

    let integrations = fetch_connected_integrations(&config).await;
    assert!(integrations.iter().any(|item| item.toolkit == "gmail"
        && item.connected
        && item
            .tools
            .iter()
            .any(|tool| tool.name == "GMAIL_FETCH_EMAILS")));
    assert!(integrations.iter().any(|item| item.toolkit == "github"
        && !item.connected
        && item.non_active_status.as_deref() == Some("EXPIRED")));
    assert!(cached_active_integrations(&config).is_some());
    assert!(matches!(
        fetch_connected_integrations_status(&config).await,
        FetchConnectedIntegrationsStatus::Authoritative(_)
    ));

    let authorize = composio_authorize(
        &config,
        " Gmail ",
        Some(json!({ "oauth_scopes": "profile", "custom": "value" })),
    )
    .await
    .expect("authorize")
    .into_cli_compatible_json()
    .expect("authorize json");
    assert_eq!(
        authorize.pointer("/result/connectUrl"),
        Some(&json!("https://connect.example/Gmail"))
    );

    let tools = composio_list_tools(
        &config,
        Some(vec![" github ".into(), "gmail".into()]),
        Some(vec![" repos ".into(), " ".into()]),
    )
    .await
    .expect("list tools")
    .into_cli_compatible_json()
    .expect("tools json");
    let tool_names: Vec<String> = tools
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool.pointer("/function/name").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect();
    assert!(tool_names.contains(&"GMAIL_FETCH_EMAILS".to_string()));
    assert!(tool_names.contains(&"GMAIL_DELETE_EMAIL".to_string()));

    let execute = composio_execute(
        &config,
        "GMAIL_FETCH_EMAILS",
        Some(json!({ "query": "label:INBOX" })),
        None,
    )
    .await
    .expect("execute")
    .into_cli_compatible_json()
    .expect("execute json");
    assert_eq!(
        execute.pointer("/result/data/messages/0/id"),
        Some(&json!("msg-1"))
    );

    let provider_error = composio_execute(
        &config,
        "GMAIL_SEND_EMAIL",
        Some(json!({ "to": "person@example.test" })),
        None,
    )
    .await
    .expect("provider error stays in response")
    .into_cli_compatible_json()
    .expect("provider error json");
    assert_eq!(
        provider_error.pointer("/result/successful"),
        Some(&json!(false))
    );
    assert!(provider_error
        .pointer("/result/error")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .starts_with("[composio:error:validation]"));

    let repos = composio_list_github_repos(&config, Some(" conn-github ".into()))
        .await
        .expect("github repos")
        .into_cli_compatible_json()
        .expect("repos json");
    assert_eq!(
        repos.pointer("/result/repositories/0/fullName"),
        Some(&json!("tinyhumansai/openhuman"))
    );

    let created = composio_create_trigger(
        &config,
        "GITHUB_PULL_REQUEST_EVENT",
        Some("conn-github".into()),
        Some(json!({ "owner": "tinyhumansai", "repo": "openhuman" })),
    )
    .await
    .expect("create trigger")
    .into_cli_compatible_json()
    .expect("create json");
    assert_eq!(
        created.pointer("/result/triggerId"),
        Some(&json!("trigger-created"))
    );

    let available = composio_list_available_triggers(&config, "github", Some("conn-github".into()))
        .await
        .expect("available triggers")
        .into_cli_compatible_json()
        .expect("available json");
    assert_eq!(
        available.pointer("/result/triggers/0/repo/repo"),
        Some(&json!("openhuman"))
    );

    let active = composio_list_triggers(&config, Some(" gmail ".into()))
        .await
        .expect("active triggers")
        .into_cli_compatible_json()
        .expect("active json");
    assert_eq!(
        active.pointer("/result/triggers/0/id"),
        Some(&json!("trigger-active"))
    );

    let enabled = composio_enable_trigger(
        &config,
        " conn-gmail ",
        " GMAIL_NEW_GMAIL_MESSAGE ",
        Some(json!({ "label": "INBOX" })),
    )
    .await
    .expect("enable trigger")
    .into_cli_compatible_json()
    .expect("enable json");
    assert_eq!(
        enabled.pointer("/result/connectionId"),
        Some(&json!("conn-gmail"))
    );

    let disabled = composio_disable_trigger(&config, " trigger-active ")
        .await
        .expect("disable trigger")
        .into_cli_compatible_json()
        .expect("disable json");
    assert_eq!(disabled.pointer("/result/deleted"), Some(&json!(true)));

    let deleted = composio_delete_connection(&config, "conn-slack", false)
        .await
        .expect("delete connection")
        .into_cli_compatible_json()
        .expect("delete json");
    assert_eq!(deleted.pointer("/result/deleted"), Some(&json!(true)));
    assert!(cached_active_integrations(&config).is_some());

    let missing_provider = composio_sync(&config, "conn-slack", Some("manual".into()))
        .await
        .expect_err("slack has no native provider in this test path");
    assert!(missing_provider.contains("no native provider"));
    let bad_reason = composio_sync(&config, "conn-gmail", Some("typo".into()))
        .await
        .expect_err("bad sync reason validates before network");
    assert!(bad_reason.contains("unrecognized sync reason"));

    init_composio_trigger_history(config.workspace_dir.clone())
        .expect("init trigger history store");
    let store = openhuman_core::openhuman::composio::global_composio_trigger_history()
        .expect("global trigger history");
    store
        .record_trigger(
            "gmail",
            "GMAIL_NEW_GMAIL_MESSAGE",
            "metadata-round14",
            "uuid-round14",
            &json!({ "subject": "coverage" }),
        )
        .expect("record trigger history");
    let history = composio_list_trigger_history(&config, Some(5000))
        .await
        .expect("list trigger history")
        .into_cli_compatible_json()
        .expect("history json");
    assert_eq!(
        history.pointer("/result/entries/0/metadata_id"),
        Some(&json!("metadata-round14"))
    );

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().any(|req| {
        req.method == "GET"
            && req.path == "/agent-integrations/composio/tools"
            && req.query.contains("toolkits=github,gmail")
            && req.query.contains("tags=repos")
    }));
    assert!(requests.iter().any(|req| {
        req.method == "POST"
            && req.path == "/agent-integrations/composio/authorize"
            && req.body["oauth_scopes"].as_array().is_some_and(|scopes| {
                scopes
                    .iter()
                    .any(|scope| scope == "https://www.googleapis.com/auth/gmail.readonly")
            })
    }));
}

#[tokio::test]
async fn composio_direct_key_ops_and_agent_tools_take_local_validation_paths() {
    let dir = tempdir().expect("tempdir");
    let mut config = Config {
        workspace_dir: dir.path().join("workspace"),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
    config.save().await.expect("save config");

    let set = composio_set_api_key(&config, "  cmp_round14_key  ", false)
        .await
        .expect("store direct api key")
        .into_cli_compatible_json()
        .expect("set key json");
    assert_eq!(set.pointer("/result/stored"), Some(&json!(true)));
    assert_eq!(set.pointer("/result/mode"), Some(&json!("backend")));

    let mode = composio_get_mode(&config)
        .await
        .expect("get mode")
        .into_cli_compatible_json()
        .expect("mode json");
    assert_eq!(mode.pointer("/result/api_key_set"), Some(&json!(true)));

    config.composio.mode = "direct".to_string();
    config.composio.api_key = Some("cmp_inline_round14".to_string());
    config.save().await.expect("save direct config");

    let direct_toolkits = composio_list_toolkits(&config)
        .await
        .expect("direct list toolkits is local")
        .into_cli_compatible_json()
        .expect("direct toolkits json");
    assert_eq!(
        direct_toolkits.pointer("/result/toolkits"),
        Some(&json!([]))
    );

    let direct_validation = composio_execute(
        &config,
        "GMAIL_SEND_EMAIL",
        Some(json!({ "subject": "missing recipient" })),
        None,
    )
    .await
    .expect_err("direct execution validates before network");
    assert!(direct_validation.starts_with("[composio:error:"));
    assert!(direct_validation.contains("recipient"));

    let arc_config = Arc::new(config.clone());
    let execute_tool = ComposioExecuteTool::new(arc_config.clone());
    let execute_result = execute_tool
        .execute(json!({ "tool": "GMAIL_SEND_EMAIL", "arguments": { "subject": "no to" } }))
        .await
        .expect("agent execute validation result");
    assert!(execute_result.is_error);
    assert!(execute_result.text().contains("recipient"));

    let action_tool = ComposioActionTool::new(
        arc_config,
        "GMAIL_SEND_EMAIL".to_string(),
        "Send mail".to_string(),
        Some(json!({ "type": "object" })),
    );
    let action_result = action_tool
        .execute(json!({ "subject": "no to" }))
        .await
        .expect("per-action validation result");
    assert!(action_result.is_error);
    assert!(action_result.text().contains("recipient"));
}

#[tokio::test]
async fn composio_controller_registry_validates_params_without_backend_network() {
    let schemas = all_composio_controller_schemas();
    let controllers = all_composio_registered_controllers();
    assert_eq!(schemas.len(), controllers.len());
    assert!(schemas.iter().any(|schema| schema.function == "list_tools"));
    assert!(controllers.iter().all(|controller| {
        controller
            .rpc_method_name()
            .starts_with("openhuman.composio_")
    }));

    for (function, input_count) in [
        ("list_toolkits", 0),
        ("list_capabilities", 0),
        ("list_agent_ready_toolkits", 0),
        ("list_connections", 0),
        ("authorize", 2),
        ("delete_connection", 2),
        ("list_tools", 2),
        ("execute", 3),
        ("list_github_repos", 1),
        ("create_trigger", 3),
        ("get_user_profile", 1),
        ("refresh_all_identities", 0),
        ("sync", 2),
        ("list_trigger_history", 1),
        ("get_user_scopes", 1),
        ("set_user_scopes", 4),
        ("list_available_triggers", 2),
        ("list_triggers", 1),
        ("enable_trigger", 3),
        ("disable_trigger", 1),
        ("get_mode", 0),
        ("set_api_key", 2),
        ("clear_api_key", 0),
    ] {
        let schema = openhuman_core::openhuman::composio::schemas::schemas(function);
        assert_eq!(schema.namespace, "composio");
        assert_eq!(schema.function, function);
        assert_eq!(schema.inputs.len(), input_count, "{function}");
        assert!(!schema.description.is_empty(), "{function}");
    }
    let unknown = openhuman_core::openhuman::composio::schemas::schemas("missing");
    assert_eq!(unknown.function, "unknown");

    let authorize_missing = composio_call(controller(&controllers, "authorize"), json!({}))
        .await
        .expect_err("authorize toolkit required");
    assert!(authorize_missing.contains("missing required param 'toolkit'"));

    let delete_blank = composio_call(
        controller(&controllers, "delete_connection"),
        json!({ "connection_id": " " }),
    )
    .await
    .expect_err("delete rejects blank connection id");
    assert!(delete_blank.contains("'connection_id' must not be empty"));

    let list_tools_bad = composio_call(
        controller(&controllers, "list_tools"),
        json!({ "toolkits": "gmail" }),
    )
    .await
    .expect_err("toolkits must be array");
    assert!(list_tools_bad.contains("invalid 'toolkits'"));

    let execute_missing = composio_call(controller(&controllers, "execute"), json!({}))
        .await
        .expect_err("execute tool required");
    assert!(execute_missing.contains("missing required param 'tool'"));

    let create_blank = composio_call(
        controller(&controllers, "create_trigger"),
        json!({ "slug": " " }),
    )
    .await
    .expect_err("create trigger rejects blank slug");
    assert!(create_blank.contains("'slug' must not be empty"));

    let sync_bad_reason = composio_call(
        controller(&controllers, "sync"),
        json!({ "connection_id": "conn-1", "reason": "surprise" }),
    )
    .await
    .expect_err("bad sync reason rejects before backend");
    assert!(sync_bad_reason.contains("unrecognized sync reason"));

    let available_blank = composio_call(
        controller(&controllers, "list_available_triggers"),
        json!({ "toolkit": " " }),
    )
    .await
    .expect_err("available triggers rejects blank toolkit");
    assert!(available_blank.contains("'toolkit' must not be empty"));

    let enable_blank_connection = composio_call(
        controller(&controllers, "enable_trigger"),
        json!({ "connection_id": " ", "slug": "GMAIL_NEW_GMAIL_MESSAGE" }),
    )
    .await
    .expect_err("enable trigger rejects blank connection");
    assert!(enable_blank_connection.contains("'connection_id' must not be empty"));

    let disable_missing = composio_call(controller(&controllers, "disable_trigger"), json!({}))
        .await
        .expect_err("disable trigger id required");
    assert!(disable_missing.contains("missing required param 'trigger_id'"));

    let set_key_blank = composio_call(
        controller(&controllers, "set_api_key"),
        json!({ "api_key": "" }),
    )
    .await
    .expect_err("set api key rejects blank key");
    assert!(set_key_blank.contains("'api_key' must not be empty"));

    let bad_history = composio_call(
        controller(&controllers, "list_trigger_history"),
        json!({ "limit": "many" }),
    )
    .await
    .expect_err("history limit must be numeric");
    assert!(bad_history.contains("invalid params"));
}

async fn composio_backend_handler(State(state): State<MockState>, request: Request) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or_default().to_string();
    let body_bytes = to_bytes(request.into_body(), usize::MAX)
        .await
        .expect("mock request body");
    let body: Value = if body_bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&body_bytes).expect("json body")
    };
    state
        .requests
        .lock()
        .expect("requests")
        .push(RecordedRequest {
            method: method.as_str().to_string(),
            path: path.clone(),
            query: query.clone(),
            body: body.clone(),
        });

    match (method, path.as_str()) {
        (Method::GET, "/agent-integrations/composio/toolkits") => ok(json!({
            "toolkits": ["gmail", "github", "slack"]
        })),
        (Method::GET, "/agent-integrations/composio/connections") => ok(json!({
            "connections": [
                {
                    "id": "conn-gmail",
                    "toolkit": "gmail",
                    "status": "ACTIVE",
                    "createdAt": "2026-05-29T12:00:00Z"
                },
                {
                    "id": "conn-github",
                    "toolkit": "github",
                    "status": "EXPIRED"
                },
                {
                    "id": "conn-slack",
                    "toolkit": "slack",
                    "status": "ACTIVE"
                }
            ]
        })),
        (Method::POST, "/agent-integrations/composio/authorize") => ok(json!({
            "connectUrl": format!(
                "https://connect.example/{}",
                body.get("toolkit").and_then(Value::as_str).unwrap_or("unknown")
            ),
            "connectionId": "conn-authorized"
        })),
        (Method::GET, "/agent-integrations/composio/tools") => ok(json!({
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "GMAIL_FETCH_EMAILS",
                        "description": "Fetch Gmail messages",
                        "parameters": {
                            "type": "object",
                            "required": ["query"],
                            "properties": {
                                "query": { "type": "string" },
                                "max_results": { "type": "number" }
                            }
                        }
                    }
                },
                {
                    "type": "function",
                    "function": {
                        "name": "GMAIL_DELETE_EMAIL",
                        "description": "Delete Gmail messages",
                        "parameters": { "type": "object" }
                    }
                },
                {
                    "type": "function",
                    "function": {
                        "name": "GITHUB_STAR_A_REPOSITORY_FOR_THE_AUTHENTICATED_USER",
                        "description": "Star repository",
                        "parameters": { "type": "object" }
                    }
                }
            ]
        })),
        (Method::POST, "/agent-integrations/composio/execute") => {
            match body.get("tool").and_then(Value::as_str) {
                Some("GMAIL_SEND_EMAIL") => ok(json!({
                    "data": null,
                    "successful": false,
                    "error": "missing required field to",
                    "costUsd": 0.0
                })),
                Some("GMAIL_FETCH_EMAILS") => ok(json!({
                    "data": {
                        "messages": [{ "id": "msg-1", "subject": "hello" }]
                    },
                    "successful": true,
                    "error": null,
                    "costUsd": 0.02,
                    "markdownFormatted": "Fetched 1 message"
                })),
                other => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "success": false,
                        "error": format!("unexpected execute tool: {other:?}")
                    })),
                )
                    .into_response(),
            }
        }
        (Method::GET, "/agent-integrations/composio/github/repos") => ok(json!({
            "connectionId": "conn-github",
            "repositories": [{
                "owner": "tinyhumansai",
                "repo": "openhuman",
                "fullName": "tinyhumansai/openhuman",
                "private": false,
                "defaultBranch": "main",
                "htmlUrl": "https://github.com/tinyhumansai/openhuman"
            }]
        })),
        (Method::POST, "/agent-integrations/composio/triggers") => {
            if body.get("slug").and_then(Value::as_str) == Some("GITHUB_PULL_REQUEST_EVENT") {
                ok(json!({
                    "triggerId": "trigger-created",
                    "status": "enabled"
                }))
            } else if body.get("connectionId").is_some() {
                ok(json!({
                    "triggerId": "trigger-enabled",
                    "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                    "connectionId": "conn-gmail"
                }))
            } else {
                ok(json!({
                    "triggerId": "trigger-created-generic",
                    "status": "enabled"
                }))
            }
        }
        (Method::GET, "/agent-integrations/composio/triggers/available") => ok(json!({
            "triggers": [{
                "slug": "GITHUB_PULL_REQUEST_EVENT",
                "scope": "github_repo",
                "defaultConfig": { "event": "pull_request" },
                "requiredConfigKeys": ["owner", "repo"],
                "repo": { "owner": "tinyhumansai", "repo": "openhuman" }
            }]
        })),
        (Method::GET, "/agent-integrations/composio/triggers") => ok(json!({
            "triggers": [{
                "id": "trigger-active",
                "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                "toolkit": "gmail",
                "connectionId": "conn-gmail",
                "triggerConfig": { "label": "INBOX" },
                "state": "enabled"
            }]
        })),
        (Method::DELETE, path) if path.starts_with("/agent-integrations/composio/triggers/") => {
            ok(json!({ "deleted": true }))
        }
        (Method::DELETE, path) if path.starts_with("/agent-integrations/composio/connections/") => {
            ok(json!({ "deleted": true, "memory_chunks_deleted": 0 }))
        }
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": format!("unhandled {path}") })),
        )
            .into_response(),
    }
}

fn controller<'a>(
    controllers: &'a [RegisteredController],
    function: &str,
) -> &'a RegisteredController {
    controllers
        .iter()
        .find(|controller| controller.schema.function == function)
        .unwrap_or_else(|| panic!("controller {function} registered"))
}

async fn composio_call(controller: &RegisteredController, params: Value) -> Result<Value, String> {
    let params: Map<String, Value> = params.as_object().cloned().unwrap_or_default();
    (controller.handler)(params).await
}

async fn start_loopback_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock composio backend");
    let addr = listener.local_addr().expect("mock backend addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn store_app_session_token(config: &Config, token: &str) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            token,
            std::collections::HashMap::new(),
            true,
        )
        .expect("store app session token");
}

fn ok(data: Value) -> Response {
    Json(json!({ "success": true, "data": data })).into_response()
}
