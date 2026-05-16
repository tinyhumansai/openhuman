//! Tests for the shared integrations HTTP client.
//!
//! Focus: backend error body propagation. Pre-fix, non-2xx responses
//! discarded the body (`let _body_text = …`) leaving callers with a
//! generic `"Backend returned 400 …"` message — see #1296. These tests
//! lock in the new behaviour where `extract_error_detail` pulls the
//! envelope's `error` field (or falls back to truncated raw text) and
//! the bail message includes it.

use super::*;
use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;

// ── Unit: `extract_error_detail` ──────────────────────────────────

#[test]
fn extract_error_detail_envelope_returns_inner_message() {
    let body = r#"{"success":false,"error":"Insufficient balance"}"#;
    assert_eq!(extract_error_detail(body, 500), "Insufficient balance");
}

#[test]
fn extract_error_detail_envelope_trims_whitespace() {
    let body = r#"{"success":false,"error":"   Toolkit \"foo\" is not enabled   "}"#;
    assert_eq!(
        extract_error_detail(body, 500),
        "Toolkit \"foo\" is not enabled"
    );
}

#[test]
fn extract_error_detail_falls_back_for_non_json_body() {
    let body = "<html>500 internal error</html>";
    assert_eq!(extract_error_detail(body, 500), body);
}

#[test]
fn extract_error_detail_handles_empty_body() {
    assert_eq!(extract_error_detail("", 500), "<empty body>");
}

#[test]
fn extract_error_detail_truncates_long_non_json_bodies_at_char_boundary() {
    // Multi-byte UTF-8 (€ = 3 bytes). Building a string longer than `max`
    // ensures truncate_at_char_boundary backs off until it lands on a
    // valid char boundary instead of slicing inside a code point.
    let body = "€".repeat(200); // 600 bytes
    let out = extract_error_detail(&body, 50);
    assert!(out.ends_with('…'), "expected ellipsis, got: {out}");
    // Hard cap check: the returned string MUST NOT exceed `max` bytes
    // including the ellipsis. Earlier the helper appended `…` after
    // slicing to `max`, which leaked 3 bytes past the advertised cap;
    // CR flagged this. Now the cap is strict.
    assert!(
        out.len() <= 50,
        "output ({} bytes) exceeded advertised cap of 50",
        out.len()
    );
}

#[test]
fn extract_error_detail_with_max_below_ellipsis_returns_empty() {
    // Edge case: when `max` is smaller than the ellipsis byte length
    // (3 bytes), there's no room for any content + ellipsis, so the
    // helper must return an empty string rather than panic or emit a
    // partial codepoint.
    let body = "€".repeat(10);
    assert_eq!(extract_error_detail(&body, 2), "");
}

#[test]
fn extract_error_detail_envelope_missing_error_field_falls_back() {
    let body = r#"{"success":false}"#;
    // No `error` key — fall back to truncated raw body so the caller
    // still has *something* to grep for.
    assert_eq!(extract_error_detail(body, 500), body);
}

#[test]
fn extract_error_detail_envelope_blank_error_falls_back() {
    let body = r#"{"success":false,"error":"   "}"#;
    assert_eq!(extract_error_detail(body, 500), body);
}

// ── Integration: HTTP error propagation through `post`/`get` ──────

async fn start_mock_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn client_for(base: String) -> IntegrationClient {
    IntegrationClient::new(base, "test-token".into())
}

#[tokio::test]
async fn post_400_propagates_backend_error_envelope_message() {
    // Mirror the real backend BadRequestError shape from
    // `backend-openhuman/src/middlewares/errorHandler.ts` — the 400
    // body is JSON `{ success:false, error:"<msg>" }`.
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|| async {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "success": false, "error": "Insufficient balance" })),
            )
                .into_response()
        }),
    );
    let base = start_mock_backend(app).await;
    let client = client_for(base);
    let err = client
        .post::<serde_json::Value>(
            "/agent-integrations/composio/execute",
            &json!({ "tool": "GMAIL_FETCH_EMAILS" }),
        )
        .await
        .expect_err("400 must surface as Err");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("Insufficient balance"),
        "expected backend error in propagated message, got: {msg}"
    );
    assert!(msg.contains("400"), "expected status code, got: {msg}");
}

