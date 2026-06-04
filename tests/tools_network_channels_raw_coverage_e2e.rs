//! Round 15 raw integration coverage for network tools plus web-channel paths.
//!
//! Everything here stays local-only: loopback HTTP mocks, temp git/cron
//! workspaces, and validation/error branches that do not touch the desktop.

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use serde_json::json;
use tempfile::{tempdir, TempDir};
use tokio::time::timeout;

use openhuman_core::core::socketio::WebChannelEvent;
use openhuman_core::openhuman::channels::providers::web::{
    all_web_channel_controller_schemas, all_web_channel_registered_controllers, cancel_chat,
    channel_web_cancel, publish_web_channel_event, schemas as web_channel_schema, start_chat,
    subscribe_web_channel_events,
};
use openhuman_core::openhuman::config::{
    AutonomyConfig, Config, PolymarketClobCredentials, PolymarketConfig,
};
use openhuman_core::openhuman::security::{AutonomyLevel, SecurityPolicy};
use openhuman_core::openhuman::tools::{
    ComposioTool, GitOperationsTool, MouseTool, PolymarketTool, ScheduleTool, Tool, ToolCallOptions,
};

#[derive(Clone, Debug)]
struct MockRequest {
    method: Method,
    path: String,
    query: Option<String>,
    body: String,
    poly_api_key: Option<String>,
}

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<MockRequest>>>,
}

async fn start_polymarket_mock() -> (String, MockState) {
    let state = MockState::default();
    let app = Router::new()
        .route("/", any(polymarket_handler))
        .fallback(any(polymarket_handler))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock serve");
    });
    (format!("http://127.0.0.1:{}", addr.port()), state)
}

async fn polymarket_handler(
    State(state): State<MockState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().to_string();
    let query = uri.query().map(str::to_string);
    let body = String::from_utf8_lossy(&body).to_string();
    let poly_api_key = headers
        .get("poly_api_key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    state
        .requests
        .lock()
        .expect("requests lock")
        .push(MockRequest {
            method: method.clone(),
            path: path.clone(),
            query,
            body,
            poly_api_key,
        });

    let payload = match (method, path.as_str()) {
        (Method::GET, "/markets") => json!([
            {
                "id": "m-1",
                "slug": "will-it-rain",
                "question": "Will it rain tomorrow?"
            }
        ]),
        (Method::GET, "/markets/m-1") => json!({
            "id": "m-1",
            "slug": "will-it-rain",
            "outcomes": ["Yes", "No"]
        }),
        (Method::GET, "/events") => json!({
            "data": [
                { "id": "e-1", "slug": "weather" }
            ],
            "next_cursor": "cursor-2"
        }),
        (Method::GET, "/events/e-1") => json!({
            "id": "e-1",
            "title": "Weather"
        }),
        (Method::GET, "/book") => json!({
            "bids": [["0.42", "10"]],
            "asks": [["0.43", "12"]]
        }),
        (Method::GET, "/price") => json!({ "price": "0.42" }),
        (Method::GET, "/data/positions") => json!({
            "positions": [
                { "asset": "token-yes", "size": "3.5" }
            ]
        }),
        (Method::GET, "/data/balance") => json!({ "balance": "125.50" }),
        (Method::GET, "/orders") => json!({ "orders": [] }),
        (Method::POST, "/") => json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "0x00000000000000000000000000000000000000000000000000000000000f4240"
        }),
        _ => {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(json!({ "error": format!("unhandled {path}") })),
            )
                .into_response();
        }
    };

    axum::Json(payload).into_response()
}

fn full_security(workspace: &std::path::Path) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::from_config(
        &AutonomyConfig {
            level: AutonomyLevel::Full,
            max_actions_per_hour: 10_000,
            ..Default::default()
        },
        workspace,
        workspace,
    ))
}

fn readonly_security(workspace: &std::path::Path) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::from_config(
        &AutonomyConfig {
            level: AutonomyLevel::ReadOnly,
            max_actions_per_hour: 10_000,
            ..Default::default()
        },
        workspace,
        workspace,
    ))
}

fn temp_config() -> (TempDir, Config) {
    let tmp = tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace");
    (tmp, config)
}

fn text(result: &openhuman_core::openhuman::tools::ToolResult) -> String {
    result.output()
}

fn assert_contains(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "expected {haystack:?} to contain {needle:?}"
    );
}

