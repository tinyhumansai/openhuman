//! `IntegrationClient` authenticated with a TinyHumans API key instead of a
//! session JWT: the key rides `x-api-key` on every route (JSON, download,
//! raw DELETE), and a 401 is an API-key rejection, never a session expiry.

use super::*;
use axum::http::HeaderMap;
use std::sync::Mutex;

use openhuman_embed::__host::security::credentials::session_support::BackendCredential;

const KEY: &str = "tiny_test_integrations_key";

fn api_key_client(base: String) -> IntegrationClient {
    IntegrationClient::new_with_credential(base, BackendCredential::ApiKey(KEY.into()))
}

/// `(x-api-key, authorization)` seen by the mock for one request.
type Seen = Arc<Mutex<Vec<(Option<String>, Option<String>)>>>;

fn record(seen: &Seen, headers: &HeaderMap) {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    seen.lock()
        .unwrap()
        .push((header("x-api-key"), header("authorization")));
}

#[tokio::test]
async fn json_requests_send_the_api_key_as_x_api_key_and_no_bearer() {
    let seen: Seen = Arc::default();
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get({
            let seen = Arc::clone(&seen);
            move |headers: HeaderMap| async move {
                record(&seen, &headers);
                Json(json!({ "success": true, "data": { "connections": [] } }))
            }
        }),
    );
    let client = api_key_client(start_mock_backend(app).await);
    assert!(client.uses_api_key());
    client
        .get::<serde_json::Value>(
            &Default::default(),
            "/agent-integrations/composio/connections",
        )
        .await
        .expect("an API-key client reaches composio");

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0.as_deref(), Some(KEY));
    assert_eq!(seen[0].1, None, "an API key must never ride Authorization");
}

#[tokio::test]
async fn binary_download_sends_the_api_key_as_x_api_key() {
    let seen: Seen = Arc::default();
    let app = Router::new().route(
        "/agent-integrations/file-storage/files/f1/download",
        get({
            let seen = Arc::clone(&seen);
            move |headers: HeaderMap| async move {
                record(&seen, &headers);
                "bytes"
            }
        }),
    );
    let client = api_key_client(start_mock_backend(app).await);
    let (bytes, _, _) = client
        .get_bytes(
            &Default::default(),
            "/agent-integrations/file-storage/files/f1/download",
        )
        .await
        .expect("download with an API key");
    assert_eq!(&bytes[..], b"bytes");

    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].0.as_deref(), Some(KEY));
    assert_eq!(seen[0].1, None);
}

#[tokio::test]
async fn presigned_download_redirect_does_not_forward_api_key() {
    let seen: Seen = Arc::default();
    let storage = Router::new().route(
        "/blob",
        get({
            let seen = Arc::clone(&seen);
            move |headers: HeaderMap| async move {
                record(&seen, &headers);
                "stored bytes"
            }
        }),
    );
    let storage_url = start_mock_backend(storage).await;
    let location = format!("{storage_url}/blob");
    let backend = Router::new().route(
        "/agent-integrations/file-storage/files/f1/download",
        get(move || {
            let location = location.clone();
            async move {
                (
                    StatusCode::FOUND,
                    [(axum::http::header::LOCATION, location)],
                )
            }
        }),
    );
    let client = api_key_client(start_mock_backend(backend).await);
    let (bytes, _, _) = client
        .get_bytes(
            &Default::default(),
            "/agent-integrations/file-storage/files/f1/download",
        )
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"stored bytes");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0], (None, None));
}

#[tokio::test]
async fn api_key_401_is_api_key_rejected_not_session_expired() {
    use openhuman_embed::__host::core::observability::is_session_expired_message;

    let app = Router::new().route(
        "/agent-integrations/parallel/search",
        post(|| async {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "success": false, "error": "Invalid API key" })),
            )
                .into_response()
        }),
    );
    let client = api_key_client(start_mock_backend(app).await);
    let err = client
        .post::<serde_json::Value>(
            &Default::default(),
            "/agent-integrations/parallel/search",
            &json!({ "objective": "x" }),
        )
        .await
        .expect_err("401 must surface as Err");
    let msg = format!("{err:#}");
    assert!(msg.contains("API_KEY_REJECTED"), "{msg}");
    assert!(!msg.contains("SESSION_EXPIRED"), "{msg}");
    assert!(
        !is_session_expired_message(&msg),
        "a rejected API key must not drive the session re-login flow: {msg}"
    );
}
