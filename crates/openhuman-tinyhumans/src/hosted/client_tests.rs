use super::*;
use openhuman_core::core::observability::{
    expected_error_kind, is_api_key_rejected_message, is_session_expired_message,
};
use openhuman_core::security::credentials::{AuthService, APP_SESSION_PROVIDER};
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_config(tmp: &TempDir, api_url: &str) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        api_url: Some(api_url.to_string()),
        ..Config::default()
    }
}

fn store_session(config: &Config, token: &str) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            "default",
            token,
            std::collections::HashMap::new(),
            true,
        )
        .expect("store session token");
}

#[test]
fn backend_origin_strips_path_query_and_fragment() {
    assert_eq!(
        backend_origin("https://api.example.com/v1/chat/completions?x=1#f").unwrap(),
        "https://api.example.com"
    );
    assert_eq!(
        backend_origin("http://127.0.0.1:8080/").unwrap(),
        "http://127.0.0.1:8080"
    );
    assert!(backend_origin("not a url").is_err());
    assert!(backend_origin("ftp://example.com").is_err());
}

#[test]
fn session_401_maps_to_session_expired_sentinel() {
    let err = map_error(
        CredentialKind::Session,
        "GET /teams/me/usage",
        SdkError::Status {
            status: 401,
            body: json!({"error": "Invalid token"}),
        },
    );
    assert!(err.starts_with("SESSION_EXPIRED:"), "{err}");
    assert!(is_session_expired_message(&err));
}

#[test]
fn api_key_401_maps_to_api_key_rejected_not_session_expiry() {
    let err = map_error(
        CredentialKind::ApiKey,
        "GET /teams/me/usage",
        SdkError::Status {
            status: 401,
            body: Value::Null,
        },
    );
    assert!(err.starts_with(API_KEY_REJECTED_PREFIX), "{err}");
    assert!(is_api_key_rejected_message(&err));
    assert!(
        !err.contains("SESSION_EXPIRED"),
        "a rejected api key must not trigger session sign-out: {err}"
    );
    assert!(expected_error_kind(&err).is_some());
}

#[test]
fn other_status_keeps_the_authed_json_error_shape() {
    let err = map_error(
        CredentialKind::Session,
        "GET /payments/summary",
        SdkError::Status {
            status: 503,
            body: Value::String("upstream down".into()),
        },
    );
    assert_eq!(
        err,
        "GET /payments/summary failed (503 Service Unavailable): upstream down"
    );

    let err = map_error(
        CredentialKind::Session,
        "POST /coupons/redeem",
        SdkError::Status {
            status: 400,
            body: json!({"success": false, "error": "bad code"}),
        },
    );
    assert!(err.starts_with("POST /coupons/redeem failed (400 Bad Request): "));
    assert!(err.contains("bad code"));
}

#[test]
fn envelope_and_route_errors_keep_context() {
    let err = map_error(
        CredentialKind::Session,
        "GET /referral/stats",
        SdkError::Envelope {
            error: "nope".into(),
            error_code: None,
            details: Value::Null,
        },
    );
    assert!(err.starts_with("backend request GET /referral/stats: "));
    assert!(err.contains("nope"));

    let err = map_error(
        CredentialKind::Session,
        "POST /admin/x",
        SdkError::RouteNotExposed("POST".into(), "/admin/x".into()),
    );
    assert!(err.contains("not exposed"), "{err}");
}

#[test]
fn offline_local_session_fails_before_any_request() {
    let tmp = TempDir::new().unwrap();
    // Unroutable: a request here would fail loudly, not return the sentinel.
    let config = test_config(&tmp, "http://127.0.0.1:9");
    store_session(&config, "desktop.test.local");
    let err = HostedClient::from_config(&config)
        .err()
        .expect("local refuses");
    assert_eq!(
        err,
        openhuman_core::security::credentials::session_support::LOCAL_SESSION_BACKEND_UNAVAILABLE
    );
    assert!(expected_error_kind(&err).is_some());
}

#[test]
fn missing_credential_fails_before_any_request() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, "http://127.0.0.1:9");
    assert!(HostedClient::from_config(&config).is_err());
}

#[tokio::test]
async fn session_client_sends_bearer_and_product_identity() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/teams/me/usage"))
        .and(header("authorization", "Bearer jwt.a.b"))
        .and(header("x-sdk-name", "openhuman"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {"remainingUsd": 2.0}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, &server.uri());
    store_session(&config, "jwt.a.b");
    let client = HostedClient::from_config(&config).unwrap();
    assert_eq!(client.kind(), CredentialKind::Session);
    let value = client
        .finish_value(
            "GET /teams/me/usage",
            client.sdk().teams().get_my_usage().await,
        )
        .unwrap();
    assert_eq!(value, json!({"remainingUsd": 2.0}));
}

#[tokio::test]
async fn api_key_client_sends_x_api_key_and_maps_401() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/teams/me/usage"))
        .and(header("x-api-key", "th_key"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "bad key"})))
        .expect(1)
        .mount(&server)
        .await;

    let client =
        HostedClient::with_credential(&server.uri(), BackendCredential::ApiKey("th_key".into()));
    let err = client
        .finish_value(
            "GET /teams/me/usage",
            client.sdk().teams().get_my_usage().await,
        )
        .unwrap_err();
    assert!(is_api_key_rejected_message(&err), "{err}");
}