#[tokio::test]
async fn polymarket_loopback_exercises_gamma_clob_and_polygon_read_paths() {
    let (_tmp, config) = temp_config();
    let (base, state) = start_polymarket_mock().await;
    let user = "0x1111111111111111111111111111111111111111";
    let tool = PolymarketTool::new(
        &PolymarketConfig {
            enabled: true,
            gamma_base_url: base.clone(),
            clob_base_url: base.clone(),
            polygon_rpc_url: base,
            timeout_secs: 2,
            eoa_address: Some(user.to_string()),
            usdc_contract: "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174".to_string(),
            clob_exchange_contract: "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E".to_string(),
            derived_clob_credentials: Some(PolymarketClobCredentials {
                api_key: "local-key".to_string(),
                secret: "bG9jYWwtc2VjcmV0".to_string(),
                passphrase: "local-pass".to_string(),
            }),
        },
        full_security(&config.workspace_dir),
    );

    let cases = [
        json!({"action": "list_markets", "slug": "will-it-rain", "limit": 5, "active": true}),
        json!({"action": "get_market", "market_id": "m-1"}),
        json!({"action": "get_market", "slug": "will-it-rain"}),
        json!({"action": "list_events", "limit": 2, "closed": false, "tag": "weather"}),
        json!({"action": "list_events", "event_id": "e-1"}),
        json!({"action": "get_orderbook", "token_id": "token-yes"}),
        json!({"action": "get_price", "token_id": "token-yes", "side": "BUY"}),
        json!({"action": "get_positions", "user": user}),
        json!({"action": "get_balance", "user": user, "token": "usdc"}),
        json!({"action": "get_open_orders", "user": user}),
        json!({"action": "get_usdc_allowance", "user": user}),
    ];

    for args in cases {
        let result = tool.execute(args).await.expect("execute");
        assert!(
            !result.is_error,
            "unexpected polymarket error: {}",
            text(&result)
        );
    }

    let missing = tool
        .execute(json!({"action": "get_market"}))
        .await
        .expect("missing lookup");
    assert!(missing.is_error);
    assert_contains(&text(&missing), "get_market requires");

    let invalid_side = tool
        .execute(json!({"action": "get_price", "token_id": "token-yes", "side": "maybe"}))
        .await
        .expect("invalid side");
    assert!(invalid_side.is_error);
    assert_contains(&text(&invalid_side), "Invalid 'side'");

    let requests = state.requests.lock().expect("requests").clone();
    assert!(
        requests.iter().any(|request| request.path == "/markets"
            && request
                .query
                .as_deref()
                .unwrap_or("")
                .contains("slug=will-it-rain")),
        "list_markets query was not captured: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request.path == "/data/positions"
                && request.poly_api_key.as_deref() == Some("local-key")),
        "signed CLOB read did not include credential headers: {requests:?}"
    );
    assert!(
        requests.iter().any(|request| request.method == Method::POST
            && request.path == "/"
            && request.body.contains("eth_call")),
        "Polygon allowance RPC was not captured: {requests:?}"
    );
}

#[tokio::test]
async fn git_operations_cover_read_write_markdown_and_safety_rejections() {
    let tmp = tempdir().expect("repo tempdir");
    let repo = tmp.path();
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "round15@example.test"]);
    run_git(repo, &["config", "user.name", "Round Fifteen"]);
    std::fs::write(repo.join("tracked.txt"), "first\n").expect("write tracked");
    run_git(repo, &["add", "tracked.txt"]);
    run_git(repo, &["commit", "-m", "initial"]);
    std::fs::write(repo.join("tracked.txt"), "first\nsecond\n").expect("modify tracked");
    std::fs::write(repo.join("untracked.txt"), "new\n").expect("write untracked");

    let tool = GitOperationsTool::new(full_security(repo), repo.to_path_buf());
    let status = tool
        .execute(json!({"operation": "status"}))
        .await
        .expect("status");
    assert!(!status.is_error);
    assert_contains(&text(&status), "untracked.txt");
    assert_contains(
        status.markdown_formatted.as_deref().unwrap_or(""),
        "untracked",
    );

    let diff = tool
        .execute(json!({"operation": "diff", "files": "tracked.txt"}))
        .await
        .expect("diff");
    assert!(!diff.is_error);
    assert_contains(&text(&diff), "second");

    let blocked_diff = tool
        .execute(json!({"operation": "diff", "files": "tracked.txt;rm"}))
        .await
        .expect_err("blocked diff should hard fail in sanitizer");
    assert_contains(&blocked_diff.to_string(), "Blocked potentially");

    let add = tool
        .execute(json!({"operation": "add", "paths": "tracked.txt"}))
        .await
        .expect("add");
    assert!(!add.is_error, "add failed: {}", text(&add));

    let commit = tool
        .execute(json!({"operation": "commit", "message": "\n round15 commit \n"}))
        .await
        .expect("commit");
    assert!(!commit.is_error, "commit failed: {}", text(&commit));

    let log = tool
        .execute(json!({"operation": "log", "limit": 2}))
        .await
        .expect("log");
    assert!(!log.is_error);
    assert_contains(&text(&log), "round15 commit");

    let branch = tool
        .execute_with_options(
            json!({"operation": "branch"}),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("branch");
    assert!(!branch.is_error);
    assert_contains(
        branch.markdown_formatted.as_deref().unwrap_or(""),
        "current",
    );

    let bad_checkout = tool
        .execute(json!({"operation": "checkout", "branch": "main~1"}))
        .await
        .expect_err("invalid branch should be a hard validation error");
    assert_contains(&bad_checkout.to_string(), "invalid characters");

    let readonly = GitOperationsTool::new(readonly_security(repo), repo.to_path_buf());
    let blocked = readonly
        .execute(json!({"operation": "add", "paths": "untracked.txt"}))
        .await
        .expect("readonly add");
    assert!(blocked.is_error);
    assert_contains(&text(&blocked), "[policy-blocked]");
}

