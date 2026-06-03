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

#[test]
fn managed_budget_gate_applies_to_agent_integration_paths() {
    assert!(managed_budget_applies_to_path(
        "/agent-integrations/composio/execute"
    ));
    assert!(managed_budget_applies_to_path(
        "/agent-integrations/parallel/search"
    ));
    assert!(!managed_budget_applies_to_path(
        "/agent-integrations/pricing"
    ));
    assert!(!managed_budget_applies_to_path("/teams/me/usage"));
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

// ── Unit: `sanitize_backend_url` (issue #2075) ────────────────────

// ── TAURI-RUST-5KG: typed BackendUserStateError boundary ────────────
//
// 1860 Sentry events / 9 users from `web_search_tool` → backend 400
// "Insufficient balance". The integrations breadcrumb path already
// demoted the event, but the per-call error bubbled up as a flat
// `anyhow::Error` and the agent's tool runner re-captured it. The fix
// types the error here so the runner can `downcast_ref::<…>()` and
// route to the warn-only path. These tests pin both halves of the
// contract: (a) classify-and-wrap fires on user-state failures,
// (b) Display string is preserved for stringify-only callers, and
// (c) genuine system failures stay un-typed so capture still works.

#[tokio::test]
async fn post_400_insufficient_balance_returns_typed_backend_user_state_error() {
    let app = Router::new().route(
        "/agent-integrations/parallel/search",
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
            "/agent-integrations/parallel/search",
            &json!({ "objective": "test" }),
        )
        .await
        .expect_err("400 must surface as Err");

    // Typed: the agent tool runner relies on this exact downcast to
    // route the failure to the warn-only path instead of `report_error`.
    assert!(
        is_backend_user_state_error(&err),
        "400 'Insufficient balance' must carry BackendUserStateError marker so the \
         tool runner can route it to the warn-only path (TAURI-RUST-5KG); got: {err:#}"
    );

    // Display preserved: every caller that just stringifies the error
    // (toasts, logs, prior bail-format consumers) keeps seeing the same
    // message — typing is purely additive.
    let msg = format!("{err:#}");
    assert!(
        msg.contains("Insufficient balance"),
        "Display string must still carry the user-facing error; got: {msg}"
    );
    assert!(
        msg.contains("400"),
        "Display string must still carry the HTTP status; got: {msg}"
    );
}

#[tokio::test]
async fn post_500_internal_error_is_not_marked_user_state() {
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

    // 5xx is a real failure — must remain a plain anyhow error so
    // `report_error` runs at the tool runner and triage sees it.
    assert!(
        !is_backend_user_state_error(&err),
        "5xx must NOT carry the user-state marker — that would silence real \
         backend bugs; got: {err:#}"
    );
}

#[tokio::test]
async fn get_403_toolkit_not_enabled_returns_typed_backend_user_state_error() {
    // Composio "Toolkit X is not enabled" classifies as
    // `ProviderUserState` per the observability matcher. Pin that the
    // typed marker is attached for the entire user-state bucket family,
    // not just BackendUserError / BudgetExhausted.
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            (
                StatusCode::FORBIDDEN,
                Json(json!({ "success": false, "error": "Toolkit \"slack\" is not enabled" })),
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

    assert!(
        is_backend_user_state_error(&err),
        "Provider-user-state 403 must carry BackendUserStateError marker; got: {err:#}"
    );
    let msg = format!("{err:#}");
    assert!(
        msg.contains("Toolkit \"slack\" is not enabled"),
        "Display must preserve the actionable error; got: {msg}"
    );
}

#[tokio::test]
async fn post_envelope_user_state_failure_returns_typed_backend_user_state_error() {
    // 2xx + `success: false` user-state envelope failure (composio
    // "Toolkit X is not enabled" wire shape on the 2xx path). The
    // envelope-error branch must wrap with the typed marker too —
    // otherwise the runner re-captures it on the next tool call.
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|| async {
            (
                StatusCode::OK,
                Json(json!({
                    "success": false,
                    "error": "Toolkit \"slack\" is not enabled"
                })),
            )
                .into_response()
        }),
    );
    let base = start_mock_backend(app).await;
    let client = client_for(base);
    let err = client
        .post::<serde_json::Value>("/agent-integrations/composio/execute", &json!({}))
        .await
        .expect_err("envelope-failure must surface as Err");

    assert!(
        is_backend_user_state_error(&err),
        "envelope user-state failure must carry BackendUserStateError marker; \
         got: {err:#}"
    );
}

#[test]
fn backend_user_state_error_display_is_message_verbatim() {
    let typed = BackendUserStateError {
        message: "Backend returned 400 Bad Request for POST x: Insufficient balance".into(),
    };
    // The Display impl is the single source of truth for what callers
    // see; an `anyhow::Error::new(typed)` rendering must match this
    // exactly so the existing bail-format contract holds.
    assert_eq!(
        typed.to_string(),
        "Backend returned 400 Bad Request for POST x: Insufficient balance"
    );
}

#[test]
fn is_backend_user_state_error_matches_wrapped_anyhow() {
    // `anyhow::Error::new(typed)` puts the typed value at the root of
    // the chain — confirm both the direct downcast and the chain walk
    // catch it (defense-in-depth against future `.context(…)` wraps).
    let typed = BackendUserStateError {
        message: "x".into(),
    };
    let err: anyhow::Error = typed.into();
    assert!(is_backend_user_state_error(&err));

    let plain = anyhow::anyhow!("not user-state");
    assert!(!is_backend_user_state_error(&plain));
}

#[test]
fn is_backend_user_state_error_finds_typed_marker_through_context_wraps() {
    // Defense-in-depth: if a caller wraps the typed error with
    // `.context("more info")`, the marker still lives in the chain.
    // `is_backend_user_state_error` must walk to find it — otherwise
    // any future `with_context` at a call site silently re-enables
    // Sentry capture for user-state failures.
    use anyhow::Context;

    let typed = BackendUserStateError {
        message: "Backend returned 400 …: Insufficient balance".into(),
    };
    let err: anyhow::Error = anyhow::Error::new(typed).context("while executing web_search_tool");
    assert!(
        is_backend_user_state_error(&err),
        "marker must be reachable after .context() wraps; got: {err:#}"
    );
}

#[test]
fn sanitize_backend_url_strips_inference_path() {
    // Regression: a misconfigured `BACKEND_URL` baked into the build
    // (`https://api.tinyhumans.ai/openai/v1/chat/completions`) used to
    // become every integration call's prefix, producing 404s such as
    // `…/openai/v1/chat/completions/agent-integrations/composio/connections`.
    let cleaned = sanitize_backend_url("https://api.tinyhumans.ai/openai/v1/chat/completions");
    assert_eq!(cleaned, "https://api.tinyhumans.ai");
}

#[test]
fn sanitize_backend_url_idempotent_on_clean_root() {
    let cleaned = sanitize_backend_url("https://api.tinyhumans.ai");
    assert_eq!(cleaned, "https://api.tinyhumans.ai");
}

#[test]
fn sanitize_backend_url_preserves_empty_input() {
    // Empty / unparseable input must round-trip unchanged so we don't
    // overwrite a caller's explicit "no backend" sentinel.
    assert_eq!(sanitize_backend_url(""), "");
}

#[test]
fn integration_client_new_strips_inference_path_from_backend_url() {
    let client = IntegrationClient::new(
        "https://api.tinyhumans.ai/openai/v1/chat/completions".to_string(),
        "token".to_string(),
    );
    assert_eq!(client.backend_url, "https://api.tinyhumans.ai");
}
