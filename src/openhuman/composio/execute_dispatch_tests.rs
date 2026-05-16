use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{routing::post, Json, Router};
use serde_json::json;

use super::execute_composio_action;
use crate::openhuman::composio::client::ComposioClient;
use crate::openhuman::integrations::IntegrationClient;

async fn start_mock_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // The spawned task is intentionally orphaned; it dies with the tokio
    // runtime when the test finishes. The previous oneshot-based graceful
    // shutdown was broken because the sender was dropped immediately,
    // signalling shutdown before the test could exercise the server.
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

fn build_client(base: &str) -> ComposioClient {
    let inner = Arc::new(IntegrationClient::new(
        base.to_string(),
        "test-token".to_string(),
    ));
    ComposioClient::new(inner)
}

#[tokio::test]
async fn local_validation_skips_network() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post({
            let attempts = attempts.clone();
            move || async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                Json(json!({"success": true, "data": {"successful": true, "data": {}, "costUsd": 0.0}}))
            }
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client(&base);
    let err = execute_composio_action(
        &client,
        "GMAIL_SEND_EMAIL",
        Some(json!({ "subject": "hello" })),
    )
    .await
    .unwrap_err();
    assert!(err.contains("[composio:error:"));
    assert_eq!(attempts.load(Ordering::SeqCst), 0);
}
