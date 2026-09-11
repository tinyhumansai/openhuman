//! Round24 raw coverage for broad tools/composio cold branches.
//!
//! All HTTP traffic stays on loopback mocks. These tests drive public tool
//! APIs so coverage lands on the same paths an agent/tool call uses.

use std::sync::{Arc, Mutex};

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};

use openhuman_core::openhuman::security::{AutonomyLevel, SecurityPolicy};
use openhuman_core::openhuman::tools::{ComposioTool, Tool};

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

#[tokio::test]
async fn round24_composio_direct_covers_v3_v2_fallbacks_and_account_shapes() {
    let state = MockState::default();
    let base = start_loopback(
        Router::new()
            .fallback(any(composio_handler))
            .with_state(state.clone()),
    )
    .await;
    let tool = ComposioTool::new_with_base_urls_for_loopback(
        "  ck_round24  ",
        Some(" entity-round24 "),
        Arc::new(SecurityPolicy::default()),
        format!("{base}/api/v2"),
        format!("{base}/api/v3"),
    )
    .expect("loopback composio tool");

    let connected = tool
        .list_connected_accounts()
        .await
        .expect("connected accounts");
    assert_eq!(connected.len(), 3, "blank id row should be dropped");
    assert_eq!(
        connected
            .iter()
            .map(|account| account.toolkit_slug().unwrap())
            .collect::<Vec<_>>(),
        vec!["gmail", "github", "slack"]
    );

    let execute = tool
        .execute(json!({
            "action": "execute",
            "tool_slug": "GMAIL_SEND_EMAIL",
            "params": { "to": "a@example.test" },
            "connected_account_id": "conn-secret"
        }))
        .await
        .expect("execute falls back to v2");
    assert!(!execute.is_error, "{}", execute.output());
    assert!(execute.output().contains("v2-fallback-ok"));

    let connect = tool
        .execute(json!({
            "action": "connect",
            "app": "gmail"
        }))
        .await
        .expect("connect resolves auth config");
    assert!(!connect.is_error, "{}", connect.output());
    assert!(connect
        .output()
        .contains("https://connect.example.test/round24"));

    let requests = state.requests.lock().expect("requests").clone();
    let v3_execute = requests
        .iter()
        .find(|request| {
            request.method == Method::POST
                && request.path == "/api/v3/tools/execute/GMAIL_SEND_EMAIL"
        })
        .expect("v3 execute request");
    assert_eq!(v3_execute.body["connected_account_id"], "conn-secret");
    assert_eq!(v3_execute.body["user_id"], "entity-round24");

    let v2_execute = requests
        .iter()
        .find(|request| {
            request.method == Method::POST
                && request.path == "/api/v2/actions/GMAIL_SEND_EMAIL/execute"
        })
        .expect("v2 execute fallback request");
    assert_eq!(v2_execute.body["entityId"], "entity-round24");

    assert!(requests.iter().any(|request| {
        request.method == Method::GET
            && request.path == "/api/v3/auth_configs"
            && request.query.contains("toolkit_slug=gmail")
            && request.query.contains("show_disabled=true")
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::POST
            && request.path == "/api/v3/connected_accounts/link"
            && request.body["auth_config_id"] == "auth-enabled"
            && request.body["user_id"] == "entity-round24"
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

async fn composio_handler(State(state): State<MockState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let method = parts.method;
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().unwrap_or_default().to_string();
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
            query,
            body: body_json,
        });

    match (method, path.as_str()) {
        (Method::GET, "/api/v3/connected_accounts") => Json(json!({
            "items": [
                { "id": "conn-gmail", "status": "ACTIVE", "toolkit": { "slug": "gmail" }, "created_at": "2026-05-30T00:00:00Z" },
                { "id": "", "status": "ACTIVE", "toolkit": "dropme" },
                { "id": "conn-github", "status": "INITIATED", "toolkit": "github", "createdAt": "2026-05-30T00:00:00Z" },
                { "id": "conn-slack", "status": "ACTIVE", "appName": "slack" }
            ]
        }))
        .into_response(),
        (Method::POST, "/api/v3/tools/execute/GMAIL_SEND_EMAIL") => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": "bad connected_account_id conn-secret for entity_id entity-round24"
                }
            })),
        )
            .into_response(),
        (Method::POST, "/api/v2/actions/GMAIL_SEND_EMAIL/execute") => Json(json!({
            "successful": true,
            "data": { "message": "v2-fallback-ok" }
        }))
        .into_response(),
        (Method::GET, "/api/v3/auth_configs") => Json(json!({
            "items": [
                { "id": "auth-disabled", "status": "disabled", "enabled": false },
                { "id": "auth-enabled", "status": "enabled", "enabled": true }
            ]
        }))
        .into_response(),
        (Method::POST, "/api/v3/connected_accounts/link") => Json(json!({
            "data": {
                "redirect_url": "https://connect.example.test/round24"
            }
        }))
        .into_response(),
        _ => (StatusCode::NOT_FOUND, Json(json!({ "message": "not found" }))).into_response(),
    }
}
