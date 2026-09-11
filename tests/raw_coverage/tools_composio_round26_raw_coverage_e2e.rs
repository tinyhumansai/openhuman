//! Round26 raw/E2E coverage for tools + Composio cold network paths.
//!
//! All outbound HTTP is routed to loopback mocks. The tests drive the public
//! tool surfaces so coverage lands on the same paths used by agent calls.

use std::sync::{Arc, Mutex};

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};

use openhuman_core::openhuman::security::{AutonomyLevel, SecurityPolicy};
use openhuman_core::openhuman::tools::{ComposioTool, PermissionLevel, Tool};

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
}

#[tokio::test]
async fn round26_composio_direct_tool_covers_connect_execute_and_error_fallbacks() {
    let state = MockState::default();
    let base = start_loopback(
        Router::new()
            .fallback(any(composio_handler))
            .with_state(state.clone()),
    )
    .await;
    let tool = ComposioTool::new_with_base_urls_for_loopback(
        " ck_round26 ",
        Some(" entity-round26 "),
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            ..SecurityPolicy::default()
        }),
        format!("{base}/api/v2"),
        format!("{base}/api/v3"),
    )
    .expect("loopback composio");

    assert_eq!(tool.name(), "composio");
    assert!(tool.external_effect());
    assert!(!tool.external_effect_with_args(&json!({ "action": "list" })));
    assert!(!tool.external_effect_with_args(&json!({ "action": "connect" })));
    assert!(tool.external_effect_with_args(&json!({ "action": "execute" })));

    let missing_action = tool
        .execute(json!({}))
        .await
        .expect_err("missing action is an anyhow validation error");
    assert!(missing_action.to_string().contains("Missing 'action'"));

    let unknown = tool
        .execute(json!({ "action": "inspect" }))
        .await
        .expect("unknown action returns tool result");
    assert!(unknown.is_error);
    assert!(unknown.output().contains("Unknown action"));

    let listed = tool
        .execute(json!({ "action": "list", "app": "fallback-list" }))
        .await
        .expect("list with v2 fallback");
    assert!(!listed.is_error, "{}", listed.output());
    assert!(listed.output().contains("LEGACY_ROUND26_ACTION"));

    let executed = tool
        .execute(json!({
            "action": "execute",
            "tool_slug": "ROUND26_ACTION",
            "params": { "value": 42 },
            "connected_account_id": " account-round26 "
        }))
        .await
        .expect("execute v3 success");
    assert!(!executed.is_error, "{}", executed.output());
    assert!(executed.output().contains("v3-execute-round26"));

    let v2_execute = tool
        .execute(json!({
            "action": "execute",
            "action_name": "ROUND26_V2_ONLY",
            "params": { "value": "fallback" }
        }))
        .await
        .expect("execute v2 fallback");
    assert!(!v2_execute.is_error, "{}", v2_execute.output());
    assert!(v2_execute.output().contains("v2-execute-round26"));

    let direct_auth_config = tool
        .execute(json!({
            "action": "connect",
            "auth_config_id": "auth-direct-round26"
        }))
        .await
        .expect("connect via auth_config_id");
    assert!(
        !direct_auth_config.is_error,
        "{}",
        direct_auth_config.output()
    );
    assert!(direct_auth_config
        .output()
        .contains("https://connect.example.test/direct-round26"));

    let v2_connect = tool
        .execute(json!({
            "action": "connect",
            "app": "fallback-connect"
        }))
        .await
        .expect("connect v2 fallback");
    assert!(!v2_connect.is_error, "{}", v2_connect.output());
    assert!(v2_connect
        .output()
        .contains("https://connect.example.test/v2-round26"));

    let missing_auth = tool
        .execute(json!({
            "action": "connect",
            "app": "missing-auth"
        }))
        .await
        .expect("missing auth config returns tool result");
    assert!(missing_auth.is_error);
    assert!(missing_auth.output().contains("No auth config found"));

    let failed_no_app_fallback = tool
        .execute(json!({
            "action": "connect",
            "auth_config_id": "auth-link-fails-round26"
        }))
        .await
        .expect("v3 failure without app returns tool result");
    assert!(failed_no_app_fallback.is_error);
    assert!(failed_no_app_fallback
        .output()
        .contains("v2 fallback requires 'app'"));

    let requests = state.requests.lock().expect("requests").clone();
    let v3_execute = requests
        .iter()
        .find(|request| {
            request.method == Method::POST && request.path == "/api/v3/tools/execute/ROUND26_ACTION"
        })
        .expect("v3 execute request");
    assert_eq!(v3_execute.body["user_id"], "entity-round26");
    assert_eq!(v3_execute.body["connected_account_id"], "account-round26");

    assert!(requests.iter().any(|request| {
        request.method == Method::GET
            && request.path == "/api/v3/tools"
            && request.query.contains("toolkits=fallback-list")
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::POST
            && request.path == "/api/v2/connectedAccounts"
            && request.body["integrationId"] == "fallback-connect"
            && request.body["entityId"] == "entity-round26"
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
    let (method, path, query, body_json) = record_request_parts(request).await;
    state
        .requests
        .lock()
        .expect("requests")
        .push(RecordedRequest {
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            body: body_json,
        });

    match (method, path.as_str()) {
        (Method::GET, "/api/v3/tools") if query.contains("toolkits=fallback-list") => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": { "message": "v3 list unavailable" } })),
        )
            .into_response(),
        (Method::GET, "/api/v2/actions") => Json(json!({
            "items": [
                {
                    "name": "LEGACY_ROUND26_ACTION",
                    "appName": "legacy",
                    "description": "legacy list fallback",
                    "enabled": true
                }
            ]
        }))
        .into_response(),
        (Method::POST, "/api/v3/tools/execute/ROUND26_ACTION") => Json(json!({
            "successful": true,
            "data": { "message": "v3-execute-round26" }
        }))
        .into_response(),
        (Method::POST, "/api/v3/tools/execute/ROUND26_V2_ONLY") => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "message": "v3 execute unavailable" })),
        )
            .into_response(),
        (Method::POST, "/api/v2/actions/ROUND26_V2_ONLY/execute") => Json(json!({
            "successful": true,
            "data": { "message": "v2-execute-round26" }
        }))
        .into_response(),
        (Method::POST, "/api/v3/connected_accounts/link")
            if path.as_str() == "/api/v3/connected_accounts/link" =>
        {
            match state.requests.lock().expect("requests").last() {
                Some(record) if record.body["auth_config_id"] == "auth-direct-round26" => {
                    Json(json!({
                        "redirectUrl": "https://connect.example.test/direct-round26"
                    }))
                    .into_response()
                }
                Some(record) if record.body["auth_config_id"] == "auth-link-fails-round26" => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "message": "link failed for round26" })),
                )
                    .into_response(),
                Some(record) if record.body["auth_config_id"] == "auth-fallback-round26" => (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({ "message": "force v2 connect fallback" })),
                )
                    .into_response(),
                _ => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "message": "unexpected auth config" })),
                )
                    .into_response(),
            }
        }
        (Method::GET, "/api/v3/auth_configs") if query.contains("toolkit_slug=missing-auth") => {
            Json(json!({ "items": [] })).into_response()
        }
        (Method::GET, "/api/v3/auth_configs")
            if query.contains("toolkit_slug=fallback-connect") =>
        {
            Json(json!({
                "items": [
                    { "id": "auth-fallback-round26", "status": "enabled" }
                ]
            }))
            .into_response()
        }
        (Method::POST, "/api/v2/connectedAccounts") => {
            match state.requests.lock().expect("requests").last() {
                Some(record) if record.body["integrationId"] == "missing-auth" => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "message": "v2 missing auth config" })),
                )
                    .into_response(),
                _ => Json(json!({
                    "redirectUrl": "https://connect.example.test/v2-round26"
                }))
                .into_response(),
            }
        }
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "not found" })),
        )
            .into_response(),
    }
}

async fn record_request_parts(request: Request) -> (Method, String, String, Value) {
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
    (method, path, query, body_json)
}
