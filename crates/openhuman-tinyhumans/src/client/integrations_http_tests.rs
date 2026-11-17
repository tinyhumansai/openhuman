use super::*;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn session(token: &str) -> BackendCredential {
    BackendCredential::Session(token.to_string())
}

#[test]
fn sanitize_strips_inference_paths_and_is_idempotent() {
    let cleaned = sanitize_backend_url("https://api.tinyhumans.ai/openai/v1/chat/completions");
    assert_eq!(cleaned, "https://api.tinyhumans.ai");
    assert_eq!(sanitize_backend_url(&cleaned), cleaned);
    assert_eq!(
        sanitize_backend_url("https://api.tinyhumans.ai/"),
        "https://api.tinyhumans.ai"
    );
}

#[test]
fn privileged_paths_are_refused_before_sending() {
    for route in ["/webhooks/x", "/agent-integrations/admin/y", "/a/ADMIN?q=1"] {
        assert!(
            reject_privileged_backend_path("GET", route).is_err(),
            "{route}"
        );
    }
    reject_privileged_backend_path("GET", "/agent-integrations/composio/connections").unwrap();
}

#[test]
fn credential_headers_put_session_on_bearer_and_key_on_x_api_key() {
    let headers = credential_headers(&session(" jwt ")).unwrap();
    assert_eq!(headers[AUTHORIZATION], "Bearer jwt");
    assert!(headers.get(API_KEY_HEADER).is_none());

    let headers = credential_headers(&BackendCredential::ApiKey("th_k".into())).unwrap();
    assert_eq!(headers[API_KEY_HEADER], "th_k");
    assert!(headers.get(AUTHORIZATION).is_none());
}

#[test]
fn credential_guard_refuses_a_key_to_a_foreign_host() {
    let http = IntegrationsHttp::new("https://example.com", BackendCredential::ApiKey("k".into()));
    assert!(http.uses_api_key());
    assert!(http
        .validate_credential_endpoint()
        .unwrap_err()
        .to_string()
        .contains("TinyHumans API key requires"));
    assert!(http.auth_headers().is_err());
}

#[tokio::test]
async fn send_json_uses_the_integrations_profile_and_returns_the_raw_envelope() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/agent-integrations/pricing"))
        .and(header("authorization", "Bearer jwt"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"success": true, "data": {"n": 1}})),
        )
        .mount(&server)
        .await;

    let http = IntegrationsHttp::new(&server.uri(), session("jwt"));
    let value = http
        .send_json(Method::GET, "/agent-integrations/pricing", None)
        .await
        .unwrap();
    // unwrap_envelope is false: the integrations domain parses it itself.
    assert_eq!(value, json!({"success": true, "data": {"n": 1}}));
}

#[tokio::test]
async fn send_json_surfaces_a_non_2xx_as_the_raw_status_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/x"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "Invalid token"})))
        .mount(&server)
        .await;

    let http = IntegrationsHttp::new(&server.uri(), session("jwt"));
    let err = http
        .send_json(Method::POST, "/agent-integrations/x", Some(&json!({})))
        .await
        .unwrap_err();
    assert!(
        matches!(err, BackendTransportError::Status { status: 401, .. }),
        "{err:?}"
    );
}

#[tokio::test]
async fn send_json_refuses_privileged_routes_without_a_request() {
    let server = MockServer::start().await;
    let http = IntegrationsHttp::new(&server.uri(), session("jwt"));
    let err = http
        .send_json(Method::GET, "/webhooks/abc", None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not exposed"), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn send_multipart_posts_a_form() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/upload"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"success": true, "data": {"ok": true}})),
        )
        .mount(&server)
        .await;

    let http = IntegrationsHttp::new(&server.uri(), session("jwt"));
    let form = reqwest::multipart::Form::new().text("k", "v");
    let value = http
        .send_multipart("/agent-integrations/upload", form)
        .await
        .unwrap();
    assert_eq!(value["ok"], json!(true));
    let received = server.received_requests().await.unwrap();
    let content_type = received[0]
        .headers
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        content_type.starts_with("multipart/form-data"),
        "{content_type}"
    );
}