#[tokio::test]
async fn post_500_propagates_html_body_truncated() {
    let app = Router::new().route(
        "/foo",
        post(|| async {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "<html>upstream blew up</html>",
            )
                .into_response()
        }),
    );
    let base = start_mock_backend(app).await;
    let client = client_for(base);
    let err = client
        .post::<serde_json::Value>("/foo", &json!({}))
        .await
        .expect_err("500 must surface as Err");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("upstream blew up"),
        "expected raw body in propagated message, got: {msg}"
    );
}

#[tokio::test]
async fn get_403_propagates_backend_error_envelope_message() {
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            (
                StatusCode::FORBIDDEN,
                Json(json!({ "success": false, "error": "Toolkit \"x\" is not enabled" })),
            )
                .into_response()
        }),
    );
    let base = start_mock_backend(app).await;
    let client = client_for(base);
    let err = client
        .get::<serde_json::Value>("/agent-integrations/composio/connections")
        .await
        .expect_err("403 must surface as Err");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("Toolkit \"x\" is not enabled"),
        "expected backend error in propagated message, got: {msg}"
    );
    assert!(msg.contains("403"), "expected status code, got: {msg}");
}

// ── OPENHUMAN-TAURI-BC regression: wire format pins to classifier ─

/// Regression guard for OPENHUMAN-TAURI-BC: the exact bail message
/// `IntegrationClient::post` builds for a 4xx user-input failure must
/// classify as `BackendUserError` so the observability layer routes
/// the report through a warn breadcrumb instead of a Sentry event.
///
/// If the format string in `client.rs` drifts away from the prefix
/// `is_backend_user_error_message` matches on, every Composio /
/// integrations 4xx will start spamming Sentry again — exactly the
/// regression this guards.
#[tokio::test]
async fn post_400_user_input_failure_classifies_as_backend_user_error() {
    use crate::core::observability::{expected_error_kind, ExpectedErrorKind};

    let app = Router::new().route(
        "/agent-integrations/composio/authorize",
        post(|| async {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "error": "Composio authorization failed: 400 {\"error\":{\"message\":\"Missing required fields: Tenant Name\",\"slug\":\"ConnectedAccount_MissingRequiredFields\",\"status\":400}}"
                })),
            )
                .into_response()
        }),
    );
    let base = start_mock_backend(app).await;
    let client = client_for(base);
    let err = client
        .post::<serde_json::Value>(
            "/agent-integrations/composio/authorize",
            &json!({ "toolkit": "sharepoint" }),
        )
        .await
        .expect_err("400 must surface as Err");
    let msg = format!("{err:#}");

    // The propagated message must still match the classifier — both the
    // `IntegrationClient::post` bail string and the
    // `observability::report_error_or_expected` argument share the same
    // shape, so this is a tight pin against drift on either side.
    //
    // After #1472 wave E added `ProviderUserState` (which matches
    // `"missing required fields"` regardless of HTTP status), the
    // SharePoint shape now lands in the more specific bucket. Either
    // expected-kind silences Sentry; assert the new tighter bucket so
    // a regression in the precedence ordering surfaces here.
    assert_eq!(
        expected_error_kind(&msg),
        Some(ExpectedErrorKind::ProviderUserState),
        "OPENHUMAN-TAURI-BC: propagated 400 must classify as ProviderUserState (more \
         specific than BackendUserError, takes precedence per #1472 wave E); got: {msg}"
    );
}

/// Counterpart: a 5xx must remain actionable. If the classifier ever
/// over-reaches and silences 5xx, this test catches it before users do.
#[tokio::test]
async fn post_500_remains_actionable() {
    use crate::core::observability::expected_error_kind;

    let app = Router::new().route(
        "/foo",
        post(|| async {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "<html>upstream blew up</html>",
            )
                .into_response()
        }),
    );
    let base = start_mock_backend(app).await;
    let client = client_for(base);
    let err = client
        .post::<serde_json::Value>("/foo", &json!({}))
        .await
        .expect_err("500 must surface as Err");
    let msg = format!("{err:#}");
    assert_eq!(
        expected_error_kind(&msg),
        None,
        "5xx must remain actionable, not classified as expected; got: {msg}"
    );
}

