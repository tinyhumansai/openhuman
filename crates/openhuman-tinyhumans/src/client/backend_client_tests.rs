use super::*;
use crate::client::classify::{backend_api_body_shape, BACKEND_API_BODY_SHAPE_MAX_BYTES};
use crate::client::{flatten_authed_error, HostedApiError};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use reqwest::Method;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

/// The attribution header the transport stamps on every request.
const PRODUCT_IDENTITY_HEADER: &str = "x-sdk-name";

#[tokio::test]
async fn authed_json_rejects_api_key_for_foreign_or_plaintext_endpoint() {
    use openhuman_embed::__host::security::credentials::session_support::BackendCredential;

    for endpoint in ["https://example.com", "http://example.com"] {
        let client = BackendClient::new(endpoint).unwrap();
        let error = client
            .authed_json(
                BackendCredential::ApiKey("th_test_key".to_string()),
                Method::GET,
                "/probe",
                None,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("TinyHumans API key requires"));
    }
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
    async fn capture_me(
        State(captured): State<CapturedHeaders>,
        headers: HeaderMap,
    ) -> Json<Value> {
        captured.push(&headers);
        Json(json!({
            "success": true,
            "data": { "_id": "user-123" }
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
        .route("/auth/me", get(capture_me))
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
async fn authed_json_sends_an_api_key_as_x_api_key_and_no_bearer() {
    // Library mode: the TinyHumans API key rides the SDK REST routes as
    // `x-api-key`, never as `Authorization: Bearer` (that header is the
    // managed-inference shape, handled by `OpenHumanBackendModel`).
    use openhuman_embed::__host::security::credentials::session_support::BackendCredential;

    let (base_url, captured) = spawn_header_capture_server().await;
    let client = BackendClient::new(&base_url).unwrap();

    let response = client
        .authed_json(
            &BackendCredential::ApiKey("th_test_key".to_string()),
            Method::GET,
            "/probe",
            None,
        )
        .await
        .unwrap();
    assert_eq!(response, json!({ "ok": true }));

    let headers = captured.take();
    let request_headers = headers.last().unwrap();
    assert_eq!(
        request_headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok()),
        Some("th_test_key")
    );
    assert!(
        request_headers.get("authorization").is_none(),
        "an API key must not also be sent as a bearer"
    );
    assert!(request_headers.get(PRODUCT_IDENTITY_HEADER).is_some());
}

#[tokio::test]
async fn authed_json_sends_a_session_credential_as_a_bearer_only() {
    use openhuman_embed::__host::security::credentials::session_support::BackendCredential;

    let (base_url, captured) = spawn_header_capture_server().await;
    let client = BackendClient::new(&base_url).unwrap();

    client
        .authed_json(
            &BackendCredential::Session("jwt-token".to_string()),
            Method::GET,
            "/probe",
            None,
        )
        .await
        .unwrap();

    let headers = captured.take();
    let request_headers = headers.last().unwrap();
    assert_eq!(
        request_headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer jwt-token")
    );
    assert!(request_headers.get("x-api-key").is_none());
}

// Regression: OPENHUMAN-TAURI-8K / Sentry issue 7473650958.
// When config.api_url is a full LLM completions URL (e.g. /v1/chat/completions),
// Url::join used to produce wrong paths like /v1/chat/teams/me/usage instead of
// /teams/me/usage — BackendClient::new must strip the path to prevent this.
#[test]
fn new_strips_path_from_completions_url() {
    for base in [
        "https://api.tinyhumans.ai/v1/chat/completions",
        "https://api.openai.com/v1/chat/completions",
        "https://api.tinyhumans.ai",
        "https://api.tinyhumans.ai/",
    ] {
        let client = BackendClient::new(base).unwrap();
        let url = client.url_for("/teams/me/usage").unwrap();
        assert_eq!(url.path(), "/teams/me/usage", "{base}");
    }
}

#[tokio::test]
async fn authed_json_surfaces_unauthorized_on_401() {
    // OPENHUMAN-TAURI-4K8: 401 on any authed backend endpoint must surface a
    // typed `HostedApiError::Unauthorized` and NOT funnel into `report_error`.
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
    let client = BackendClient::new(&base_url).unwrap();

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
    let typed = err.downcast_ref::<HostedApiError>().unwrap();
    let HostedApiError::Unauthorized { method, path } = typed else {
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
    let typed = err.downcast_ref::<HostedApiError>().unwrap();
    let HostedApiError::Unauthorized { method, path } = typed else {
        panic!("expected Unauthorized, got {typed:?}");
    };
    assert_eq!(method, "GET");
    assert_eq!(path, "/referral/stats");
}

/// Regression: a 401 for an API-key-authenticated request must classify as
/// `HostedApiError::ApiKeyRejected`, not `Unauthorized`. `Unauthorized` is
/// what `flatten_authed_error` maps onto the `SESSION_EXPIRED` sentinel, and
/// `core/jsonrpc.rs`'s `is_session_expired_error` treats that sentinel as
/// "clear the app session and sign out" — the wrong recovery for a
/// library-mode runtime that authenticates with an API key and has no
/// session at all.
#[tokio::test]
async fn authed_json_surfaces_api_key_rejected_not_unauthorized_on_401() {
    use openhuman_embed::__host::security::credentials::session_support::BackendCredential;

    let app = Router::new().route(
        "/teams/me/usage",
        get(|| async { (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized") }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{addr}");
    let client = BackendClient::new(&base_url).unwrap();

    let err = client
        .authed_json(
            BackendCredential::ApiKey("th_test_key".to_string()),
            Method::GET,
            "/teams/me/usage",
            None,
        )
        .await
        .unwrap_err();
    let typed = err.downcast_ref::<HostedApiError>().unwrap();
    let HostedApiError::ApiKeyRejected { method, path } = typed else {
        panic!("expected ApiKeyRejected for an api-key credential, got {typed:?}");
    };
    assert_eq!(method, "GET");
    assert_eq!(path, "/teams/me/usage");

    // The flattened message must NOT carry the `SESSION_EXPIRED` sentinel —
    // that would make `core/jsonrpc.rs::is_session_expired_error` clear an
    // app session that was never the problem.
    let flattened = flatten_authed_error(err);
    assert!(
        !flattened.contains("SESSION_EXPIRED"),
        "an api-key 401 must not trigger session-expiry recovery: {flattened}"
    );
    assert!(flattened.contains("API_KEY_REJECTED"), "{flattened}");

    // A session credential on the same endpoint still classifies as the
    // original `Unauthorized` / `SESSION_EXPIRED` path — this fix must not
    // regress the existing session-expiry recovery.
    let err = client
        .authed_json(
            BackendCredential::Session("mock-jwt".to_string()),
            Method::GET,
            "/teams/me/usage",
            None,
        )
        .await
        .unwrap_err();
    let typed = err.downcast_ref::<HostedApiError>().unwrap();
    assert!(
        matches!(typed, HostedApiError::Unauthorized { .. }),
        "expected Unauthorized for a session credential, got {typed:?}"
    );
    assert!(flatten_authed_error(err).contains("SESSION_EXPIRED"));
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

#[path = "backend_channel_tests.rs"]
mod channel_tests;
