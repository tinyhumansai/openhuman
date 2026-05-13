use super::*;
use crate::openhuman::config::Config;

/// `build_composio_client` must return `None` when the user has no auth
/// token — callers treat that as "skip silently" (user not signed in).
#[test]
fn build_composio_client_none_without_auth_token() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    assert!(build_composio_client(&config).is_none());
}

#[test]
fn build_composio_client_some_with_auth_token() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    crate::openhuman::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::openhuman::credentials::APP_SESSION_PROVIDER,
            crate::openhuman::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "test-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");
    let client = build_composio_client(&config).expect("client should build when session is set");
    assert!(
        !client.inner().auth_token.is_empty(),
        "resolved auth token should not be empty"
    );
}

/// `authorize()` is input-validated — an empty / whitespace toolkit
/// must error without making any HTTP call.
#[tokio::test]
async fn authorize_rejects_empty_toolkit() {
    let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
        "http://127.0.0.1:0".into(),
        "test".into(),
    ));
    let client = ComposioClient::new(inner);
    let err = client.authorize("   ", None).await.unwrap_err();
    assert!(
        err.to_string().contains("toolkit must not be empty"),
        "unexpected error: {err}"
    );
}

/// `authorize()` must reject a non-object `extra_params` before making any HTTP call.
#[tokio::test]
async fn authorize_rejects_non_object_extra_params() {
    let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
        "http://127.0.0.1:0".into(),
        "test".into(),
    ));
    let client = ComposioClient::new(inner);
    let err = client
        .authorize("whatsapp", Some(serde_json::json!("waba-123")))
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("extra_params must be a JSON object"),
        "unexpected error: {err}"
    );
}

/// `authorize()` must reject an `extra_params` object that tries to override a reserved key.
#[tokio::test]
async fn authorize_rejects_reserved_key_override() {
    let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
        "http://127.0.0.1:0".into(),
        "test".into(),
    ));
    let client = ComposioClient::new(inner);
    let err = client
        .authorize("whatsapp", Some(serde_json::json!({ "toolkit": "gmail" })))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("cannot override reserved key"),
        "unexpected error: {err}"
    );
}

/// `delete_connection()` likewise must reject empty connection ids.
#[tokio::test]
async fn delete_connection_rejects_empty_id() {
    let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
        "http://127.0.0.1:0".into(),
        "test".into(),
    ));
    let client = ComposioClient::new(inner);
    let err = client.delete_connection("").await.unwrap_err();
    assert!(
        err.to_string().contains("connectionId must not be empty"),
        "unexpected error: {err}"
    );
}

/// `execute_tool()` must refuse empty slugs — otherwise the backend
/// would receive a malformed request.
#[tokio::test]
async fn execute_tool_rejects_empty_slug() {
    let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
        "http://127.0.0.1:0".into(),
        "test".into(),
    ));
    let client = ComposioClient::new(inner);
    let err = client.execute_tool("", None).await.unwrap_err();
    assert!(
        err.to_string().contains("tool slug must not be empty"),
        "unexpected error: {err}"
    );
}

/// ComposioClient is `Clone` so each tool gets a cheap handle share.
/// Inner client must be Arc-shared — no duplication.
#[test]
fn client_clone_shares_inner_arc() {
    let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
        "http://127.0.0.1:0".into(),
        "test".into(),
    ));
    let client_a = ComposioClient::new(inner);
    let client_b = client_a.clone();
    assert!(
        Arc::ptr_eq(client_a.inner(), client_b.inner()),
        "clones should share the same Arc<IntegrationClient>"
    );
}

// ── Mock-backend integration tests ─────────────────────────────
//
// These stand up a real axum HTTP server on a random localhost port,
// point a `ComposioClient` at it, and drive each method end-to-end.
// That exercises the envelope parsing, HTTP plumbing, and URL
// construction in `ComposioClient` — which is otherwise only covered
// by live backend tests.

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;

async fn start_mock_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn build_client_for(base_url: String) -> ComposioClient {
    let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
        base_url,
        "test-token".into(),
    ));
    ComposioClient::new(inner)
}

