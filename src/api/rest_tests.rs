use super::{key_bytes_from_string, sanitize_client_version, BackendOAuthClient};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[test]
fn decodes_base64url_no_pad() {
    // A 32-byte key that, when base64url-encoded, contains both `-` and `_`.
    let raw = [
        0xff_u8, 0xfb, 0xef, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa,
        0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
        0x0b, 0x0c, 0x0d,
    ];
    let url_key = URL_SAFE_NO_PAD.encode(raw);
    assert!(url_key.contains('-') || url_key.contains('_'));
    let decoded = key_bytes_from_string(&url_key).unwrap();
    assert_eq!(decoded, raw);
}

#[test]
fn decodes_standard_base64() {
    let raw = [0x41_u8; 32];
    let std_key = STANDARD.encode(raw);
    let decoded = key_bytes_from_string(&std_key).unwrap();
    assert_eq!(decoded, raw);
}

#[test]
fn decodes_raw_32_byte_key() {
    let raw = "abcdefghijklmnopqrstuvwxyz012345";
    assert_eq!(raw.len(), 32);
    let decoded = key_bytes_from_string(raw).unwrap();
    assert_eq!(decoded, raw.as_bytes());
}

#[test]
fn trims_whitespace() {
    let raw = [0x42_u8; 32];
    let url_key = format!("  {}\n", URL_SAFE_NO_PAD.encode(raw));
    let decoded = key_bytes_from_string(&url_key).unwrap();
    assert_eq!(decoded, raw);
}

#[test]
fn rejects_wrong_length() {
    let err = key_bytes_from_string("tooshort").unwrap_err();
    assert!(err.to_string().contains("must decode to 32 raw bytes"));
}

use super::user_id_from_profile_payload;

#[test]
fn extracts_id_from_root() {
    let payload1 = json!({ "id": "123" });
    let payload2 = json!({ "_id": "456" });
    let payload3 = json!({ "userId": "789" });

    assert_eq!(user_id_from_profile_payload(&payload1).unwrap(), "123");
    assert_eq!(user_id_from_profile_payload(&payload2).unwrap(), "456");
    assert_eq!(user_id_from_profile_payload(&payload3).unwrap(), "789");
}

#[test]
fn extracts_id_from_data_nested() {
    let payload = json!({
        "data": { "id": "abc" }
    });
    assert_eq!(user_id_from_profile_payload(&payload).unwrap(), "abc");
}

#[test]
fn extracts_id_from_user_nested() {
    let payload = json!({
        "user": { "id": "def" }
    });
    assert_eq!(user_id_from_profile_payload(&payload).unwrap(), "def");
}

#[test]
fn extracts_id_from_data_user_nested() {
    let payload = json!({
        "data": {
            "user": { "userId": "ghi" }
        }
    });
    assert_eq!(user_id_from_profile_payload(&payload).unwrap(), "ghi");
}

#[test]
fn ignores_whitespace_only_ids() {
    let payload = json!({
        "data": {
            "id": "   ",
            "_id": "real_id"
        }
    });
    assert_eq!(user_id_from_profile_payload(&payload).unwrap(), "real_id");
}

#[test]
fn trims_extracted_ids() {
    let payload = json!({
        "id": "  padded_id  "
    });
    assert_eq!(user_id_from_profile_payload(&payload).unwrap(), "padded_id");
}

#[test]
fn rejects_non_string_ids() {
    let payload = json!({
        "id": 123,
        "_id": ["not_a_string"],
        "userId": "valid_id"
    });
    assert_eq!(user_id_from_profile_payload(&payload).unwrap(), "valid_id");
}

#[test]
fn returns_none_for_missing_ids() {
    let payload = json!({
        "data": { "name": "alice" }
    });
    assert!(user_id_from_profile_payload(&payload).is_none());
}

#[test]
fn returns_none_for_non_object_payload() {
    let payload = json!("just a string");
    assert!(user_id_from_profile_payload(&payload).is_none());
}

#[test]
fn sanitize_client_version_strips_invalid_chars_and_clamps_length() {
    let raw = format!(" 1.2.3 (desktop)+build!?{} ", "a".repeat(80));
    let sanitized = sanitize_client_version(&raw).unwrap();
    assert_eq!(sanitized, format!("1.2.3desktop+build{}", "a".repeat(46)));
    assert_eq!(sanitized.len(), 64);
}