// ── Jira subdomain / ConnectedAccount_MissingRequiredFields (issue#1702) ─

/// The Jira authorization flow requires an Atlassian subdomain ("Tenant
/// Name"). When the user submits the form without it, Composio returns a
/// `ConnectedAccount_MissingRequiredFields` error. The error must:
///   1. Propagate through `IntegrationClient::post` so the RPC layer can
///      surface it to the UI (not silently swallowed).
///   2. Classify as `BackendUserError` so the observability layer demotes
///      it from a Sentry event to a warn breadcrumb — this is an expected
///      user-input failure, not a product bug.
///
/// The first assertion locks in the error string; the second pins the
/// classifier to `BackendUserError` so future changes to either side
/// (format string in `client.rs` or classifier in `observability.rs`)
/// are caught at review rather than in production.
#[tokio::test]
async fn jira_missing_subdomain_error_propagates_and_classifies_as_user_error() {
    use crate::core::observability::{expected_error_kind, ExpectedErrorKind};

    let app = Router::new().route(
        "/agent-integrations/composio/authorize",
        post(|| async {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "error": "Composio authorization failed: 400 {\"error\":{\"message\":\"Missing required fields: Tenant Name\",\"slug\":\"ConnectedAccount_MissingRequiredFields\",\"status\":400}}"
                })),
            )
                .into_response()
        }),
    );
    let base = start_mock_backend(app).await;
    let client = client_for(base);
    let err = client
        .post::<serde_json::Value>(
            "/agent-integrations/composio/authorize",
            &json!({ "toolkit": "jira" }),
        )
        .await
        .expect_err("Jira missing-subdomain must surface as Err");
    let msg = format!("{err:#}");

    // 1. The error string from the Composio payload must propagate so the
    //    UI can show "Missing required fields: Tenant Name" in the connect
    //    form and prompt for the Atlassian subdomain.
    assert!(
        msg.contains("Tenant Name") || msg.contains("ConnectedAccount_MissingRequiredFields"),
        "Jira missing-subdomain error must propagate; got: {msg}"
    );

    // 2. The classifier must route this as an expected user-input failure —
    //    not a Sentry-reportable product error. After #1472 wave E added the
    //    `ProviderUserState` bucket (which anchors on
    //    `"missing required fields"` regardless of HTTP status, so it also
    //    catches the 500-wrapped composio variant), the Jira missing-subdomain
    //    shape lands there rather than in the generic `BackendUserError`
    //    bucket. Either expected-kind silences Sentry — assert the tighter
    //    bucket so a regression in the precedence ordering surfaces here.
    assert_eq!(
        expected_error_kind(&msg),
        Some(ExpectedErrorKind::ProviderUserState),
        "Jira ConnectedAccount_MissingRequiredFields must classify as ProviderUserState \
         (more specific than BackendUserError per #1472 wave E); got: {msg}"
    );
}

/// Complementary: a Jira 400 where the slug is *not*
/// `ConnectedAccount_MissingRequiredFields` (e.g. a token revocation)
/// must still classify as `BackendUserError` via the outer 400 shape —
/// not as an unexpected error that would create Sentry noise.
#[tokio::test]
async fn jira_generic_400_classifies_as_backend_user_error() {
    use crate::core::observability::{expected_error_kind, ExpectedErrorKind};

    let app = Router::new().route(
        "/agent-integrations/composio/authorize",
        post(|| async {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "error": "Composio authorization failed: 400 {\"error\":{\"message\":\"Invalid subdomain\",\"slug\":\"ConnectedAccount_InvalidSubdomain\",\"status\":400}}"
                })),
            )
                .into_response()
        }),
    );
    let base = start_mock_backend(app).await;
    let client = client_for(base);
    let err = client
        .post::<serde_json::Value>(
            "/agent-integrations/composio/authorize",
            &json!({ "toolkit": "jira" }),
        )
        .await
        .expect_err("400 must surface as Err");
    let msg = format!("{err:#}");
    assert_eq!(
        expected_error_kind(&msg),
        Some(ExpectedErrorKind::BackendUserError),
        "Jira generic 400 must classify as BackendUserError; got: {msg}"
    );
}
