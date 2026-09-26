use super::*;
use serde_json::json;
// ── Router-not-initialized fallback paths ─────────────────────
// These tests run without a global SocketManager so the router
// accessor returns an error and the ops fall back gracefully.

#[tokio::test]
async fn list_registrations_returns_empty_when_router_not_initialized() {
    // No global socket manager → graceful empty response.
    let out = list_registrations().await.unwrap();
    assert!(out.value.registrations.is_empty());
    assert!(out.logs.iter().any(|l| l.contains("returned 0")));
}

#[tokio::test]
async fn list_logs_returns_empty_when_router_not_initialized() {
    let out = list_logs(Some(50)).await.unwrap();
    assert!(out.value.logs.is_empty());
    assert!(out.logs.iter().any(|l| l.contains("returned 0")));

    let out2 = list_logs(None).await.unwrap();
    assert!(out2.value.logs.is_empty());
}

#[tokio::test]
async fn clear_logs_reports_zero_when_router_not_initialized() {
    let out = clear_logs().await.unwrap();
    assert_eq!(out.value.cleared, 0);
    assert!(out.logs.iter().any(|l| l.contains("removed 0")));
}

#[tokio::test]
async fn register_echo_errors_when_router_not_initialized() {
    // Without the router, register_echo must return an Err.
    let err = register_echo("uuid-1", Some("name".into()), Some("btid-1".into()))
        .await
        .unwrap_err();
    assert!(
        err.contains("socket manager not initialized")
            || err.contains("webhook router not initialized")
            || err.contains("register_echo failed"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn unregister_echo_errors_when_router_not_initialized() {
    let err = unregister_echo("uuid-1").await.unwrap_err();
    assert!(
        err.contains("socket manager not initialized")
            || err.contains("webhook router not initialized")
            || err.contains("unregister_echo failed"),
        "unexpected error: {err}"
    );
}

// ── build_echo_response ───────────────────────────────────────

#[test]
fn build_echo_response_encodes_request_fields_and_sets_headers() {
    let mut query = std::collections::HashMap::new();
    query.insert("q".to_string(), "1".to_string());
    let mut headers = std::collections::HashMap::new();
    headers.insert("X-Foo".to_string(), json!("bar"));
    let req = WebhookRequest {
        correlation_id: "c-1".into(),
        tunnel_id: "tid-1".into(),
        tunnel_uuid: "uuid-1".into(),
        tunnel_name: "hook".into(),
        method: "POST".into(),
        path: "/p".into(),
        headers,
        query,
        body: "cGF5bG9hZA==".into(), // base64 of "payload"
    };
    let resp = build_echo_response(&req);

    assert_eq!(resp.correlation_id, "c-1");
    assert_eq!(resp.status_code, 200);
    assert_eq!(
        resp.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        resp.headers
            .get("x-openhuman-webhook-target")
            .map(String::as_str),
        Some("echo")
    );
    // Decode the body and check the echoed fields survived the round-trip.
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(resp.body.as_bytes())
        .expect("base64 body");
    let v: serde_json::Value = serde_json::from_slice(&decoded).expect("json body");
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["echo"]["correlationId"], json!("c-1"));
    assert_eq!(v["echo"]["method"], json!("POST"));
    assert_eq!(v["echo"]["path"], json!("/p"));
    assert_eq!(v["echo"]["bodyBase64"], json!("cGF5bG9hZA=="));
}

// ── Validation on trimmed inputs ──────────────────────────────

// ── Authed HTTP round-trips via a mock backend ───────────────
