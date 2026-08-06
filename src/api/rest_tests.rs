use super::{
    backend_api_body_shape, flatten_authed_error, is_announcements_latest_path,
    is_unmatched_route_404, key_bytes_from_string, parse_message_path, sanitize_client_version,
    BackendApiError, BackendOAuthClient, BACKEND_API_BODY_SHAPE_MAX_BYTES,
};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use reqwest::Method;
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
            "data": { "jwt": "mock-jwt-token" }
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
        .route("/auth/login-token/consume", post(capture_consume))
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
    assert_eq!(
        request_headers
            .get("x-sdk-client")
            .and_then(|value| value.to_str().ok()),
        Some("tinyhumans-rust"),
        "typed auth requests must be sent by the TinyHumans SDK transport"
    );
}

#[tokio::test]
async fn authed_json_uses_sdk_transport_with_bearer_and_host_headers() {
    let (base_url, captured) = spawn_header_capture_server().await;
    let client = BackendOAuthClient::new(&base_url).unwrap();

    let response = client
        .authed_json("sdk-cutover-token", Method::GET, "/probe", None)
        .await
        .unwrap();
    assert_eq!(response, json!({ "ok": true }));

    let headers = captured.take();
    let request_headers = headers.last().unwrap();
    assert_eq!(
        request_headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer sdk-cutover-token")
    );
    assert_eq!(
        request_headers
            .get("x-sdk-client")
            .and_then(|value| value.to_str().ok()),
        Some("tinyhumans-rust")
    );
    assert!(
        request_headers.get("x-core-version").is_some(),
        "OpenHuman host metadata must survive the SDK cutover"
    );
}

#[tokio::test]
async fn authed_json_cannot_bypass_sdk_admin_or_webhook_exclusions() {
    let client = BackendOAuthClient::new("http://127.0.0.1:9").unwrap();

    for (method, path) in [
        (Method::POST, "/admin/announcements"),
        (Method::GET, "/webhooks/core"),
    ] {
        let err = client
            .authed_json("token", method, path, None)
            .await
            .unwrap_err();
        assert!(
            err.chain().any(|source| {
                let message = source.to_string();
                message.contains("intentionally not exposed")
                    || message.contains("webhook routes are not exposed")
            }),
            "{path} must be rejected locally by the SDK: {err:#}"
        );
    }
}