#[tokio::test]
async fn schedule_tool_covers_cron_once_agent_prompt_and_policy_edges() {
    let (_tmp, config) = temp_config();
    let tool = ScheduleTool::new(full_security(&config.workspace_dir), config.clone());

    let empty = tool.execute(json!({"action": "list"})).await.expect("list");
    assert!(!empty.is_error);
    assert_contains(&text(&empty), "No scheduled jobs");

    let natural_language = tool
        .execute(json!({
            "action": "create",
            "delay": "30m",
            "command": "remind me to stretch",
            "name": "stretch"
        }))
        .await
        .expect("agent prompt");
    assert!(!natural_language.is_error, "{}", text(&natural_language));
    assert_contains(&text(&natural_language), "Created agent job");

    let recurring = tool
        .execute(json!({
            "action": "add",
            "expression": "*/15 * * * *",
            "command": "echo round15"
        }))
        .await
        .expect("recurring");
    assert!(!recurring.is_error, "{}", text(&recurring));
    let recurring_id = text(&recurring)
        .split_whitespace()
        .nth(3)
        .expect("job id")
        .to_string();

    let once = tool
        .execute(json!({
            "action": "once",
            "run_at": "2035-01-01T00:00:00Z",
            "command": "echo future"
        }))
        .await
        .expect("once");
    assert!(!once.is_error, "{}", text(&once));

    let list = tool.execute(json!({"action": "list"})).await.expect("list");
    assert!(!list.is_error);
    assert_contains(&text(&list), "echo round15");
    assert_contains(&text(&list), "[one-shot]");

    let get = tool
        .execute(json!({"action": "get", "id": recurring_id}))
        .await
        .expect("get");
    assert!(!get.is_error);
    assert_contains(&text(&get), "echo round15");

    let id = text(&get)
        .lines()
        .find(|line| line.contains("\"id\""))
        .and_then(|line| line.split('"').nth(3))
        .expect("json id")
        .to_string();

    for action in ["pause", "resume", "cancel"] {
        let result = tool
            .execute(json!({"action": action, "id": id}))
            .await
            .unwrap_or_else(|err| panic!("{action}: {err}"));
        assert!(!result.is_error, "{action} failed: {}", text(&result));
    }

    let missing_command = tool
        .execute(json!({"action": "create", "expression": "* * * * *"}))
        .await
        .expect("missing command");
    assert!(missing_command.is_error);
    assert_contains(&text(&missing_command), "Provide 'command'");

    let invalid_once = tool
        .execute(json!({
            "action": "once",
            "delay": "5m",
            "run_at": "2035-01-01T00:00:00Z",
            "command": "echo invalid"
        }))
        .await
        .expect("invalid once");
    assert!(invalid_once.is_error);
    assert_contains(&text(&invalid_once), "not both");

    let readonly = ScheduleTool::new(readonly_security(&config.workspace_dir), config);
    let blocked = readonly
        .execute(json!({
            "action": "create",
            "expression": "* * * * *",
            "command": "echo blocked"
        }))
        .await
        .expect("readonly create");
    assert!(blocked.is_error);
    assert_contains(&text(&blocked), "read-only");
}

