use super::*;
use crate::client::classify::{
    backend_api_body_shape, backend_api_error_code, BACKEND_API_BODY_SHAPE_MAX_BYTES,
    BACKEND_API_ERROR_BODY_MAX_BYTES,
};
use crate::client::{flatten_authed_error, HostedApiError};
use crate::transport::{billing_refusal, BillingRefusal};
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
    use openhuman_embed::__host::credentials::backend_credential::BackendCredential;

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
    // managed-inference shape, handled by `TinyHumansModel` through the host adapter).
    use openhuman_embed::__host::credentials::backend_credential::BackendCredential;

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
    use openhuman_embed::__host::credentials::backend_credential::BackendCredential;

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
    use openhuman_embed::__host::credentials::backend_credential::BackendCredential;

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

#[tokio::test]
async fn non_2xx_error_body_is_truncated_in_the_returned_error() {
    let body = format!("{}SECRET_TAIL", "x".repeat(1024 * 1024));
    let app = Router::new().route(
        "/teams/me/usage",
        get(move || {
            let body = body.clone();
            async move { (axum::http::StatusCode::UNPROCESSABLE_ENTITY, body) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = BackendClient::new(&format!("http://{addr}")).unwrap();
    let err = client
        .authed_json("mock-jwt", Method::GET, "/teams/me/usage", None)
        .await
        .unwrap_err();
    let message = flatten_authed_error(err);
    assert!(message.contains("failed (422"), "{message}");
    assert!(!message.contains("SECRET_TAIL"));
    assert!(
        message.len() < BACKEND_API_ERROR_BODY_MAX_BYTES + 256,
        "{}",
        message.len()
    );
}

#[test]
fn backend_api_error_code_only_echoes_identifier_like_codes() {
    let body = r#"{"success":false,"error":"x","errorCode":"INVALID_ARGUMENT"}"#;
    assert_eq!(
        backend_api_error_code(body).as_deref(),
        Some("INVALID_ARGUMENT")
    );
    let free_text = r#"{"errorCode":"bad value for jo@example.com"}"#;
    assert_eq!(backend_api_error_code(free_text), None);
    assert_eq!(backend_api_error_code("not json"), None);
}

#[test]
fn backend_api_error_code_rejects_non_identifier_and_non_string_codes() {
    let body = r#"{"success":false,"error":"jo@example.com is not entitled","errorCode":"FORBIDDEN_PLAN"}"#;
    assert_eq!(
        backend_api_error_code(body).as_deref(),
        Some("FORBIDDEN_PLAN")
    );
    assert_eq!(backend_api_error_code(r#"{"errorCode":7}"#), None);
    assert_eq!(backend_api_error_code(r#"{"error":"x"}"#), None);
    assert_eq!(backend_api_error_code("Forbidden"), None);
}

/// The exact 182-byte body behind TAURI-RUST-13BR (backend
/// `gemini/liveSessions.ts`): balance below the Gemini Live session reserve.
const GEMINI_LIVE_RESERVE_BODY: &str = r#"{"success":false,"error":"Gemini Live requires an available balance of at least $1.00 (reserved per open session)","errorCode":"USER_INSUFFICIENT_CREDITS","details":{"reserveUsd":1}}"#;

/// The transport's classifier on a raw body, parsed as the SDK hands it over
/// (JSON when it parses, otherwise the text as a JSON string).
fn backend_billing_refusal(status: u16, body: &str) -> Option<BillingRefusal> {
    let body = serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.into()));
    billing_refusal(status, &body)
}

#[test]
fn backend_billing_refusal_classifies_the_live_voice_gates() {
    assert_eq!(GEMINI_LIVE_RESERVE_BODY.len(), 182);
    assert_eq!(
        backend_billing_refusal(400, GEMINI_LIVE_RESERVE_BODY),
        Some(BillingRefusal::InsufficientCredits)
    );
    // TAURI-RUST-11G3: the voice-agent signed-url gate answers 403 FORBIDDEN.
    let balance = r#"{"success":false,"error":"Insufficient balance to start a voice session. Add credits to continue.","errorCode":"FORBIDDEN"}"#;
    assert_eq!(
        backend_billing_refusal(403, balance),
        Some(BillingRefusal::InsufficientCredits)
    );
    let plan = r#"{"success":false,"error":"Voice sessions require a paid plan.","errorCode":"FORBIDDEN"}"#;
    assert_eq!(
        backend_billing_refusal(403, plan),
        Some(BillingRefusal::PaidPlanRequired)
    );
}

#[test]
fn backend_billing_refusal_leaves_other_failures_alone() {
    // A real 403 (wrong role) and a validation 400 must keep reaching Sentry.
    let forbidden = r#"{"success":false,"error":"Forbidden","errorCode":"FORBIDDEN"}"#;
    assert_eq!(backend_billing_refusal(403, forbidden), None);
    let invalid = r#"{"success":false,"error":"Invalid request","errorCode":"VALIDATION_ERROR","details":{}}"#;
    assert_eq!(backend_billing_refusal(400, invalid), None);
    // The billing wording only counts on a 403, and only in the `error` field.
    let paid_400 = r#"{"success":false,"error":"Voice sessions require a paid plan.","errorCode":"BAD_REQUEST"}"#;
    assert_eq!(backend_billing_refusal(400, paid_400), None);
    let echoed = r#"{"success":false,"error":"Forbidden","details":{"hint":"add credits"}}"#;
    assert_eq!(backend_billing_refusal(403, echoed), None);
    assert_eq!(backend_billing_refusal(403, "Insufficient balance"), None);
}

#[tokio::test]
async fn authed_json_surfaces_billing_refusals_as_typed_errors() {
    let app = Router::new()
        .route(
            "/agent-integrations/gemini/live/sessions",
            post(|| async {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    GEMINI_LIVE_RESERVE_BODY,
                )
            }),
        )
        .route(
            "/agent-integrations/coded-budget",
            post(|| async {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(json!({
                        "success": false,
                        "error": "Insufficient balance",
                        "errorCode": "USER_INSUFFICIENT_CREDITS",
                    })),
                )
            }),
        )
        .route(
            "/voice-agent/get-signed-url",
            get(|| async {
                (
                    axum::http::StatusCode::FORBIDDEN,
                    Json(json!({
                        "success": false,
                        "error": "Voice sessions require a paid plan.",
                        "errorCode": "FORBIDDEN",
                    })),
                )
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = BackendClient::new(&format!("http://{addr}")).unwrap();

    let err = client
        .authed_json(
            "mock-jwt",
            Method::POST,
            "/agent-integrations/gemini/live/sessions",
            Some(json!({ "mode": "conversation" })),
        )
        .await
        .unwrap_err();
    let typed = err.downcast_ref::<HostedApiError>().unwrap();
    let HostedApiError::InsufficientCredits { status, path, .. } = typed else {
        panic!("expected InsufficientCredits, got {typed:?}");
    };
    assert_eq!(*status, 400);
    assert_eq!(path, "/agent-integrations/gemini/live/sessions");
    // Same message shape as the generic failure, behind the sentinel.
    let flattened = flatten_authed_error(err);
    assert!(
        flattened.starts_with(
            "INSUFFICIENT_CREDITS:POST /agent-integrations/gemini/live/sessions failed (400): "
        ),
        "{flattened}"
    );
    assert!(
        flattened.contains("USER_INSUFFICIENT_CREDITS"),
        "{flattened}"
    );

    // The explicit code wins over the generic budget-text path.
    assert!(tinyinference_providers::is_budget_exhausted_message(
        "Insufficient balance"
    ));
    let err = client
        .authed_json(
            "mock-jwt",
            Method::POST,
            "/agent-integrations/coded-budget",
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<HostedApiError>(),
            Some(HostedApiError::InsufficientCredits { status: 400, .. })
        ),
        "expected InsufficientCredits, got {err:?}"
    );

    let err = client
        .authed_json("mock-jwt", Method::GET, "/voice-agent/get-signed-url", None)
        .await
        .unwrap_err();
    let typed = err.downcast_ref::<HostedApiError>().unwrap();
    assert!(
        matches!(typed, HostedApiError::PaidPlanRequired { status: 403, .. }),
        "expected PaidPlanRequired, got {typed:?}"
    );
    assert!(flatten_authed_error(err).starts_with("PAID_PLAN_REQUIRED:"));
}