#[tokio::test]
async fn backend_client_sends_x_tauri_version_when_env_set() {
    // Serialize against any concurrent test that also touches this env var.
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap();

    std::env::set_var("OPENHUMAN_TAURI_VERSION", "9.8.7-shell+test");
    let (base_url, captured) = spawn_header_capture_server().await;
    let client = BackendOAuthClient::new(&base_url).unwrap();
    let url = client.url_for("/probe").unwrap();
    let response = client.raw_client().get(url).send().await.unwrap();
    assert!(response.status().is_success());
    std::env::remove_var("OPENHUMAN_TAURI_VERSION");

    let headers = captured.take();
    let request_headers = headers.last().unwrap();
    let tauri_version = request_headers
        .get("x-tauri-version")
        .and_then(|value| value.to_str().ok())
        .unwrap();
    assert_eq!(tauri_version, "9.8.7-shell+test");
    // Core version still flows alongside the new tauri version header.
    assert!(request_headers.get("x-core-version").is_some());
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

#[tokio::test]
async fn authed_json_surfaces_message_not_found_on_404() {
    let app = Router::new()
        .route(
            "/channels/telegram/messages/1103",
            post(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") }),
        )
        .route(
            "/channels/discord/messages/abc",
            post(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{addr}");
    let client = BackendOAuthClient::new(&base_url).unwrap();

    // Telegram path — matches OPENHUMAN-TAURI-2Y shape.
    let err = client
        .authed_json(
            "mock-jwt",
            Method::POST,
            "/channels/telegram/messages/1103",
            None,
        )
        .await
        .unwrap_err();
    let typed = err.downcast_ref::<BackendApiError>().unwrap();
    let BackendApiError::MessageNotFound {
        provider,
        message_id,
    } = typed
    else {
        panic!("expected MessageNotFound, got {typed:?}");
    };
    assert_eq!(provider, "telegram");
    assert_eq!(message_id, "1103");

    // Discord path — proves the helper is provider-agnostic.
    let err = client
        .authed_json(
            "mock-jwt",
            Method::POST,
            "/channels/discord/messages/abc",
            None,
        )
        .await
        .unwrap_err();
    let typed = err.downcast_ref::<BackendApiError>().unwrap();
    let BackendApiError::MessageNotFound {
        provider,
        message_id,
    } = typed
    else {
        panic!("expected MessageNotFound, got {typed:?}");
    };
    assert_eq!(provider, "discord");
    assert_eq!(message_id, "abc");
}

#[tokio::test]
async fn authed_json_surfaces_announcement_not_found_on_404() {
    // TAURI-RUST-HW0 / TAURI-RUST-KHX: 404 on `/announcements/latest` must
    // surface a typed `BackendApiError::AnnouncementNotFound` (so the caller
    // can degrade to `null`) instead of a generic non-2xx error.
    let app = Router::new().route(
        "/announcements/latest",
        get(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{addr}");
    let client = BackendOAuthClient::new(&base_url).unwrap();

    let err = client
        .authed_json("mock-jwt", Method::GET, "/announcements/latest", None)
        .await
        .unwrap_err();
    let typed = err.downcast_ref::<BackendApiError>().unwrap();
    assert!(matches!(typed, BackendApiError::AnnouncementNotFound));
}

#[tokio::test]
async fn authed_json_only_classifies_get_announcements_latest_as_not_found() {
    // Defense-in-depth: a 404 on a *different* path must not be misclassified
    // as AnnouncementNotFound just because it shares a prefix/suffix.
    let app = Router::new().route(
        "/announcements/latest/extra",
        get(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{addr}");
    let client = BackendOAuthClient::new(&base_url).unwrap();

    let err = client
        .authed_json("mock-jwt", Method::GET, "/announcements/latest/extra", None)
        .await
        .unwrap_err();
    assert!(err.downcast_ref::<BackendApiError>().is_none());
}

#[tokio::test]
async fn authed_json_surfaces_announcement_not_found_with_base_path_prefix() {
    // OPENHUMAN-TAURI-R7-style regression: a BACKEND_URL/path override that
    // makes the resolved path `/api/v1/announcements/latest` must still
    // classify as AnnouncementNotFound, not fall through to a generic error.
    let app = Router::new().route(
        "/api/v1/announcements/latest",
        get(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{addr}");
    let client = BackendOAuthClient::new(&base_url).unwrap();

    let err = client
        .authed_json(
            "mock-jwt",
            Method::GET,
            "/api/v1/announcements/latest",
            None,
        )
        .await
        .unwrap_err();
    let typed = err.downcast_ref::<BackendApiError>().unwrap();
    assert!(matches!(typed, BackendApiError::AnnouncementNotFound));
}

#[tokio::test]
async fn authed_json_surfaces_unauthorized_on_401() {
    // OPENHUMAN-TAURI-4K8: 401 on any authed backend endpoint must surface a
    // typed `BackendApiError::Unauthorized` and NOT funnel into `report_error`.
    // The mascot TTS path (`/openai/v1/audio/speech`) was the loudest reporter,
    // but the same shape fires on every authed endpoint once a session lapses,
    // so we cover two different paths/methods to prove the suppression is
    // status-driven, not path-keyed.
    let app = Router::new()
        .route(
            "/openai/v1/audio/speech",
            post(|| async { (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized") }),
        )
        .route(
            "/referral/stats",
            get(|| async { (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized") }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{addr}");
    let client = BackendOAuthClient::new(&base_url).unwrap();

    // Mascot TTS path — the original reporter.
    let err = client
        .authed_json(
            "mock-jwt",
            Method::POST,
            "/openai/v1/audio/speech",
            Some(json!({ "text": "hello" })),
        )
        .await
        .unwrap_err();
    let typed = err.downcast_ref::<BackendApiError>().unwrap();
    let BackendApiError::Unauthorized { method, path } = typed else {
        panic!("expected Unauthorized, got {typed:?}");
    };
    assert_eq!(method, "POST");
    assert_eq!(path, "/openai/v1/audio/speech");

    // Generic GET on a non-TTS path — proves the suppression is per-status,
    // not per-path. (Same root cause: expired/revoked backend session.)
    let err = client
        .authed_json("mock-jwt", Method::GET, "/referral/stats", None)
        .await
        .unwrap_err();
    let typed = err.downcast_ref::<BackendApiError>().unwrap();
    let BackendApiError::Unauthorized { method, path } = typed else {
        panic!("expected Unauthorized, got {typed:?}");
    };
    assert_eq!(method, "GET");
    assert_eq!(path, "/referral/stats");
}

#[test]
fn backend_api_body_shape_emits_safe_keys_not_values() {
    // PII guard (Codex P1 on #4058): the body SHAPE must expose only schema-like
    // top-level key NAMES and NEVER the values — a non-2xx body can carry emails /
    // tokens / profile JSON that would otherwise leak to unscrubbed daily logs.
    let body = r#"{"error":"not found","email":"jo@example.com","token":"sk-secret"}"#;
    let shape = backend_api_body_shape(body);
    assert_eq!(shape, "object(keys=3,safe=[email,error,token],redacted=0)");
    assert!(!shape.contains("jo@example.com"), "value leaked: {shape}");
    assert!(!shape.contains("sk-secret"), "value leaked: {shape}");
    assert!(!shape.contains("not found"), "value leaked: {shape}");
}

#[test]
fn backend_api_body_shape_redacts_pii_and_nonidentifier_keys() {
    // CodeRabbit Major on #4058: key NAMES are response-controlled too. A foreign
    // backend can put an email / free text / unicode in the KEY position; those
    // must be counted as `redacted`, never echoed.
    let body = r#"{"jo@example.com":1,"a b":2,"naïve":3,"error":4}"#;
    let shape = backend_api_body_shape(body);
    // Only the schema-like `error` survives; the other three are redacted.
    assert_eq!(shape, "object(keys=4,safe=[error],redacted=3)");
    assert!(!shape.contains("jo@example.com"), "PII key leaked: {shape}");
    assert!(!shape.contains("naïve"), "non-ascii key leaked: {shape}");
    assert!(!shape.contains("a b"), "free-text key leaked: {shape}");
}

#[test]
fn backend_api_body_shape_classifies_non_object_bodies() {
    assert_eq!(backend_api_body_shape(""), "empty");
    assert_eq!(backend_api_body_shape("   "), "empty");
    assert_eq!(
        backend_api_body_shape("Cannot GET /teams/me/usage"),
        "non_json"
    );
    assert_eq!(backend_api_body_shape("<html>404</html>"), "non_json");
    assert_eq!(backend_api_body_shape("[1,2,3]"), "array");
    assert_eq!(backend_api_body_shape("42"), "scalar");
}

#[test]
fn backend_api_body_shape_bounds_long_safe_key_list() {
    // The `safe=[…]` list is truncated at BACKEND_API_BODY_SHAPE_MAX_BYTES = 120.
    // Surviving keys are ASCII identifiers (non-ASCII keys are redacted upstream),
    // so build many ASCII keys to overflow the cap and assert the truncation
    // CONTRACT: bounded, ellipsis-terminated, and not carrying the last key.
    let mut obj = serde_json::Map::new();
    for i in 0..30 {
        obj.insert(format!("field{i:02}"), json!(1)); // 30 × "fieldNN" (7 bytes) ≫ 120
    }
    let body = serde_json::to_string(&Value::Object(obj)).unwrap();
    let shape = backend_api_body_shape(&body);

    let keys = shape
        .strip_prefix("object(keys=30,safe=[")
        .and_then(|s| s.strip_suffix("],redacted=0)"))
        .unwrap_or_else(|| panic!("unexpected shape: {shape}"));
    assert!(
        keys.len() <= BACKEND_API_BODY_SHAPE_MAX_BYTES,
        "safe list exceeds cap ({} > {BACKEND_API_BODY_SHAPE_MAX_BYTES}): {keys}",
        keys.len()
    );
    assert!(keys.ends_with('…'), "expected ellipsis-terminated: {keys}");
    assert!(
        !keys.contains("field29"),
        "last key should be truncated away: {keys}"
    );
}

#[tokio::test]
async fn authed_json_reports_non_channel_404_still_propagates() {
    // TAURI-RUST-8C: a GET 404 on a non-channel path (e.g. `/teams/me/usage`)
    // falls through to `report_error` (not a typed/suppressed state) — it must
    // still return an Err (no suppression) and not a typed `BackendApiError`.
    let app = Router::new().route(
        "/teams/me/usage",
        get(|| async {
            (
                axum::http::StatusCode::NOT_FOUND,
                r#"{"message":"Not Found"}"#,
            )
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{addr}");
    let client = BackendOAuthClient::new(&base_url).unwrap();

    let err = client
        .authed_json("mock-jwt", Method::GET, "/teams/me/usage", None)
        .await
        .unwrap_err();
    assert!(err.downcast_ref::<BackendApiError>().is_none());
    let msg = format!("{err:#}");
    assert!(msg.contains("404"), "error should carry the status: {msg}");
    assert!(
        msg.contains("/teams/me/usage"),
        "error should carry the path: {msg}"
    );
}

#[test]
fn flatten_authed_error_maps_unauthorized_to_session_expired_sentinel() {
    // #3297: the typed `Unauthorized` (expected session-lapse 401) must flatten
    // onto a string that the JSON-RPC session-expiry classifiers recognise, so
    // it is suppressed from Sentry (TAURI-RUST-8WY / 8WZ) instead of leaking.
    let err = anyhow::Error::new(BackendApiError::Unauthorized {
        method: "GET".to_string(),
        path: "/teams/me/usage".to_string(),
    });
    let flat = flatten_authed_error(err);

    // Carries the SESSION_EXPIRED sentinel + preserves method/path for logs.
    assert!(
        flat.contains("SESSION_EXPIRED"),
        "expected sentinel, got: {flat}"
    );
    assert!(flat.contains("GET"), "method preserved: {flat}");
    assert!(flat.contains("/teams/me/usage"), "path preserved: {flat}");

    // Contract cross-check: the flattened string MUST classify as session
    // expiry. This couples the mapping to the actual classifier — if either the
    // sentinel or the classifier drifts, this fails instead of silently leaking.
    assert!(
        crate::core::observability::is_session_expired_message(&flat),
        "flattened Unauthorized must classify as session expiry: {flat}"
    );
}

#[test]
fn flatten_authed_error_preserves_non_unauthorized_chain() {
    // A non-Unauthorized failure (e.g. a transient network/timeout error) keeps
    // its full `{e:#}` anyhow chain and must NOT be demoted to session expiry —
    // genuine failures still reach Sentry.
    let err = anyhow::anyhow!("connect timeout").context("backend request GET /teams/me/usage");
    let flat = flatten_authed_error(err);

    assert!(!flat.contains("SESSION_EXPIRED"), "must not map: {flat}");
    assert!(flat.contains("connect timeout"), "cause preserved: {flat}");
    assert!(
        !crate::core::observability::is_session_expired_message(&flat),
        "non-auth error must NOT classify as session expiry: {flat}"
    );
}

#[test]
fn flatten_authed_error_does_not_swallow_message_not_found() {
    // `MessageNotFound` is a different expected state handled by its own callers
    // (channel streaming/delete paths downcast it); it must not be collapsed
    // into the session-expiry sentinel here.
    let err = anyhow::Error::new(BackendApiError::MessageNotFound {
        provider: "telegram".to_string(),
        message_id: "1103".to_string(),
    });
    let flat = flatten_authed_error(err);

    assert!(!flat.contains("SESSION_EXPIRED"), "must not map: {flat}");
    assert!(
        flat.contains("message not found"),
        "display preserved: {flat}"
    );
}

#[test]
fn flatten_authed_error_does_not_swallow_announcement_not_found() {
    // `announcements::ops::get_latest_announcement` intercepts
    // `AnnouncementNotFound` before it ever reaches `flatten_authed_error`, but
    // this is defense-in-depth: if a future caller skips that interception,
    // `flatten_authed_error` must still preserve the typed state's Display
    // text rather than collapsing it into the session-expiry sentinel.
    let err = anyhow::Error::new(BackendApiError::AnnouncementNotFound);
    let flat = flatten_authed_error(err);

    assert!(!flat.contains("SESSION_EXPIRED"), "must not map: {flat}");
    assert!(
        flat.contains("no announcement available"),
        "display preserved: {flat}"
    );
}

#[tokio::test]
async fn authed_json_403_is_not_demoted_to_unauthorized() {
    // 403 (Forbidden) is a genuine authorization/permission problem — the
    // token authenticated but lacked scope. That IS a code/config bug we
    // want to keep in Sentry; only 401 (token rejected as a whole) maps
    // to the expected-state `Unauthorized` variant.
    let app = Router::new().route(
        "/openai/v1/audio/speech",
        post(|| async { (axum::http::StatusCode::FORBIDDEN, "Forbidden") }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{addr}");
    let client = BackendOAuthClient::new(&base_url).unwrap();

    let err = client
        .authed_json("mock-jwt", Method::POST, "/openai/v1/audio/speech", None)
        .await
        .unwrap_err();
    assert!(
        err.downcast_ref::<BackendApiError>().is_none(),
        "403 must not be classified as Unauthorized"
    );
}

#[tokio::test]
async fn authed_json_404_outside_messages_path_still_reports() {
    // 404 on a non-`/channels/<provider>/messages/<id>` path should NOT be
    // demoted to MessageNotFound — it's a real backend bug or routing
    // mistake and must keep its Sentry signal.
    let app = Router::new().route(
        "/auth/profile",
        get(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{addr}");
    let client = BackendOAuthClient::new(&base_url).unwrap();

    let err = client
        .authed_json("mock-jwt", Method::GET, "/auth/profile", None)
        .await
        .unwrap_err();
    assert!(
        err.downcast_ref::<BackendApiError>().is_none(),
        "non-channel-message 404 must not be classified as MessageNotFound"
    );
}

// ── parse_message_path unit tests (TAURI-R7 regression guard) ───────────────

#[test]
fn parse_message_path_canonical_form() {
    assert_eq!(
        parse_message_path("/channels/telegram/messages/1103"),
        Some(("telegram", "1103"))
    );
}

#[test]
fn parse_message_path_discord_provider() {
    assert_eq!(
        parse_message_path("/channels/discord/messages/abc"),
        Some(("discord", "abc"))
    );
}

#[test]
fn parse_message_path_base_path_prefix() {
    // TAURI-R7 root cause: BACKEND_URL with a path prefix adds segments,
    // breaking the strict 4-segment check. The sliding window must handle it.
    assert_eq!(
        parse_message_path("/api/v1/channels/telegram/messages/1103"),
        Some(("telegram", "1103"))
    );
}

#[test]
fn parse_message_path_double_prefix() {
    assert_eq!(
        parse_message_path("/v2/api/channels/discord/messages/abc"),
        Some(("discord", "abc"))
    );
}

#[test]
fn parse_message_path_trailing_slash() {
    assert_eq!(
        parse_message_path("/channels/telegram/messages/1103/"),
        Some(("telegram", "1103"))
    );
}

#[test]
fn parse_message_path_percent_encoded_slug() {
    // Channel slugs with percent-encoded characters must pass through verbatim.
    assert_eq!(
        parse_message_path("/channels/telegram%3Abot/messages/1103"),
        Some(("telegram%3Abot", "1103"))
    );
}

#[test]
fn parse_message_path_non_message_path_returns_none() {
    assert_eq!(parse_message_path("/channels/telegram/typing"), None);
    assert_eq!(parse_message_path("/channels/telegram"), None);
    assert_eq!(parse_message_path("/auth/profile"), None);
    assert_eq!(parse_message_path("/"), None);
    assert_eq!(parse_message_path(""), None);
}

#[test]
fn is_announcements_latest_path_matches_canonical_form() {
    assert!(is_announcements_latest_path("/announcements/latest"));
}

#[test]
fn is_announcements_latest_path_tolerates_base_path_prefix() {
    // Same OPENHUMAN-TAURI-R7 reasoning as parse_message_path: a BACKEND_URL
    // override with a path prefix must not defeat the 404 classification.
    assert!(is_announcements_latest_path("/api/v1/announcements/latest"));
    assert!(is_announcements_latest_path("/v2/api/announcements/latest"));
}

#[test]
fn is_announcements_latest_path_trailing_slash() {
    assert!(is_announcements_latest_path("/announcements/latest/"));
}

#[test]
fn is_announcements_latest_path_rejects_other_paths() {
    assert!(!is_announcements_latest_path("/announcements/latest/extra"));
    assert!(!is_announcements_latest_path("/announcements"));
    assert!(!is_announcements_latest_path("/latest"));
    assert!(!is_announcements_latest_path("/auth/profile"));
    assert!(!is_announcements_latest_path("/"));
    assert!(!is_announcements_latest_path(""));
}

// ── authed_json defense-in-depth: PATCH 404 with base-path prefix ───────────

#[tokio::test]
async fn authed_json_patch_404_with_base_path_prefix_does_not_report() {
    // Regression for TAURI-R7: if the resolved URL has a base-path prefix,
    // authed_json must still suppress the 404 — NOT call report_error.
    //
    // Since BackendOAuthClient strips the base path in `new()`, the path
    // passed to authed_json is always joined against the stripped base. We
    // verify that a PATCH 404 returns an error without panicking and that it
    // is not classified as a code bug (no Sentry event).
    //
    // #5230: the classification is `ChannelEditUnsupported`, NOT
    // `MessageNotFound`. The backend implements no `PATCH
    // /channels/:channel/messages/:messageId`, so this 404 is route absence.
    // Reporting it as a missing *message* made `bus.rs` forget a message id it
    // still owned, orphaning the streaming draft and the "💭 Thinking:" bubble.
    let app = axum::Router::new().route(
        "/channels/telegram/messages/9999",
        axum::routing::any(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{addr}");
    let client = BackendOAuthClient::new(&base_url).unwrap();

    let err = client
        .authed_json(
            "mock-jwt",
            Method::PATCH,
            "/channels/telegram/messages/9999",
            None,
        )
        .await
        .unwrap_err();
    let typed = err.downcast_ref::<BackendApiError>().unwrap();
    let BackendApiError::ChannelEditUnsupported {
        provider,
        message_id,
    } = typed
    else {
        panic!("expected ChannelEditUnsupported, got {typed:?}");
    };
    assert_eq!(provider, "telegram");
    assert_eq!(message_id, "9999");
}

#[tokio::test]
async fn send_channel_edit_404_is_route_absence_not_a_missing_message() {
    // #5230 root-cause pin, driven through the real client method rather than
    // `authed_json` directly: the deployed backend serves only
    // `POST /channels/:channel/messages` and
    // `DELETE /channels/:channel/messages/:messageId`, so every edit hits the
    // unmatched-route 404. It must NOT surface as `MessageNotFound` — that
    // variant means "this message is gone", and acting on it discards a
    // message id that is still live.
    // POST + DELETE only — deliberately mirrors the real backend router. The
    // explicit `fallback` reproduces Express's behaviour for an unmatched
    // method+path pair: it falls through to the app's 404 handler rather than
    // answering 405 (which is what axum would do on its own).
    let app = Router::new().route(
        "/channels/telegram/messages/1103",
        post(|| async { (axum::http::StatusCode::OK, "{\"success\":true}") })
            .delete(|| async { (axum::http::StatusCode::OK, "{\"success\":true}") })
            .fallback(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = BackendOAuthClient::new(&format!("http://{addr}")).unwrap();
    let err = client
        .send_channel_edit("telegram", "1103", "mock-jwt", serde_json::json!({}))
        .await
        .unwrap_err();

    let typed = err
        .downcast_ref::<BackendApiError>()
        .expect("edit 404 must carry a typed BackendApiError");
    assert!(
        matches!(typed, BackendApiError::ChannelEditUnsupported { .. }),
        "edit 404 must be ChannelEditUnsupported, got {typed:?}"
    );
    assert!(
        !matches!(typed, BackendApiError::MessageNotFound { .. }),
        "a missing edit route must never masquerade as a deleted message"
    );
}

#[tokio::test]
async fn channel_edit_404_from_a_real_handler_stays_a_missing_message() {
    // #5230 review: the twin of the test above, for the world where the edit
    // route DOES exist (staging, a custom backend, or after the backend PR
    // lands). A handler answering "that message is gone" returns a JSON
    // envelope, exactly as `DELETE /channels/:channel/messages/:messageId`
    // already does. Classifying that as `ChannelEditUnsupported` would make
    // `bus.rs` call `mark_channel_edits_unsupported` and switch progressive
    // edits off for the whole provider for the rest of the process — because
    // one message expired.
    let app = Router::new().route(
        "/channels/telegram/messages/1103",
        axum::routing::patch(|| async {
            (
                axum::http::StatusCode::NOT_FOUND,
                "{\"success\":false,\"error\":\"message not found\"}",
            )
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = BackendOAuthClient::new(&format!("http://{addr}")).unwrap();
    let err = client
        .send_channel_edit("telegram", "1103", "mock-jwt", serde_json::json!({}))
        .await
        .unwrap_err();

    let typed = err
        .downcast_ref::<BackendApiError>()
        .expect("edit 404 must carry a typed BackendApiError");
    assert!(
        matches!(typed, BackendApiError::MessageNotFound { .. }),
        "a handler-level edit 404 must stay per-message, got {typed:?}"
    );
    assert!(
        !matches!(typed, BackendApiError::ChannelEditUnsupported { .. }),
        "one missing message must not disable edits for the whole provider"
    );
}

#[tokio::test]
async fn channel_edit_404_on_a_prefixed_path_keeps_the_parsed_ids() {
    // A `BACKEND_URL` with a base-path prefix plus a trailing segment. The
    // sliding window in `parse_message_path` still finds
    // `[channels, telegram, messages, 9999]`, so this must be classified as
    // route absence for PATCH *and* carry the parsed ids — not fall through to
    // the generic untyped bail! that the DELETE branch uses.
    let app = axum::Router::new().route(
        "/api/v1/channels/telegram/messages/9999/extra",
        axum::routing::any(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = BackendOAuthClient::new(&format!("http://{addr}")).unwrap();
    let err = client
        .authed_json(
            "mock-jwt",
            Method::PATCH,
            "/api/v1/channels/telegram/messages/9999/extra",
            None,
        )
        .await
        .unwrap_err();

    let typed = err
        .downcast_ref::<BackendApiError>()
        .expect("prefixed edit path must still carry a typed error");
    match typed {
        BackendApiError::ChannelEditUnsupported {
            provider,
            message_id,
        } => {
            assert_eq!(provider, "telegram");
            assert_eq!(message_id, "9999");
        }
        other => panic!("expected ChannelEditUnsupported, got {other:?}"),
    }
}

#[tokio::test]
async fn channel_edit_404_on_an_undecomposable_path_falls_back_to_unknown_ids() {
    // The case that actually exercises `authed_json`'s
    // `unwrap_or_else(("unknown", "unknown"))`: an empty message-id segment.
    // `parse_message_path` drops empty segments, so this yields only
    // `[channels, telegram, messages]` — no 4-window, hence `None` — while the
    // path still satisfies the `/channels/` + `/messages/` substring guard, so
    // the PATCH branch is entered with nothing parsed.
    let app = axum::Router::new().route(
        "/channels/telegram/messages/",
        axum::routing::any(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = BackendOAuthClient::new(&format!("http://{addr}")).unwrap();
    let err = client
        .authed_json(
            "mock-jwt",
            Method::PATCH,
            "/channels/telegram/messages/",
            None,
        )
        .await
        .unwrap_err();

    let typed = err
        .downcast_ref::<BackendApiError>()
        .expect("undecomposable edit path must still carry a typed error");
    match typed {
        BackendApiError::ChannelEditUnsupported {
            provider,
            message_id,
        } => {
            assert_eq!(provider, "unknown");
            assert_eq!(message_id, "unknown");
        }
        other => panic!("expected ChannelEditUnsupported, got {other:?}"),
    }
}

#[tokio::test]
async fn channel_delete_404_still_means_the_message_is_gone() {
    // The fix must not widen: `DELETE` keeps `MessageNotFound`, whose
    // provider-side-deletion semantics are exactly what its caller
    // (`delete_channel_message`) wants — "already gone, nothing to clean up".
    let app = Router::new().route(
        "/channels/telegram/messages/1103",
        axum::routing::delete(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = BackendOAuthClient::new(&format!("http://{addr}")).unwrap();
    let err = client
        .send_channel_delete("telegram", "1103", "mock-jwt")
        .await
        .unwrap_err();

    let typed = err.downcast_ref::<BackendApiError>().unwrap();
    assert!(
        matches!(typed, BackendApiError::MessageNotFound { .. }),
        "DELETE 404 must stay MessageNotFound, got {typed:?}"
    );
}

// The channel methods below now reach the backend through the vendored
// `tinyhumans-sdk` transport instead of `authed_json`. The routes they call
// still classify expected backend states the same way — a route must not
// change its Sentry or session-expiry behaviour just because it moved onto a
// typed SDK method. These pin that equivalence.

#[tokio::test]
async fn sdk_backed_channel_delete_surfaces_message_not_found_on_404() {
    let app = Router::new().route(
        "/channels/telegram/messages/1103",
        axum::routing::delete(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = BackendOAuthClient::new(&format!("http://{addr}")).unwrap();
    let err = client
        .send_channel_delete("telegram", "1103", "mock-jwt")
        .await
        .unwrap_err();

    let typed = err.downcast_ref::<BackendApiError>().unwrap();
    let BackendApiError::MessageNotFound {
        provider,
        message_id,
    } = typed
    else {
        panic!("expected MessageNotFound, got {typed:?}");
    };
    assert_eq!(provider, "telegram");
    assert_eq!(message_id, "1103");
}

#[tokio::test]
async fn sdk_backed_channel_typing_surfaces_unauthorized_on_401() {
    let app = Router::new().route(
        "/channels/telegram/typing",
        post(|| async { (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized") }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = BackendOAuthClient::new(&format!("http://{addr}")).unwrap();
    let err = client
        .send_channel_typing("telegram", "mock-jwt")
        .await
        .unwrap_err();

    let typed = err.downcast_ref::<BackendApiError>().unwrap();
    let BackendApiError::Unauthorized { method, path } = typed else {
        panic!("expected Unauthorized, got {typed:?}");
    };
    assert_eq!(method, "POST");
    assert_eq!(path, "/channels/telegram/typing");
    // The session-expiry sentinel must still be derivable, so the dispatcher
    // keeps routing this to re-sign-in rather than to Sentry.
    assert!(flatten_authed_error(err).starts_with("SESSION_EXPIRED:"));
}

// The SDK transport must inherit this crate's client, so the version headers
// and timeouts apply to SDK-backed calls exactly as they do to `authed_json`.
#[tokio::test]
async fn sdk_backed_calls_send_the_core_version_header() {
    let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let app = Router::new()
        .route(
            "/channels/telegram/typing",
            post(
                |State(state): State<Arc<Mutex<Option<String>>>>, headers: HeaderMap| async move {
                    *state.lock().unwrap() = headers
                        .get("x-core-version")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    Json(json!({"success": true, "data": {}}))
                },
            ),
        )
        .with_state(seen.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = BackendOAuthClient::new(&format!("http://{addr}")).unwrap();
    client
        .send_channel_typing("telegram", "mock-jwt")
        .await
        .unwrap();

    assert_eq!(
        seen.lock().unwrap().as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
}

// ── is_unmatched_route_404 (#5230 review) ──────────────────────────────────
//
// A PATCH 404 becomes `ChannelEditUnsupported`, which disables progressive edits
// for the whole provider for the rest of the process. That is only correct when
// the *route* is absent. Once the backend implements the route, a handler-level
// "that message is gone" 404 has to stay a per-message `MessageNotFound`, or one
// deleted message would switch edits off for everyone.

#[test]
fn unmatched_route_404_is_expresss_html_page() {
    // Express's built-in finalhandler — what the backend returns today, since it
    // registers no catch-all 404 and implements no PATCH route.
    assert!(is_unmatched_route_404(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<title>Error</title>\n</head>\n<body>\n         <pre>Cannot PATCH /channels/telegram/messages/1103</pre>\n</body>\n</html>"
    ));
}

#[test]
fn unmatched_route_404_covers_empty_and_plain_text_bodies() {
    // Ambiguous shapes default to route absence, preserving today's behaviour.
    assert!(is_unmatched_route_404(""));
    assert!(is_unmatched_route_404("   "));
    assert!(is_unmatched_route_404("Not Found"));
}

#[test]
fn handler_level_404_json_envelope_is_not_route_absence() {
    // The shape `DELETE /channels/:channel/messages/:messageId` already returns,
    // and the one a future PATCH handler would mirror.
    assert!(!is_unmatched_route_404(
        r#"{"success": false, "error": "message not found"}"#
    ));
    assert!(!is_unmatched_route_404(r#"{"error":"gone"}"#));
}