#[derive(Clone, Default)]
struct CapturedHeaders {
    entries: Arc<Mutex<Vec<HeaderMap>>>,
}

impl CapturedHeaders {
    fn push(&self, headers: &HeaderMap) {
        self.entries.lock().unwrap().push(headers.clone());
    }

    fn take(&self) -> Vec<HeaderMap> {
        self.entries.lock().unwrap().clone()
    }
}

async fn spawn_header_capture_server() -> (String, CapturedHeaders) {
    async fn capture_consume(
        State(captured): State<CapturedHeaders>,
        headers: HeaderMap,
    ) -> Json<Value> {
        captured.push(&headers);
        Json(json!({
            "success": true,
            "data": { "jwtToken": "mock-jwt-token" }
        }))
    }

    async fn capture_probe(
        State(captured): State<CapturedHeaders>,
        headers: HeaderMap,
    ) -> Json<Value> {
        captured.push(&headers);
        Json(json!({ "ok": true }))
    }

    let captured = CapturedHeaders::default();
    let app = Router::new()
        .route(
            "/telegram/login-tokens/{token}/consume",
            post(capture_consume),
        )
        .route("/probe", get(capture_probe))
        .with_state(captured.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (format!("http://{addr}"), captured)
}

#[tokio::test]
async fn backend_client_sends_x_core_version_on_auth_requests() {
    let (base_url, captured) = spawn_header_capture_server().await;
    let client = BackendOAuthClient::new(&base_url).unwrap();

    let jwt = client.consume_login_token("test-token").await.unwrap();
    assert_eq!(jwt, "mock-jwt-token");

    let headers = captured.take();
    let request_headers = headers.last().unwrap();
    let version = request_headers
        .get("x-core-version")
        .and_then(|value| value.to_str().ok())
        .unwrap();
    assert_eq!(
        version,
        sanitize_client_version(env!("CARGO_PKG_VERSION")).unwrap()
    );
}

// Regression: OPENHUMAN-TAURI-8K / Sentry issue 7473650958.
// When config.api_url is a full LLM completions URL (e.g. /v1/chat/completions),
// Url::join used to produce wrong paths like /v1/chat/teams/me/usage instead of
// /teams/me/usage — BackendOAuthClient::new must strip the path to prevent this.
#[test]
fn new_strips_path_from_completions_url() {
    let client = BackendOAuthClient::new("https://api.tinyhumans.ai/v1/chat/completions").unwrap();
    let url = client.url_for("/teams/me/usage").unwrap();
    assert_eq!(url.path(), "/teams/me/usage");
}

#[test]
fn new_strips_path_from_openai_style_url() {
    let client = BackendOAuthClient::new("https://api.openai.com/v1/chat/completions").unwrap();
    let url = client.url_for("/teams/me/usage").unwrap();
    assert_eq!(url.path(), "/teams/me/usage");
    assert_eq!(url.host_str(), Some("api.openai.com"));
}

#[test]
fn new_works_with_bare_origin() {
    let client = BackendOAuthClient::new("https://api.tinyhumans.ai").unwrap();
    let url = client.url_for("/teams/me/usage").unwrap();
    assert_eq!(url.path(), "/teams/me/usage");
}

#[test]
fn new_works_with_trailing_slash() {
    let client = BackendOAuthClient::new("https://api.tinyhumans.ai/").unwrap();
    let url = client.url_for("/teams/me/usage").unwrap();
    assert_eq!(url.path(), "/teams/me/usage");
}

#[tokio::test]
async fn backend_raw_client_inherits_x_core_version_default_header() {
    let (base_url, captured) = spawn_header_capture_server().await;
    let client = BackendOAuthClient::new(&base_url).unwrap();
    let url = client.url_for("/probe").unwrap();

    let response = client.raw_client().get(url).send().await.unwrap();
    assert!(response.status().is_success());

    let headers = captured.take();
    let request_headers = headers.last().unwrap();
    let version = request_headers
        .get("x-core-version")
        .and_then(|value| value.to_str().ok())
        .unwrap();
    assert_eq!(
        version,
        sanitize_client_version(env!("CARGO_PKG_VERSION")).unwrap()
    );
}
