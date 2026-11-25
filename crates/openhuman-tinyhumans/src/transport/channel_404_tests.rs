//! Channel-message 404 classification: the transport, not the core, decides
//! what a 404 on `/channels/<p>/messages/<id>` means on this backend (#5230,
//! OPENHUMAN-TAURI-2Y, TAURI-R7).

use super::*;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn send(
    base: &str,
    http_method: reqwest::Method,
    route: &str,
) -> Result<Value, BackendTransportError> {
    let credential = BackendCredential::Session("jwt".into());
    SdkBackendTransport::new()
        .unwrap()
        .send_json(BackendRequest {
            profile: TransportProfile::Api,
            base_url: base,
            method: http_method,
            path: route,
            query: &[],
            body: None,
            credential: Some(&credential),
            unwrap_envelope: true,
        })
        .await
}

async fn server_404(http_method: &str, route: &str, body: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method(http_method))
        .and(path(route))
        .respond_with(body)
        .mount(&server)
        .await;
    server
}

fn html_404() -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_string("<pre>Cannot PATCH /channels</pre>")
}

fn json_404() -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_json(json!({"success": false, "error": "gone"}))
}

#[tokio::test]
async fn patch_404_without_a_matching_route_is_route_missing() {
    let server = server_404("PATCH", "/channels/telegram/messages/1103", html_404()).await;
    let err = send(
        &server.uri(),
        Method::PATCH,
        "/channels/telegram/messages/1103",
    )
    .await
    .unwrap_err();
    let BackendTransportError::ChannelMessageRouteMissing {
        provider,
        message_id,
    } = err
    else {
        panic!("expected ChannelMessageRouteMissing, got {err:?}");
    };
    assert_eq!(
        (provider.as_str(), message_id.as_str()),
        ("telegram", "1103")
    );
}

#[tokio::test]
async fn patch_404_with_a_non_json_body_is_route_missing() {
    // A CDN or proxy HTML 404 has no "Cannot PATCH", but no backend handler
    // answered it either: the SDK's classifier treats any non-JSON body as an
    // unmatched route, and the host follows it.
    let server = server_404(
        "PATCH",
        "/channels/telegram/messages/5",
        ResponseTemplate::new(404).set_body_string("<html>Not Found</html>"),
    )
    .await;
    let err = send(
        &server.uri(),
        Method::PATCH,
        "/channels/telegram/messages/5",
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            err,
            BackendTransportError::ChannelMessageRouteMissing { .. }
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn patch_404_from_a_real_handler_is_a_missing_message() {
    // The world where the edit route exists: a handler's JSON 404 means the
    // message is gone, never that edits are unsupported for the provider.
    let server = server_404("PATCH", "/channels/discord/messages/abc", json_404()).await;
    let err = send(
        &server.uri(),
        Method::PATCH,
        "/channels/discord/messages/abc",
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            &err,
            BackendTransportError::ChannelMessageNotFound { provider, message_id }
                if provider == "discord" && message_id == "abc"
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn delete_and_post_404_are_missing_messages_whatever_the_body() {
    for (verb, method_) in [("DELETE", Method::DELETE), ("POST", Method::POST)] {
        let server = server_404(verb, "/channels/telegram/messages/9", html_404()).await;
        let err = send(&server.uri(), method_, "/channels/telegram/messages/9")
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackendTransportError::ChannelMessageNotFound { .. }),
            "{verb}: got {err:?}"
        );
    }
}

#[tokio::test]
async fn prefixed_channel_path_keeps_the_parsed_ids() {
    let server = server_404("PATCH", "/api/v1/channels/telegram/messages/77", html_404()).await;
    let err = send(
        &server.uri(),
        Method::PATCH,
        "/api/v1/channels/telegram/messages/77",
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            &err,
            BackendTransportError::ChannelMessageRouteMissing { provider, message_id }
                if provider == "telegram" && message_id == "77"
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn non_channel_404_stays_a_plain_status() {
    let server = server_404("GET", "/teams/me/usage", html_404()).await;
    let err = send(&server.uri(), Method::GET, "/teams/me/usage")
        .await
        .unwrap_err();
    assert_eq!(err.status(), Some(404));
    assert!(
        matches!(err, BackendTransportError::Status { .. }),
        "got {err:?}"
    );
}

#[test]
fn map_sdk_error_leaves_non_404_channel_errors_alone() {
    let err = map_sdk_error(
        tinyhumans_sdk::Error::Status {
            status: 401,
            body: Value::String("Unauthorized".into()),
        },
        &Method::POST,
        "/channels/telegram/messages/1",
    );
    assert!(matches!(
        err,
        BackendTransportError::Status { status: 401, .. }
    ));
}
