use super::*;
use openhuman_core::security::credentials::session_support::BackendCredential;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn only_a_404_status_counts_as_not_found() {
    let not_found: Result<(), SdkError> = Err(SdkError::Status {
        status: 404,
        body: Value::Null,
    });
    assert!(is_not_found(&not_found));

    let unauthorized: Result<(), SdkError> = Err(SdkError::Status {
        status: 401,
        body: Value::Null,
    });
    assert!(!is_not_found(&unauthorized));
    assert!(!is_not_found(&Ok::<(), SdkError>(())));
}

async fn latest_via(server: &MockServer) -> Result<Option<Value>, String> {
    let client =
        HostedClient::with_credential(&server.uri(), BackendCredential::Session("jwt".into()));
    let result = client
        .sdk()
        .announcements()
        .get_latest_announcements()
        .await;
    if is_not_found(&result) {
        return Ok(None);
    }
    client
        .finish("GET /announcements/latest", result)
        .map(|a| Some(serde_json::to_value(a).unwrap()))
}

#[tokio::test]
async fn announcement_payload_keeps_its_wire_shape() {
    let server = MockServer::start().await;
    let announcement = json!({
        "id": "a1",
        "title": "Hello",
        "body": "World",
        "severity": "INFO",
        "cta": {"label": "Open", "url": "https://example.com"},
        "startsAt": null,
        "expiresAt": null,
        "createdAt": "2026-01-01T00:00:00.000Z"
    });
    Mock::given(method("GET"))
        .and(path("/announcements/latest"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"success": true, "data": announcement.clone()})),
        )
        .mount(&server)
        .await;
    assert_eq!(latest_via(&server).await.unwrap(), Some(announcement));
}

#[tokio::test]
async fn backend_404_is_no_announcement_and_null_passes_through() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/announcements/latest"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;
    assert_eq!(latest_via(&server).await.unwrap(), None);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/announcements/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"success": true, "data": null})),
        )
        .mount(&server)
        .await;
    assert_eq!(latest_via(&server).await.unwrap(), Some(Value::Null));
}