#[tokio::test]
async fn composio_direct_and_mouse_tools_cover_validation_policy_and_schema_paths() {
    let (_tmp, config) = temp_config();
    let full = full_security(&config.workspace_dir);
    let readonly = readonly_security(&config.workspace_dir);

    let composio = ComposioTool::new("  local-test-key  ", Some(" entity-1 "), full.clone());
    assert_eq!(composio.name(), "composio");
    assert!(composio.external_effect());
    assert!(!composio.external_effect_with_args(&json!({"action": "list"})));
    assert!(!composio.external_effect_with_args(&json!({"action": "connect"})));
    assert!(composio.external_effect_with_args(&json!({"action": "execute"})));
    assert_contains(
        &composio.parameters_schema().to_string(),
        "connected_account_id",
    );

    let unknown = composio
        .execute(json!({"action": "wat"}))
        .await
        .expect("unknown composio");
    assert!(unknown.is_error);
    assert_contains(&text(&unknown), "Unknown action");

    let missing_connect = composio
        .execute(json!({"action": "connect"}))
        .await
        .expect_err("connect without app/auth_config_id should hard fail before network");
    assert_contains(&missing_connect.to_string(), "Missing 'app'");

    let readonly_composio = ComposioTool::new("local-test-key", None, readonly.clone());
    let blocked_execute = readonly_composio
        .execute(json!({
            "action": "execute",
            "tool_slug": "GMAIL_SEND_EMAIL",
            "params": { "to": "nobody@example.test" }
        }))
        .await
        .expect("readonly execute");
    assert!(blocked_execute.is_error);
    assert_contains(&text(&blocked_execute), "policy");

    let mouse = MouseTool::new(readonly);
    assert_eq!(mouse.name(), "mouse");
    assert_contains(&mouse.parameters_schema().to_string(), "double_click");
    let blocked_mouse = mouse
        .execute(json!({"action": "move", "x": 1, "y": 1, "human_like": false}))
        .await
        .expect("readonly mouse");
    assert!(blocked_mouse.is_error);
    assert_contains(&text(&blocked_mouse), "read-only");

    let mouse = MouseTool::new(full);
    let missing_xy = mouse
        .execute(json!({"action": "click", "button": "left"}))
        .await
        .expect_err("missing xy should hard fail before enigo");
    assert_contains(&missing_xy.to_string(), "Missing required 'x'");

    let bad_coord = mouse
        .execute(json!({"action": "move", "x": -1, "y": 0, "human_like": false}))
        .await
        .expect_err("bad coord should hard fail before enigo");
    assert_contains(&bad_coord.to_string(), "out of range");

    let bad_button = mouse
        .execute(json!({"action": "click", "x": 1, "y": 1, "button": "side"}))
        .await
        .expect_err("bad button should hard fail before enigo");
    assert_contains(&bad_button.to_string(), "Invalid mouse button");

    let zero_scroll = mouse
        .execute(json!({"action": "scroll", "scroll_x": 0, "scroll_y": 0}))
        .await
        .expect("zero scroll");
    assert!(zero_scroll.is_error);
    assert_contains(&text(&zero_scroll), "non-zero");

    let unknown_mouse = mouse
        .execute(json!({"action": "teleport"}))
        .await
        .expect("unknown mouse");
    assert!(unknown_mouse.is_error);
    assert_contains(&text(&unknown_mouse), "Unknown mouse action");
}

#[tokio::test]
async fn web_channel_public_paths_cover_validation_cancel_schema_and_event_bus() {
    assert_eq!(all_web_channel_controller_schemas().len(), 4);
    assert_eq!(all_web_channel_registered_controllers().len(), 4);
    assert_eq!(web_channel_schema("chat").function, "web_chat");
    assert_eq!(web_channel_schema("cancel").function, "web_cancel");
    assert_eq!(web_channel_schema("missing").function, "unknown");

    let missing_client = start_chat(" ", "thread", "hello", None, None, None, None, None)
        .await
        .expect_err("blank client");
    assert_contains(&missing_client, "client_id is required");
    let missing_thread = cancel_chat("client", " ").await.expect_err("blank thread");
    assert_contains(&missing_thread, "thread_id is required");

    let none = cancel_chat("client", "round15-thread")
        .await
        .expect("no in-flight cancel");
    assert_eq!(none, None);

    let outcome = channel_web_cancel(" client ", " round15-thread ")
        .await
        .expect("cancel rpc outcome");
    assert_eq!(outcome.value["cancelled"], false);
    assert_eq!(outcome.value["client_id"], "client");
    assert_eq!(outcome.value["thread_id"], "round15-thread");

    let mut rx = subscribe_web_channel_events();
    publish_web_channel_event(WebChannelEvent {
        event: "round15_probe".to_string(),
        client_id: "client".to_string(),
        thread_id: "thread".to_string(),
        request_id: "request".to_string(),
        message: Some("payload".to_string()),
        ..Default::default()
    });
    let event = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("event timeout")
        .expect("event");
    assert_eq!(event.event, "round15_probe");
    assert_eq!(event.message.as_deref(), Some("payload"));
}

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