#[tokio::test]
async fn list_toolkits_parses_backend_envelope() {
    let app = Router::new().route(
        "/agent-integrations/composio/toolkits",
        get(|| async {
            Json(json!({
                "success": true,
                "data": { "toolkits": ["gmail", "notion"] }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client.list_toolkits().await.unwrap();
    assert_eq!(
        resp.toolkits,
        vec!["gmail".to_string(), "notion".to_string()]
    );
}

#[tokio::test]
async fn list_connections_parses_connection_array() {
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            Json(json!({
                "success": true,
                "data": {
                    "connections": [
                        { "id": "c1", "toolkit": "gmail", "status": "ACTIVE", "createdAt": "2026-01-01T00:00:00Z" },
                        { "id": "c2", "toolkit": "notion", "status": "PENDING" }
                    ]
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client.list_connections().await.unwrap();
    assert_eq!(resp.connections.len(), 2);
    assert_eq!(resp.connections[0].id, "c1");
    assert_eq!(resp.connections[1].status, "PENDING");
}

#[tokio::test]
async fn authorize_posts_toolkit_and_returns_connect_url() {
    let app = Router::new().route(
        "/agent-integrations/composio/authorize",
        post(|Json(body): Json<Value>| async move {
            // Echo toolkit back so we know our POST body made it.
            let tk = body["toolkit"].as_str().unwrap_or("").to_string();
            Json(json!({
                "success": true,
                "data": {
                    "connectUrl": format!("https://composio.example/{tk}/consent"),
                    "connectionId": "conn-abc"
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client.authorize("gmail", None).await.unwrap();
    assert!(resp.connect_url.contains("gmail"));
    assert_eq!(resp.connection_id, "conn-abc");
}

#[tokio::test]
async fn authorize_forwards_extra_params_and_returns_connect_url() {
    let app = Router::new().route(
        "/agent-integrations/composio/authorize",
        post(|Json(body): Json<Value>| async move {
            // Assert extra_params are forwarded alongside toolkit.
            assert_eq!(body["toolkit"].as_str(), Some("whatsapp"));
            assert_eq!(body["waba_id"].as_str(), Some("waba-123"));
            Json(json!({
                "success": true,
                "data": {
                    "connectUrl": "https://composio.example/whatsapp/consent",
                    "connectionId": "conn-xyz"
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let extra = serde_json::json!({ "waba_id": "waba-123" });
    let resp = client.authorize("whatsapp", Some(extra)).await.unwrap();
    assert!(resp.connect_url.contains("whatsapp"));
    assert_eq!(resp.connection_id, "conn-xyz");
}

#[tokio::test]
async fn list_tools_filters_pass_through_as_csv_query_param() {
    let app = Router::new().route(
        "/agent-integrations/composio/tools",
        get(|Query(q): Query<HashMap<String, String>>| async move {
            let filter = q.get("toolkits").cloned().unwrap_or_default();
            // Echo the requested filter back in the payload so the
            // test can assert it reached the server correctly.
            Json(json!({
                "success": true,
                "data": {
                    "tools": [{
                        "type": "function",
                        "function": {
                            "name": format!("ECHO_{filter}"),
                            "description": "echo",
                            "parameters": {}
                        }
                    }]
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);

    // No filter: URL should lack `toolkits` query
    let resp_all = client.list_tools(None).await.unwrap();
    assert_eq!(resp_all.tools.len(), 1);
    assert_eq!(resp_all.tools[0].function.name, "ECHO_");

    // With filter: CSV-joined
    let resp_filtered = client
        .list_tools(Some(&["gmail".to_string(), "notion".to_string()]))
        .await
        .unwrap();
    assert_eq!(resp_filtered.tools[0].function.name, "ECHO_gmail,notion");

    // Whitespace entries should be dropped before joining
    let resp_trimmed = client
        .list_tools(Some(&["gmail".to_string(), "  ".to_string()]))
        .await
        .unwrap();
    assert_eq!(resp_trimmed.tools[0].function.name, "ECHO_gmail");
}

#[tokio::test]
async fn execute_tool_returns_cost_and_success_flags() {
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|Json(body): Json<Value>| async move {
            let tool = body["tool"].as_str().unwrap_or("").to_string();
            Json(json!({
                "success": true,
                "data": {
                    "data": { "echoed_tool": tool },
                    "successful": true,
                    "error": null,
                    "costUsd": 0.0025
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client
        .execute_tool("GMAIL_SEND_EMAIL", Some(json!({"to": "a@b.com"})))
        .await
        .unwrap();
    assert!(resp.successful);
    assert!((resp.cost_usd - 0.0025).abs() < f64::EPSILON);
    assert_eq!(resp.data["echoed_tool"], "GMAIL_SEND_EMAIL");
}

#[tokio::test]
async fn execute_tool_without_arguments_sends_empty_object() {
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|Json(body): Json<Value>| async move {
            // Verify default arguments is an object (not missing / null).
            assert!(body["arguments"].is_object());
            Json(json!({
                "success": true,
                "data": {
                    "data": {},
                    "successful": true,
                    "error": null,
                    "costUsd": 0.0
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client.execute_tool("NOOP_ACTION", None).await.unwrap();
    assert!(resp.successful);
}

#[tokio::test]
async fn backend_error_envelope_becomes_bail() {
    let app = Router::new().route(
        "/agent-integrations/composio/toolkits",
        get(|| async { Json(json!({ "success": false, "error": "backend unavailable" })) }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let err = client.list_toolkits().await.unwrap_err();
    assert!(err.to_string().contains("backend unavailable"));
}

#[tokio::test]
async fn http_error_status_propagates() {
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let err = client.list_connections().await.unwrap_err();
    assert!(err.to_string().contains("500") || err.to_string().contains("Backend returned"));
}

#[tokio::test]
async fn delete_connection_happy_path_returns_deleted_true() {
    let app = Router::new().route(
        "/agent-integrations/composio/connections/{id}",
        axum::routing::delete(|Path(id): Path<String>| async move {
            assert_eq!(id, "conn-42");
            Json(json!({
                "success": true,
                "data": { "deleted": true }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client.delete_connection("conn-42").await.unwrap();
    assert!(resp.deleted);
}

// ── Trigger management (PR #671) ────────────────────────────────────

#[tokio::test]
async fn list_available_triggers_rejects_empty_toolkit() {
    let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
        "http://127.0.0.1:0".into(),
        "test".into(),
    ));
    let client = ComposioClient::new(inner);
    let err = client
        .list_available_triggers("   ", None)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("toolkit must not be empty"),
        "unexpected: {err}"
    );
}

#[tokio::test]
async fn list_available_triggers_forwards_query_params() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/available",
        get(|Query(q): Query<HashMap<String, String>>| async move {
            assert_eq!(q.get("toolkit").map(String::as_str), Some("github"));
            assert_eq!(q.get("connectionId").map(String::as_str), Some("c1"));
            Json(json!({
                "success": true,
                "data": {"triggers": [{"slug": "GITHUB_PUSH_EVENT", "scope": "github_repo"}]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client
        .list_available_triggers("github", Some("c1"))
        .await
        .unwrap();
    assert_eq!(resp.triggers.len(), 1);
    assert_eq!(resp.triggers[0].scope, "github_repo");
}

#[tokio::test]
async fn list_active_triggers_filters_by_toolkit() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers",
        get(|Query(q): Query<HashMap<String, String>>| async move {
            assert_eq!(q.get("toolkit").map(String::as_str), Some("gmail"));
            Json(json!({
                "success": true,
                "data": {"triggers": []}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client.list_active_triggers(Some("gmail")).await.unwrap();
    assert!(resp.triggers.is_empty());
}

#[tokio::test]
async fn enable_trigger_rejects_empty_inputs() {
    let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
        "http://127.0.0.1:0".into(),
        "test".into(),
    ));
    let client = ComposioClient::new(inner);

    let err = client.enable_trigger("", "X", None).await.unwrap_err();
    assert!(err.to_string().contains("connectionId must not be empty"));

    let err = client.enable_trigger("c1", "  ", None).await.unwrap_err();
    assert!(err.to_string().contains("slug must not be empty"));
}

#[tokio::test]
async fn enable_trigger_posts_body_and_parses_response() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["connectionId"], "c1");
            assert_eq!(body["slug"], "GMAIL_NEW_GMAIL_MESSAGE");
            assert_eq!(body["triggerConfig"]["labelIds"], "INBOX");
            Json(json!({
                "success": true,
                "data": {
                    "triggerId": "ti_1",
                    "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                    "connectionId": "c1"
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client
        .enable_trigger(
            "c1",
            "GMAIL_NEW_GMAIL_MESSAGE",
            Some(json!({"labelIds": "INBOX"})),
        )
        .await
        .unwrap();
    assert_eq!(resp.trigger_id, "ti_1");
}

#[tokio::test]
async fn disable_trigger_rejects_empty_id() {
    let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
        "http://127.0.0.1:0".into(),
        "test".into(),
    ));
    let client = ComposioClient::new(inner);
    let err = client.disable_trigger("").await.unwrap_err();
    assert!(err.to_string().contains("triggerId must not be empty"));
}

#[tokio::test]
async fn disable_trigger_calls_delete_path() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/{id}",
        axum::routing::delete(|Path(id): Path<String>| async move {
            assert_eq!(id, "ti_1");
            Json(json!({"success": true, "data": {"deleted": true}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client.disable_trigger("ti_1").await.unwrap();
    assert!(resp.deleted);
}

#[tokio::test]
async fn disable_trigger_surfaces_non_2xx_status() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/{id}",
        axum::routing::delete(|Path(_id): Path<String>| async move {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"success": false, "error": "no"})),
            )
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let err = client.disable_trigger("ti_x").await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("404"), "expected status 404, got: {msg}");
    // Phase A (#1296): raw_delete must propagate the envelope's `error`
    // field so callers can tell *why* the backend rejected the call.
    assert!(
        msg.contains("no"),
        "expected envelope error detail in message, got: {msg}"
    );
}

#[tokio::test]
async fn delete_connection_surfaces_envelope_error_detail() {
    // Direct cover of the `raw_delete` envelope-error path used by
    // `delete_connection` — proves the backend message ("Connection
    // not found") makes it into the propagated bail message rather
    // than being discarded with the body. Mirror of the `post`/`get`
    // envelope tests in `integrations/client_tests.rs`.
    let app = Router::new().route(
        "/agent-integrations/composio/connections/{id}",
        axum::routing::delete(|Path(_id): Path<String>| async move {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"success": false, "error": "Connection not found"})),
            )
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let err = client.delete_connection("missing-id").await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("Connection not found"),
        "expected backend error detail in message, got: {msg}"
    );
    assert!(msg.contains("400"), "expected status 400, got: {msg}");
}
