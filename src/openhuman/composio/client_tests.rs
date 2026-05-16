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
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

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
async fn execute_tool_retries_once_on_post_oauth_auth_readiness_error() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/agent-integrations/composio/execute",
            post(|State(attempts): State<Arc<AtomicUsize>>| async move {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    Json(json!({
                        "success": true,
                        "data": {
                            "data": {},
                            "successful": false,
                            "error": "Connection error, try to authenticate",
                            "costUsd": 0.0
                        }
                    }))
                } else {
                    Json(json!({
                        "success": true,
                        "data": {
                            "data": {"ok": true},
                            "successful": true,
                            "error": null,
                            "costUsd": 0.001
                        }
                    }))
                }
            }),
        )
        .with_state(attempts.clone());
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);

    let resp = client
        .execute_tool_with_post_oauth_retry(
            "GOOGLECALENDAR_EVENTS_LIST",
            &json!({
                "tool": "GOOGLECALENDAR_EVENTS_LIST",
                "arguments": {}
            }),
            std::time::Duration::ZERO,
        )
        .await
        .unwrap();

    assert!(resp.successful);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[test]
fn post_oauth_auth_readiness_error_matches_known_gateway_variants() {
    for err in [
        "Connection error, try to authenticate",
        "connection error, try to authenticate",
        "CONNECTION ERROR, TRY TO AUTHENTICATE",
        "Action failed: Connection error, try to authenticate (gateway code 401)",
    ] {
        assert!(
            is_post_oauth_auth_readiness_error(&ComposioExecuteResponse {
                data: json!({}),
                successful: false,
                error: Some(err.to_string()),
                cost_usd: 0.0,
                markdown_formatted: None,
            }),
            "should classify retryable Composio auth-readiness error: {err}"
        );
    }
}

#[test]
fn post_oauth_auth_readiness_error_rejects_unrelated_or_successful_payloads() {
    for err in ["invalid_grant", "ratelimited", ""] {
        assert!(
            !is_post_oauth_auth_readiness_error(&ComposioExecuteResponse {
                data: json!({}),
                successful: false,
                error: Some(err.to_string()),
                cost_usd: 0.0,
                markdown_formatted: None,
            }),
            "should not classify unrelated error as retryable: {err}"
        );
    }

    assert!(!is_post_oauth_auth_readiness_error(
        &ComposioExecuteResponse {
            data: json!({}),
            successful: true,
            error: Some("Connection error, try to authenticate".to_string()),
            cost_usd: 0.0,
            markdown_formatted: None,
        }
    ));
}

