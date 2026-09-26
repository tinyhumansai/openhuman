use super::*;
use crate::hosted::test_support;
use tempfile::TempDir;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ID: &str = "0123456789abcdef01234567";

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"success": true, "data": data}))
}

#[tokio::test]
async fn validation_runs_before_the_credential() {
    let tmp = TempDir::new().unwrap();
    let config = test_support::config(&tmp, "http://127.0.0.1:9");
    assert_eq!(
        oauth_connect(&config, " / ", None, None, None)
            .await
            .unwrap_err(),
        "provider is required"
    );
    assert_eq!(
        oauth_fetch_integration_tokens(&config, "short", "k")
            .await
            .unwrap_err(),
        "integrationId must be a 24-char hex id"
    );
    assert_eq!(
        oauth_revoke_integration(&config, "  ").await.unwrap_err(),
        "integration id is required"
    );
    assert!(oauth_list_integrations(&config)
        .await
        .unwrap_err()
        .contains("no backend session token"));
}

#[tokio::test]
async fn local_session_is_backend_unavailable() {
    let tmp = TempDir::new().unwrap();
    let config = test_support::local_session(&tmp);
    let err = oauth_connect(&config, "github", None, None, None)
        .await
        .unwrap_err();
    assert!(err.starts_with("BACKEND_UNAVAILABLE:"), "{err}");
}

#[tokio::test]
async fn connect_forwards_non_empty_query_and_shapes_the_result() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/github/connect"))
        .and(query_param("skillId", "s1"))
        .and(query_param("encryptionMode", "encrypted"))
        .respond_with(ok(
            json!({"oauthUrl": "https://gh/authorize", "state": "st"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let tmp = TempDir::new().unwrap();
    let config = test_support::signed_in(&tmp, &server.uri());
    let out = oauth_connect(&config, "/github/", Some("s1"), Some(""), Some("encrypted"))
        .await
        .unwrap();
    assert_eq!(
        out.value,
        json!({"oauthUrl": "https://gh/authorize", "state": "st"})
    );
}

#[tokio::test]
async fn connect_rejects_a_response_without_url_or_state() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/notion/connect"))
        .respond_with(ok(json!({"oauthUrl": "https://n"})))
        .mount(&server)
        .await;
    let tmp = TempDir::new().unwrap();
    let config = test_support::signed_in(&tmp, &server.uri());
    let err = oauth_connect(&config, "notion", None, None, None)
        .await
        .unwrap_err();
    assert!(err.contains("missing state"), "{err}");
}

#[tokio::test]
async fn list_revoke_and_token_handoff_round_trip() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/integrations"))
        .respond_with(ok(json!({"integrations": [
            {"id": ID, "provider": "google", "createdAt": "2026-01-01T00:00:00Z", "extra": 1}
        ]})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("/auth/integrations/{ID}")))
        .respond_with(ok(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/auth/integrations/{ID}/tokens")))
        .and(body_json(json!({"key": "enc-key"})))
        .respond_with(ok(json!({"encrypted": "not-base64!"})))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = test_support::signed_in(&tmp, &server.uri());

    let out = oauth_list_integrations(&config).await.unwrap();
    assert_eq!(
        out.value,
        json!([{"id": ID, "provider": "google", "createdAt": "2026-01-01T00:00:00Z"}])
    );

    let out = oauth_revoke_integration(&config, ID).await.unwrap();
    assert_eq!(out.value, json!({"revoked": true, "integrationId": ID}));

    let err = oauth_fetch_integration_tokens(&config, ID, " enc-key ")
        .await
        .unwrap_err();
    assert!(err.starts_with("integration tokens handoff:"), "{err}");
}

#[tokio::test]
async fn token_handoff_without_encrypted_payload_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/auth/integrations/{ID}/tokens")))
        .respond_with(ok(json!({})))
        .mount(&server)
        .await;
    let tmp = TempDir::new().unwrap();
    let config = test_support::signed_in(&tmp, &server.uri());
    let err = oauth_fetch_integration_tokens(&config, ID, "k")
        .await
        .unwrap_err();
    assert_eq!(err, "integration tokens response missing encrypted payload");
}
