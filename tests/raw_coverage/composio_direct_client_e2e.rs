//! Raw integration coverage for direct Composio tools.
//!
//! This binary stays on loopback mocks and temp stores. It exercises the
//! direct BYO-key tool surface without contacting Composio.

use std::sync::{Arc, Mutex};

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};

use openhuman_core::config::Config;
use openhuman_core::integrations::composio::client::{direct_list_connections, DirectCredential};

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
    api_key: Option<String>,
}

#[tokio::test]
async fn direct_composio_client_uses_loopback_for_connected_accounts() {
    // The read runs in the connector module, which is process-global.
    let _module = crate::CONNECTOR_MODULE_LOCK.lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    let state = MockState::default();
    let app = Router::new()
        .fallback(any(composio_direct_handler))
        .with_state(state.clone());
    let base = start_loopback(app).await;
    let tool = Arc::new(
        DirectCredential::new_with_base_urls_for_loopback(
            " ck_round16 ",
            format!("{base}/api/v2"),
            format!("{base}/api/v3"),
        )
        .expect("loopback direct client"),
    );

    let mapped = direct_list_connections(&config, &tool)
        .await
        .expect("mapped connected accounts");
    assert_eq!(mapped.connections.len(), 4);
    assert!(mapped
        .connections
        .iter()
        .any(|conn| conn.id == "acct-github" && conn.toolkit == "github"));
    // Slug extraction: padded string, nested object, `appName` fallback, and a
    // row with no recognizable slug is kept with an empty toolkit.
    let toolkits: Vec<&str> = mapped
        .connections
        .iter()
        .map(|c| c.toolkit.as_str())
        .collect();
    assert_eq!(toolkits[..2], ["gmail", "github"]);
    assert_eq!(toolkits[2], "slack");
    assert_eq!(toolkits[3], "");

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().all(|request| {
        request.api_key.as_deref() == Some("ck_round16") || request.path == "/health"
    }));
    assert!(requests.iter().any(|request| {
        request.method == "GET"
            && request.path == "/api/v3/connected_accounts"
            && request.query.contains("limit=200")
    }));
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
        .map(ToString::to_string);
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
            query,
            body: body.clone(),
            api_key,
        });

    match (method, path.as_str()) {
        (Method::GET, "/api/v3/tools") => Json(json!({
            "items": [
                {
                    "slug": "gmail-fetch-emails",
                    "name": "Gmail fetch fallback",
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
                    "appName": "gmail",
                    "parameters": { "type": "object" }
                },
                {
                    "description": "dropped because it has no slug or name",
                    "toolkit": { "slug": "gmail" }
                }
            ]
        }))
        .into_response(),
        (Method::POST, "/api/v3/tools/execute/GMAIL_FETCH_EMAILS") => Json(json!({
            "successful": true,
            "data": {
                "messages": [{ "id": "msg-direct", "subject": "hello" }]
            }
        }))
        .into_response(),
        (Method::POST, "/api/v3/tools/execute/FALLBACK_ACTION") => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": { "message": "temporary v3 outage" }
            })),
        )
            .into_response(),
        (Method::POST, "/api/v2/actions/FALLBACK_ACTION/execute") => Json(json!({
            "legacy": true,
            "input": body.get("input").cloned().unwrap_or_else(|| json!({}))
        }))
        .into_response(),
        (Method::POST, "/api/v3/tools/execute/BROKEN_ACTION") => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": "bad connected_account_id acct-secret for entity_id entity-secret"
                }
            })),
        )
            .into_response(),
        (Method::POST, "/api/v2/actions/BROKEN_ACTION/execute") => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "message": "legacy connectedAccountId acct-secret and entityId entity-secret failed"
            })),
        )
            .into_response(),
        (Method::GET, "/api/v3/auth_configs") => Json(json!({
            "items": [
                { "id": "auth-disabled", "enabled": false, "status": "disabled" },
                { "id": "auth-enabled", "status": "ENABLED" }
            ]
        }))
        .into_response(),
        (Method::POST, "/api/v3/connected_accounts/link") => {
            let auth_config = body
                .get("auth_config_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if auth_config == "auth-explicit" {
                Json(json!({ "redirectUrl": "https://connect.example/from-redirect-url" }))
                    .into_response()
            } else {
                Json(json!({
                    "data": {
                        "redirect_url": "https://connect.example/from-data"
                    }
                }))
                .into_response()
            }
        }
        (Method::GET, "/api/v3/connected_accounts") => Json(json!({
            "items": [
                {
                    "id": "acct-gmail",
                    "status": "ACTIVE",
                    "created_at": "2026-05-29T12:00:00Z",
                    "toolkit": " gmail "
                },
                {
                    "id": "acct-github",
                    "status": "INITIATED",
                    "createdAt": "2026-05-29T12:01:00Z",
                    "toolkit": { "slug": "github" }
                },
                {
                    "id": "acct-slack",
                    "status": "FAILED",
                    "app_name": "slack"
                },
                {
                    "id": "acct-empty-toolkit",
                    "status": "ACTIVE",
                    "toolkit": null
                },
                {
                    "id": "   ",
                    "status": "ACTIVE",
                    "toolkit": "not-returned"
                }
            ]
        }))
        .into_response(),
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": { "message": format!("unhandled {path}") } })),
        )
            .into_response(),
    }
}

async fn start_loopback(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock composio direct server");
    let addr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://127.0.0.1:{}", addr.port())
}