#[tokio::test]
async fn execute_tool_does_not_retry_other_auth_errors() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/agent-integrations/composio/execute",
            post(|State(attempts): State<Arc<AtomicUsize>>| async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "success": true,
                    "data": {
                        "data": {},
                        "successful": false,
                        "error": "Invalid OAuth scope",
                        "costUsd": 0.0
                    }
                }))
            }),
        )
        .with_state(attempts.clone());
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);

    let resp = client
        .execute_tool_with_post_oauth_retry(
            "GOOGLECALENDAR_EVENTS_LIST",
            &json!({
                "tool": "GOOGLECALENDAR_EVENTS_LIST",
                "arguments": {}
            }),
            std::time::Duration::ZERO,
        )
        .await
        .unwrap();

    assert!(!resp.successful);
    assert_eq!(resp.error.as_deref(), Some("Invalid OAuth scope"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
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

// ── execute_tool resilience tests (Batch 1 — post-OAuth readiness) ─────

/// When the backend returns `{ "successful": false, "error": "..." }` inside
/// the data envelope, `execute_tool` should still succeed at the HTTP level
/// (the envelope `success: true`) but surface the failure via the `successful`
/// flag on [`ComposioExecuteResponse`]. Callers like `composio_execute` in
/// `ops.rs` inspect `resp.successful` and propagate the inner error.
///
/// This is the shape the backend sends during the post-OAuth readiness gap
/// (e.g. "App not authorized yet") — the outer `success: true` means the
/// proxy reached Composio; `successful: false` means Composio itself rejected
/// the action.
#[tokio::test]
async fn execute_tool_surfaces_non_successful_provider_response() {
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|| async {
            Json(json!({
                "success": true,
                "data": {
                    "data": {},
                    "successful": false,
                    "error": "App not authorized yet — please complete OAuth first",
                    "costUsd": 0.0
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client.execute_tool("GMAIL_SEND_EMAIL", None).await.unwrap();
    assert!(
        !resp.successful,
        "non-successful provider response must be surfaced via the successful flag"
    );
    let err = resp.error.expect("error field must be present on failure");
    assert!(
        err.contains("not authorized"),
        "error message must pass through verbatim; got: {err}"
    );
    assert_eq!(resp.cost_usd, 0.0, "zero cost on failure");
}

/// A revoked-token error is a distinct failure mode from a transient
/// readiness gap — both manifest as `successful: false` but with different
/// error strings. The client must not swallow either; both must surface
/// in the `error` field so callers can classify them.
#[tokio::test]
async fn execute_tool_surfaces_revoked_token_error() {
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|| async {
            Json(json!({
                "success": true,
                "data": {
                    "data": {},
                    "successful": false,
                    "error": "Token revoked: the user has disconnected their account",
                    "costUsd": 0.0
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client
        .execute_tool("GMAIL_FETCH_EMAILS", None)
        .await
        .unwrap();
    assert!(!resp.successful);
    let err = resp.error.unwrap();
    assert!(
        err.contains("revoked"),
        "revoked-token message must be preserved verbatim; got: {err}"
    );
}

/// A transport-level 5xx is distinct from a provider-level failure: it
/// means the backend itself failed before reaching Composio. This must
/// surface as an `Err` from `execute_tool`, not as `successful: false`,
/// so callers that only inspect the flag don't silently swallow the
/// outage.
#[tokio::test]
async fn execute_tool_propagates_backend_5xx_as_err() {
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let result = client.execute_tool("ANY_TOOL", None).await;
    assert!(
        result.is_err(),
        "5xx backend must be an Err, not Ok(unsuccessful)"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("500") || msg.contains("Backend returned"),
        "5xx error message must contain status code; got: {msg}"
    );
}

/// `execute_tool` must forward the `tool` field in the request body so
/// the backend knows which action to proxy to Composio. Regression guard
/// for any future refactor that touches the body builder.
#[tokio::test]
async fn execute_tool_sends_tool_slug_in_request_body() {
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|Json(body): Json<Value>| async move {
            let tool_field = body["tool"].as_str().unwrap_or("").to_string();
            Json(json!({
                "success": true,
                "data": {
                    "data": { "received_tool": tool_field },
                    "successful": true,
                    "error": null,
                    "costUsd": 0.0
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);
    let resp = client
        .execute_tool("JIRA_CREATE_ISSUE", Some(json!({"project": "OH"})))
        .await
        .unwrap();
    assert!(resp.successful);
    assert_eq!(
        resp.data["received_tool"], "JIRA_CREATE_ISSUE",
        "tool slug must be forwarded in request body"
    );
}
// ── Factory tests (`create_composio_client`) ────────────────────────
//
// Mirror the four branches the spec demands:
//   1. backend mode with a session JWT — Backend variant
//   2. direct mode + stored api key — Direct variant
//   3. direct mode without api key — explicit error
//   4. unknown mode string — explicit error

fn config_with_session_token(tmp: &tempfile::TempDir) -> crate::openhuman::config::Config {
    let mut config = crate::openhuman::config::Config::default();
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
    config
}

#[test]
fn create_composio_client_backend_variant_when_mode_default() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = config_with_session_token(&tmp);
    let kind = create_composio_client(&config).expect("backend mode should build");
    assert_eq!(kind.mode(), "backend");
    assert!(matches!(kind, ComposioClientKind::Backend(_)));
}

#[test]
fn create_composio_client_backend_empty_mode_falls_back_to_backend() {
    // A literal empty string in TOML should be treated as the default
    // (`"backend"`) rather than an unknown mode error.
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = config_with_session_token(&tmp);
    config.composio.mode = String::new();
    let kind = create_composio_client(&config).expect("empty mode should fall back to backend");
    assert_eq!(kind.mode(), "backend");
}

#[test]
fn create_composio_client_backend_errors_without_session() {
    // Backend mode requires the app-session JWT — without it the
    // factory must return an explicit error (not silently downgrade).
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    let err = create_composio_client(&config)
        .err()
        .expect("must error without auth token");
    assert!(
        err.to_string().contains("no backend session token"),
        "unexpected error: {err}"
    );
}

#[test]
fn create_composio_client_direct_variant_with_stored_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = "direct".into();
    // Persist the key the way the RPC layer would.
    crate::openhuman::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::openhuman::credentials::COMPOSIO_DIRECT_PROVIDER,
            crate::openhuman::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "ck_test_key_redacted",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store direct api key");
    let kind = create_composio_client(&config).expect("direct mode with stored key should build");
    assert_eq!(kind.mode(), "direct");
    assert!(matches!(kind, ComposioClientKind::Direct(_)));
}

#[test]
fn create_composio_client_direct_falls_back_to_config_api_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = "direct".into();
    // No keychain entry — fall back to the inline config field.
    config.composio.api_key = Some("ck_inline_redacted".into());
    let kind = create_composio_client(&config)
        .expect("direct mode should accept inline config.api_key when keychain is empty");
    assert_eq!(kind.mode(), "direct");
}

#[test]
fn create_composio_client_direct_errors_without_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = "direct".into();
    let err = create_composio_client(&config)
        .err()
        .expect("direct without key must error");
    let msg = err.to_string();
    assert!(
        msg.contains("no api key is configured"),
        "unexpected error: {msg}"
    );
}

#[test]
fn create_composio_client_unknown_mode_errors() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = "voyage".into();
    let err = create_composio_client(&config)
        .err()
        .expect("unknown mode must error");
    let msg = err.to_string();
    assert!(msg.contains("unknown composio mode"), "got: {msg}");
    assert!(
        msg.contains("voyage"),
        "should echo the invalid value, got: {msg}"
    );
}

// ── Direct-mode credentials helpers ─────────────────────────────────

#[test]
fn store_get_clear_composio_api_key_roundtrip() {
    use crate::openhuman::credentials::{get_composio_api_key, COMPOSIO_DIRECT_PROVIDER};

    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");

    // Initially: nothing stored.
    assert_eq!(
        get_composio_api_key(&config).expect("read empty store"),
        None
    );

    // Store under the direct-mode provider slot.
    crate::openhuman::credentials::AuthService::from_config(&config)
        .store_provider_token(
            COMPOSIO_DIRECT_PROVIDER,
            crate::openhuman::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "ck_secret_value_redacted",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store");

    assert_eq!(
        get_composio_api_key(&config).expect("read stored"),
        Some("ck_secret_value_redacted".into())
    );

    // Clearing the profile must remove it again.
    crate::openhuman::credentials::AuthService::from_config(&config)
        .remove_profile(
            COMPOSIO_DIRECT_PROVIDER,
            crate::openhuman::credentials::DEFAULT_AUTH_PROFILE_NAME,
        )
        .expect("remove");
    assert_eq!(
        get_composio_api_key(&config).expect("read post-clear"),
        None
    );
}

// ── Pricing short-circuit ───────────────────────────────────────────

// ── Direct-mode reshapers (`direct_authorize` / `direct_execute` / ─
//   `direct_list_connections`)
//
// These helpers wrap a `ComposioTool` and reshape v3 responses into
// the backend-proxied envelope types. We can't easily mock the live
// `backend.composio.dev` endpoints in this unit-test layer (the
// `ComposioTool` builds its own `reqwest::Client`), so the assertions
// below verify the empty/invalid-input paths that don't require HTTP:
//
//   * `direct_authorize` rejects an empty toolkit before any network
//     hit, with an explicit error so the caller can surface it as a
//     400-class user error.
//   * `direct_execute` accepts a None-arguments call and falls
//     through to the underlying tool surface (which then errors on the
//     network call — covered by the integration test in `ops_tests.rs`).
//   * `direct_list_connections` is a thin mapper; the real coverage
//     for its row → ComposioConnection translation lives in the
//     `connected_account_*` tests in `composio_tests.rs`.

fn direct_tool_for_test() -> std::sync::Arc<crate::openhuman::tools::ComposioTool> {
    std::sync::Arc::new(crate::openhuman::tools::ComposioTool::new(
        "ck_test_direct",
        Some("default"),
        std::sync::Arc::new(crate::openhuman::security::SecurityPolicy::default()),
    ))
}

#[tokio::test]
async fn direct_authorize_rejects_empty_toolkit() {
    let tool = direct_tool_for_test();
    let err = super::direct_authorize(&tool, "   ", "default")
        .await
        .err()
        .expect("empty toolkit must error before any HTTP call");
    assert!(
        err.to_string().contains("toolkit must not be empty"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn pricing_for_config_short_circuits_in_direct_mode() {
    // Build a client pointed at an unreachable backend — if the
    // short-circuit fires, we never actually attempt the network call
    // and the empty default struct comes back immediately.
    let client = crate::openhuman::integrations::IntegrationClient::new(
        "http://127.0.0.1:0".into(),
        "test".into(),
    );
    let mut config = crate::openhuman::config::Config::default();
    config.composio.mode = "direct".into();

    let pricing = crate::openhuman::integrations::pricing_for_config(&client, &config).await;
    // The default struct has every per-integration entry as `None`.
    assert!(pricing.integrations.apify.is_none());
    assert!(pricing.integrations.twilio.is_none());
    assert!(pricing.integrations.google_places.is_none());
    assert!(pricing.integrations.parallel.is_none());
}
