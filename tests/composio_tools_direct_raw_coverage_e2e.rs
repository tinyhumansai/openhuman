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
use tempfile::tempdir;

use openhuman_core::openhuman::composio::client::{direct_execute, direct_list_connections};
use openhuman_core::openhuman::composio::trigger_history::ComposioTriggerHistoryStore;
use openhuman_core::openhuman::security::{AutonomyLevel, SecurityPolicy};
use openhuman_core::openhuman::tools::{ComposioTool, Tool};

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
async fn direct_composio_tool_uses_loopback_for_list_execute_connect_and_accounts() {
    let state = MockState::default();
    let app = Router::new()
        .fallback(any(composio_direct_handler))
        .with_state(state.clone());
    let base = start_loopback(app).await;
    let tool = Arc::new(
        ComposioTool::new_with_base_urls_for_loopback(
            " ck_round16 ",
            Some(" entity-round16 "),
            writable_security(),
            format!("{base}/api/v2"),
            format!("{base}/api/v3"),
        )
        .expect("loopback direct tool"),
    );

    let actions = tool
        .list_actions(Some(" gmail "))
        .await
        .expect("v3 actions");
    assert_eq!(actions.len(), 2);
    assert!(actions
        .iter()
        .any(|action| action.name == "gmail-fetch-emails"
            && action.app_name.as_deref() == Some("gmail")));
    assert!(actions
        .iter()
        .any(|action| action.name == "gmail-send-email"
            && action.description.as_deref() == Some("Send Gmail")));

    let listed = tool
        .execute(json!({ "action": "list", "app": "gmail" }))
        .await
        .expect("tool list action");
    assert!(!listed.is_error);
    assert!(listed.output().contains("Found 2 available actions"));

    let raw_execute = tool
        .execute_action(
            " GMAIL_FETCH_EMAILS ",
            json!({ "query": "label:INBOX" }),
            Some(" entity-override "),
            Some(" acct-gmail "),
        )
        .await
        .expect("v3 execute action");
    assert_eq!(
        raw_execute.pointer("/data/messages/0/id"),
        Some(&json!("msg-direct"))
    );

    let direct_response = direct_execute(
        &tool,
        "GMAIL_FETCH_EMAILS",
        Some(json!({ "query": "from:me" })),
        " entity-direct ",
        None,
    )
    .await
    .expect("direct execute envelope");
    assert!(direct_response.successful);
    assert_eq!(
        direct_response.data.pointer("/messages/0/id"),
        Some(&json!("msg-direct"))
    );
    assert_eq!(direct_response.cost_usd, 0.0);

    let fallback_execute = tool
        .execute_action("FALLBACK_ACTION", json!({ "ok": true }), None, None)
        .await
        .expect("v2 execute fallback");
    assert_eq!(fallback_execute.pointer("/legacy"), Some(&json!(true)));

    let failed = tool
        .execute_action(
            "BROKEN_ACTION",
            json!({ "connected_account_id": "acct-secret" }),
            Some("entity-secret"),
            Some("acct-secret"),
        )
        .await
        .expect_err("both v3 and v2 fail");
    let failed = failed.to_string();
    assert!(failed.contains("Composio execute failed on v3"));
    assert!(failed.contains("[redacted]"));

    let linked_by_toolkit = tool
        .get_connection_url(Some("gmail"), None, "entity-round16")
        .await
        .expect("connect via resolved auth config");
    assert_eq!(linked_by_toolkit, "https://connect.example/from-data");

    let linked_by_auth_config = tool
        .get_connection_url(None, Some("auth-explicit"), "entity-round16")
        .await
        .expect("connect via explicit auth config");
    assert_eq!(
        linked_by_auth_config,
        "https://connect.example/from-redirect-url"
    );

    let missing_connect = tool
        .get_connection_url(None, None, "entity-round16")
        .await
        .expect_err("connect needs app or auth config")
        .to_string();
    assert!(missing_connect.contains("Missing 'app' or 'auth_config_id'"));

    let accounts = tool
        .list_connected_accounts()
        .await
        .expect("connected accounts");
    assert_eq!(accounts.len(), 4);
    assert_eq!(accounts[0].toolkit_slug().as_deref(), Some("gmail"));
    assert_eq!(accounts[1].toolkit_slug().as_deref(), Some("github"));
    assert_eq!(accounts[2].toolkit_slug().as_deref(), Some("slack"));
    assert_eq!(accounts[3].toolkit_slug(), None);

    let mapped = direct_list_connections(&tool)
        .await
        .expect("mapped connected accounts");
    assert_eq!(mapped.connections.len(), 4);
    assert!(mapped
        .connections
        .iter()
        .any(|conn| conn.id == "acct-github" && conn.toolkit == "github"));

    let execute_result = tool
        .execute(json!({
            "action": "execute",
            "tool_slug": "GMAIL_FETCH_EMAILS",
            "params": { "query": "newer_than:1d" },
            "connected_account_id": "acct-gmail"
        }))
        .await
        .expect("tool execute action");
    assert!(!execute_result.is_error);
    assert!(execute_result.output().contains("msg-direct"));

    let connect_result = tool
        .execute(json!({ "action": "connect", "auth_config_id": "auth-explicit" }))
        .await
        .expect("tool connect action");
    assert!(!connect_result.is_error);
    assert!(connect_result
        .output()
        .contains("https://connect.example/from-redirect-url"));

    let unknown = tool
        .execute(json!({ "action": "unknown" }))
        .await
        .expect("unknown action returns tool error");
    assert!(unknown.is_error);
    assert!(unknown.output().contains("Unknown action"));

    let missing_action = tool.execute(json!({})).await.expect_err("missing action");
    assert!(missing_action.to_string().contains("Missing 'action'"));

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().all(|request| {
        request.api_key.as_deref() == Some("ck_round16") || request.path == "/health"
    }));
    assert!(requests.iter().any(|request| {
        request.method == "GET"
            && request.path == "/api/v3/tools"
            && request.query.contains("toolkits=gmail")
            && request.query.contains("limit=200")
    }));
    assert!(requests.iter().any(|request| {
        request.method == "POST"
            && request.path == "/api/v3/tools/execute/GMAIL_FETCH_EMAILS"
            && request.body.pointer("/user_id") == Some(&json!(" entity-override "))
            && request.body.pointer("/connected_account_id") == Some(&json!("acct-gmail"))
    }));
    assert!(requests.iter().any(|request| {
        request.method == "POST"
            && request.path == "/api/v2/actions/FALLBACK_ACTION/execute"
            && request.body.pointer("/input/ok") == Some(&json!(true))
    }));
}

