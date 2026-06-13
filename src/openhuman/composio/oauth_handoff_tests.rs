//! Tests for Meta OAuth handoff helpers (#1952).

use super::{
    authorize_with_meta_guard, is_authorize_rate_limited, is_clearable_oauth_status,
    is_inflight_oauth_status, is_meta_oauth_toolkit, meta_oauth_rate_limit_message,
    wrap_authorize_rate_limit_error,
};
use axum::{
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::Arc;

use super::ComposioClient;

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

#[test]
fn meta_oauth_toolkit_detection() {
    assert!(is_meta_oauth_toolkit("instagram"));
    assert!(is_meta_oauth_toolkit("Facebook"));
    assert!(!is_meta_oauth_toolkit("gmail"));
}

#[test]
fn inflight_and_clearable_statuses() {
    assert!(is_inflight_oauth_status("pending"));
    assert!(is_inflight_oauth_status("INITIATED"));
    assert!(!is_inflight_oauth_status("ACTIVE"));

    assert!(is_clearable_oauth_status("FAILED"));
    assert!(is_clearable_oauth_status("EXPIRED"));
    assert!(!is_clearable_oauth_status("ACTIVE"));
}

#[test]
fn authorize_rate_limit_shape_detection() {
    assert!(is_authorize_rate_limited(
        "Backend returned 429 Too Many Requests"
    ));
    assert!(is_authorize_rate_limited("rate_limit exceeded"));
    assert!(!is_authorize_rate_limited("401 Unauthorized"));
}

#[test]
fn wrap_authorize_rate_limit_error_replaces_meta_toolkit_message() {
    let err = anyhow::anyhow!("Backend returned 429 Too Many Requests");
    let wrapped = wrap_authorize_rate_limit_error("instagram", err);
    let msg = format!("{wrapped:#}");
    assert!(msg.contains("Business or Creator"));
    assert!(msg.contains("429"));
}

#[test]
fn wrap_authorize_rate_limit_error_passthrough_for_non_meta() {
    let err = anyhow::anyhow!("Backend returned 429 Too Many Requests");
    let wrapped = wrap_authorize_rate_limit_error("gmail", err);
    assert!(format!("{wrapped:#}").contains("Backend returned 429"));
}

#[test]
fn meta_oauth_rate_limit_message_mentions_business_account() {
    let msg = meta_oauth_rate_limit_message("instagram");
    assert!(msg.to_ascii_lowercase().contains("business"));
}

#[test]
fn meta_oauth_rate_limit_message_uses_facebook_specific_guidance() {
    let msg = meta_oauth_rate_limit_message("facebook");
    assert!(msg.contains("Facebook"));
    assert!(msg.contains("Business Manager"));
    assert!(!msg.contains("Instagram Business or Creator"));
}

#[tokio::test]
async fn authorize_continues_when_pre_handoff_cleanup_fails() {
    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "success": false,
                        "error": "temporary list failure"
                    })),
                )
            }),
        )
        .route(
            "/agent-integrations/composio/authorize",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["toolkit"].as_str(), Some("instagram"));
                Json(json!({
                    "success": true,
                    "data": {
                        "connectUrl": "https://composio.example/instagram/consent",
                        "connectionId": "conn-instagram"
                    }
                }))
            }),
        );
    let client = build_client_for(start_mock_backend(app).await);

    let resp = authorize_with_meta_guard(&client, "instagram", None)
        .await
        .expect("authorize should continue when cleanup is unavailable");

    assert_eq!(resp.connection_id, "conn-instagram");
    assert!(resp.connect_url.contains("instagram"));
}
