use super::*;
use crate::hosted::test_support;
use openhuman_core::core::observability::expected_error_kind;
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"success": true, "data": data}))
}

// ── validation runs before the credential ─────────────────────

#[tokio::test]
async fn create_tunnel_rejects_empty_or_whitespace_name() {
    let tmp = TempDir::new().unwrap();
    let config = test_support::config(&tmp, "http://127.0.0.1:9");
    for name in ["", "   "] {
        let err = create_tunnel(&config, name, None).await.unwrap_err();
        assert_eq!(err, "name is required");
    }
}

#[tokio::test]
async fn id_bearing_tunnel_ops_reject_empty_or_whitespace_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_support::config(&tmp, "http://127.0.0.1:9");
    for id in ["", "   "] {
        assert_eq!(get_tunnel(&config, id).await.unwrap_err(), "id is required");
        assert_eq!(
            delete_tunnel(&config, id).await.unwrap_err(),
            "id is required"
        );
        assert_eq!(
            update_tunnel(&config, id, UpdateWebhookTunnelRequest::default())
                .await
                .unwrap_err(),
            "id is required"
        );
    }
}

#[tokio::test]
async fn calls_without_a_session_fail_before_any_request() {
    let tmp = TempDir::new().unwrap();
    let config = test_support::config(&tmp, "http://127.0.0.1:9");
    assert!(list_tunnels(&config)
        .await
        .unwrap_err()
        .contains("no backend session token"));
    assert!(get_bandwidth(&config)
        .await
        .unwrap_err()
        .contains("no backend session token"));
}

#[tokio::test]
async fn local_session_is_backend_unavailable() {
    let tmp = TempDir::new().unwrap();
    let config = test_support::local_session(&tmp);
    let err = list_tunnels(&config).await.unwrap_err();
    assert!(err.starts_with("BACKEND_UNAVAILABLE:"), "{err}");
    assert!(expected_error_kind(&err).is_some());
}

// ── SDK round-trips against a mock backend ────────────────────

#[tokio::test]
async fn tunnel_crud_round_trips_through_the_sdk() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/webhooks/core"))
        .and(header("authorization", "Bearer jwt.test"))
        .respond_with(ok(json!({"tunnels": [{"id": "t-1"}]})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webhooks/core"))
        .and(body_json(json!({"name": "my-hook", "description": "desc"})))
        .respond_with(ok(json!({"id": "t-2"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webhooks/core"))
        .and(body_json(json!({"name": "nodesc"})))
        .respond_with(ok(json!({"id": "t-3"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/webhooks/core/a%3Ab"))
        .respond_with(ok(json!({"id": "a:b"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/webhooks/core/t-1"))
        .and(body_json(json!({"name": "renamed", "isActive": true})))
        .respond_with(ok(json!({"id": "t-1", "name": "renamed"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/webhooks/core/t-42"))
        .respond_with(ok(json!({"deleted": "t-42"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/webhooks/core/bandwidth"))
        .respond_with(ok(json!({"remaining": 1024})))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = test_support::signed_in(&tmp, &server.uri());

    let out = list_tunnels(&config).await.unwrap();
    assert_eq!(out.value["tunnels"][0]["id"], json!("t-1"));
    assert!(out
        .logs
        .iter()
        .any(|l| l.contains("webhook tunnels fetched")));

    let out = create_tunnel(&config, "  my-hook  ", Some("  desc  ".into()))
        .await
        .unwrap();
    assert_eq!(out.value["id"], json!("t-2"));
    create_tunnel(&config, "nodesc", Some("   ".into()))
        .await
        .unwrap();

    let out = get_tunnel(&config, "  a:b  ").await.unwrap();
    assert_eq!(out.value["id"], json!("a:b"));

    let request = UpdateWebhookTunnelRequest {
        name: Some("renamed".into()),
        description: None,
        is_active: Some(true),
    };
    let out = update_tunnel(&config, "t-1", request).await.unwrap();
    assert_eq!(out.value["name"], json!("renamed"));

    let out = delete_tunnel(&config, "t-42").await.unwrap();
    assert_eq!(out.value["deleted"], json!("t-42"));

    let out = get_bandwidth(&config).await.unwrap();
    assert_eq!(out.value["remaining"], json!(1024));
}