#[test]
fn trigger_history_lists_newest_entries_and_skips_bad_jsonl_lines() {
    let dir = tempdir().expect("tempdir");
    let store = ComposioTriggerHistoryStore::new(dir.path()).expect("history store");

    let first = store
        .record_trigger(
            "gmail",
            "GMAIL_NEW_GMAIL_MESSAGE",
            "meta-1",
            "uuid-1",
            &json!({ "message": { "id": "msg-1" } }),
        )
        .expect("first trigger");
    let second = store
        .record_trigger(
            "github",
            "GITHUB_PULL_REQUEST_EVENT",
            "meta-2",
            "uuid-2",
            &json!({ "repo": "openhuman" }),
        )
        .expect("second trigger");

    let current_file = store.list_recent(10).expect("history").current_day_file;
    std::fs::write(
        dir.path().join("state").join("triggers").join("2000-01-01.jsonl"),
        "\nnot-json\n{\"received_at_ms\":1,\"toolkit\":\"slack\",\"trigger\":\"SLACK_EVENT\",\"metadata_id\":\"meta-0\",\"metadata_uuid\":\"uuid-0\",\"payload\":{\"ok\":true}}\n",
    )
    .expect("old jsonl");
    std::fs::write(
        dir.path().join("state").join("triggers").join("ignore.txt"),
        "{\"toolkit\":\"ignored\"}\n",
    )
    .expect("ignored extension");

    let recent = store.list_recent(2).expect("limited history");
    assert_eq!(recent.entries.len(), 2);
    assert_eq!(recent.entries[0].metadata_id, second.metadata_id);
    assert_eq!(recent.entries[1].metadata_id, first.metadata_id);
    assert_eq!(recent.current_day_file, current_file);
    assert!(recent.archive_dir.ends_with("state/triggers"));

    let all = store.list_recent(0).expect("limit zero coerces to one");
    assert_eq!(all.entries.len(), 1);
    assert_eq!(all.entries[0].metadata_uuid, second.metadata_uuid);
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

fn writable_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        ..SecurityPolicy::default()
    })
}
