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

// ── Unit: local-only egress enforcement (privacy epic S7, #4441) ──

#[test]
fn backend_egress_descriptor_strips_query_and_targets_backend() {
    let desc = backend_egress_descriptor("/agent-integrations/composio/execute?foo=bar");
    assert_eq!(
        desc.reason,
        crate::openhuman::security::EgressReason::Integration
    );
    assert_eq!(desc.provider_slug, "openhuman_backend");
    // Query stripped — only the endpoint is disclosed, never carried data.
    assert_eq!(desc.service, "/agent-integrations/composio/execute");
    assert!(desc.is_external);
}

#[test]
fn enforce_backend_egress_blocks_user_data_but_allows_control_plane() {
    use crate::openhuman::config::PrivacyMode;
    use crate::openhuman::security::live_policy::test_privacy_scope;
    {
        // Thread-scoped LocalOnly — no process-global mutation, no cross-test race.
        let _mode = test_privacy_scope(PrivacyMode::LocalOnly);
        // User-data tool path → refused.
        let blocked = enforce_backend_egress("/agent-integrations/composio/execute");
        assert!(blocked.is_err());
        assert!(blocked
            .unwrap_err()
            .to_string()
            .contains("Local-only privacy mode is active"));
        // Control-plane (connection-management + pricing) → allowed even under LocalOnly.
        enforce_backend_egress("/agent-integrations/composio/connections")
            .expect("connections is control-plane");
        enforce_backend_egress("/agent-integrations/pricing").expect("pricing is control-plane");
    }

    // Standard mode → everything allowed.
    let _mode = test_privacy_scope(PrivacyMode::Standard);
    enforce_backend_egress("/agent-integrations/composio/execute").expect("Standard allows");
}

/// Privacy epic S7 (#4441): the LocalOnly gate must fire through the PUBLIC verb
/// methods, not only via the `enforce_backend_egress` helper. Each of the six
/// verbs (`post`/`get`/`patch`/`delete`/`upload_multipart`/`get_bytes`) runs the
/// gate synchronously on first poll — before any `.await` — so a user-data path
/// is refused before the request leaves the device. The client points at an
/// unreachable address, so a leaked call would surface a transport error, not
/// the local-only message; asserting the policy string therefore proves the
/// short-circuit fired end-to-end through the verb, not just the helper.
#[tokio::test]
async fn verb_methods_block_user_data_egress_under_local_only() {
    use crate::openhuman::config::PrivacyMode;
    use crate::openhuman::security::live_policy::test_privacy_scope;

    let _mode = test_privacy_scope(PrivacyMode::LocalOnly);
    let client = client_for("http://127.0.0.1:0".into());
    let path = "/agent-integrations/composio/execute"; // user-data egress (blocked)
    let body = json!({ "arguments": { "secret": "leak-me" } });

    let assert_blocked = |verb: &str, msg: String| {
        assert!(
            msg.contains("Local-only privacy mode is active"),
            "{verb}: expected a local-only block before network, got: {msg}"
        );
    };

    assert_blocked(
        "post",
        client
            .post::<serde_json::Value>(path, &body)
            .await
            .unwrap_err()
            .to_string(),
    );
    assert_blocked(
        "get",
        client
            .get::<serde_json::Value>(path)
            .await
            .unwrap_err()
            .to_string(),
    );
    assert_blocked(
        "patch",
        client
            .patch::<serde_json::Value>(path, &body)
            .await
            .unwrap_err()
            .to_string(),
    );
    assert_blocked(
        "delete",
        client
            .delete::<serde_json::Value>(path)
            .await
            .unwrap_err()
            .to_string(),
    );
    assert_blocked(
        "upload_multipart",
        client
            .upload_multipart::<serde_json::Value>(path, reqwest::multipart::Form::new())
            .await
            .unwrap_err()
            .to_string(),
    );
    assert_blocked(
        "get_bytes",
        client.get_bytes(path).await.unwrap_err().to_string(),
    );
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
async fn local_only_rejects_user_data_verb_before_transport() {
    // End-to-end proof (privacy epic S7, #4441) that a PUBLIC verb
    // (`post`/`get`) — not just the private `enforce_backend_egress` helper —
    // honours LocalOnly and refuses a user-data backend call BEFORE it hits
    // transport. The mock 403s on every route it actually receives, so if the
    // gate short-circuits first the error is the policy message, never the
    // mock body.
    use crate::openhuman::config::PrivacyMode;
    use crate::openhuman::security::live_policy::test_privacy_scope;

    let app = Router::new()
        .route(
            "/agent-integrations/composio/execute",
            post(|| async {
                (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "success": false, "error": "mock must not be reached" })),
                )
                    .into_response()
            }),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "success": false, "error": "control-plane reached transport" })),
                )
                    .into_response()
            }),
        );
    let base = start_mock_backend(app).await;
    let client = client_for(base);

    // `#[tokio::test]` runs on a current-thread runtime, so the gate's inline
    // `current_privacy_mode()` read observes this override on the same thread
    // (see `TEST_PRIVACY_MODE`).
    let _mode = test_privacy_scope(PrivacyMode::LocalOnly);

    // (1) User-data verb call is refused by the gate BEFORE transport — the
    //     error carries the policy message and NOT the mock's 403 body.
    let err = client
        .post::<serde_json::Value>(
            "/agent-integrations/composio/execute",
            &json!({ "tool": "GMAIL_FETCH_EMAILS" }),
        )
        .await
        .expect_err("LocalOnly must block the user-data POST before transport");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("Local-only privacy mode is active"),
        "expected policy block, got: {msg}"
    );
    assert!(
        !msg.contains("mock must not be reached"),
        "gate must short-circuit before the request reaches the mock: {msg}"
    );

    // (2) A control-plane verb call under the SAME LocalOnly scope passes the
    //     gate and DOES reach transport (surfaces the mock's 403) — proving the
    //     gate is selective, not a blanket network kill.
    let err = client
        .get::<serde_json::Value>("/agent-integrations/composio/connections")
        .await
        .expect_err("control-plane reaches transport and surfaces the mock 403");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("control-plane reached transport"),
        "control-plane must reach transport under LocalOnly, got: {msg}"
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

// ── TAURI-RUST-84E: session-JWT 401 → session-expiry recovery ─────

/// Root-cause regression guard for TAURI-RUST-84E. A `401 Unauthorized` from
/// the OpenHuman backend's `/agent-integrations/*` routes is the backend
/// rejecting our app-session JWT. The propagated error string MUST:
///   1. classify as `SessionExpired` (so it stays demoted from Sentry — the
///      noise suppression the prior fix established), AND
///   2. be recognised by `is_session_expired_message` (the same predicate the
///      JSON-RPC dispatcher uses to publish `DomainEvent::SessionExpired` and
///      drive re-login).
///
/// This couples the exact wire string `IntegrationClient::post`/`get` builds
/// for a session-JWT 401 to the session-expiry classifier — if either side
/// drifts, web_search (and every other backend-proxied tool) would silently
/// stop driving re-login on session expiry, leaving the user stuck behind an
/// opaque "parallel search failed: Invalid token" with no sign-in nudge.
#[tokio::test]
async fn post_401_session_jwt_classifies_as_session_expired() {
    use crate::core::observability::{
        expected_error_kind, is_session_expired_message, ExpectedErrorKind,
    };

    // Canonical TAURI-RUST-84E shape: backend auth middleware rejects the
    // session JWT with `401 {"error":"Invalid token"}`.
    let app = Router::new().route(
        "/agent-integrations/parallel/search",
        post(|| async {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "success": false, "error": "Invalid token" })),
            )
                .into_response()
        }),
    );
    let base = start_mock_backend(app).await;
    let client = client_for(base);
    let err = client
        .post::<serde_json::Value>(
            "/agent-integrations/parallel/search",
            &json!({ "objective": "x" }),
        )
        .await
        .expect_err("401 must surface as Err");
    let msg = format!("{err:#}");

    // The propagated message carries the SESSION_EXPIRED sentinel so the model
    // (and any caller that wraps it, e.g. `parallel search failed: {e:#}`)
    // sees an actionable "sign in again" instruction.
    assert!(
        msg.contains("SESSION_EXPIRED"),
        "session-JWT 401 must carry the SESSION_EXPIRED sentinel; got: {msg}"
    );

    // 1. Demotes from Sentry: classifies as SessionExpired (expected kind),
    //    not a hard error.
    assert_eq!(
        expected_error_kind(&msg),
        Some(ExpectedErrorKind::SessionExpired),
        "session-JWT 401 must classify as SessionExpired (stays demoted from Sentry); got: {msg}"
    );

    // 2. Recognised by the session-expiry predicate that the JSON-RPC
    //    dispatcher keys `DomainEvent::SessionExpired` publication on — so the
    //    re-login flow fires (the client also publishes it directly to cover
    //    the swallowing agent loop, but this keeps the propagation path
    //    correct too).
    assert!(
        is_session_expired_message(&msg),
        "session-JWT 401 must be recognised by is_session_expired_message so re-login \
         fires; got: {msg}"
    );
}

/// GET counterpart — the connections/toolkits/pricing reads must drive the
/// same session-expiry recovery on a session-JWT 401.
#[tokio::test]
async fn get_401_session_jwt_classifies_as_session_expired() {
    use crate::core::observability::{
        expected_error_kind, is_session_expired_message, ExpectedErrorKind,
    };

    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "success": false, "error": "Invalid token" })),
            )
                .into_response()
        }),
    );
    let base = start_mock_backend(app).await;
    let client = client_for(base);
    let err = client
        .get::<serde_json::Value>("/agent-integrations/composio/connections")
        .await
        .expect_err("401 must surface as Err");
    let msg = format!("{err:#}");

    assert_eq!(
        expected_error_kind(&msg),
        Some(ExpectedErrorKind::SessionExpired),
        "GET session-JWT 401 must classify as SessionExpired; got: {msg}"
    );
    assert!(
        is_session_expired_message(&msg),
        "GET session-JWT 401 must be recognised by is_session_expired_message; got: {msg}"
    );
}

/// Negative guard (task #3 — the key correctness risk): a NON-401 4xx must NOT
/// be turned into session-expiry. A 403 (authz/scope rejection on a
/// backend-mediated resource) or a 400 (user-input failure) must keep the
/// generic `BackendUserError` / `ProviderUserState` classification so we never
/// log the user out for an unrelated integration problem. If the 401 arm ever
/// widened to swallow other statuses, this catches it.
#[tokio::test]
async fn non_401_4xx_does_not_classify_as_session_expired() {
    use crate::core::observability::{expected_error_kind, is_session_expired_message};

    // 403 — e.g. an authz/scope rejection on a backend-mediated provider
    // resource. This is NOT a dead session; it must stay a generic backend
    // user-error and must NOT publish SessionExpired.
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            (
                StatusCode::FORBIDDEN,
                Json(json!({ "success": false, "error": "forbidden" })),
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
        !msg.contains("SESSION_EXPIRED"),
        "403 must NOT be turned into a session-expiry sentinel; got: {msg}"
    );
    assert!(
        !is_session_expired_message(&msg),
        "403 must NOT be recognised as session expiry (would log the user out for an \
         unrelated integration authz problem); got: {msg}"
    );
    // It still demotes from Sentry as a generic backend user-error (the prior
    // behaviour), just not as session-expiry.
    assert!(
        expected_error_kind(&msg).is_some(),
        "403 should still classify as an expected backend user-error; got: {msg}"
    );
    assert_ne!(
        expected_error_kind(&msg),
        Some(crate::core::observability::ExpectedErrorKind::SessionExpired),
        "403 must NOT classify as SessionExpired; got: {msg}"
    );
}

// ── Composio "soft" auth path (#4281) ─────────────────────────────

/// The trigger-catalog reads are the only routes treated as "soft" (sentinel
/// surfaced for an in-place CTA, no global sign-out). Every other backend route
/// stays authoritative — a 401 there drives the full re-login.
#[test]
fn composio_soft_auth_path_covers_only_trigger_reads() {
    use super::is_composio_soft_auth_path;
    // Soft: GET available catalog (with and without query), GET active-triggers list.
    assert!(is_composio_soft_auth_path(
        "GET",
        "/agent-integrations/composio/triggers/available"
    ));
    assert!(is_composio_soft_auth_path(
        "GET",
        "/agent-integrations/composio/triggers/available?toolkit=gmail&connectionId=ca_x"
    ));
    assert!(is_composio_soft_auth_path(
        "GET",
        "/agent-integrations/composio/triggers"
    ));
    // GET active-triggers list with a `toolkit` query (the `?` boundary).
    assert!(is_composio_soft_auth_path(
        "GET",
        "/agent-integrations/composio/triggers?toolkit=gmail"
    ));
    // Path-boundary guard: a bare prefix must NOT match an unrelated route
    // that merely begins with "triggers" (CodeRabbit catch).
    assert!(!is_composio_soft_auth_path(
        "GET",
        "/agent-integrations/composio/triggersXYZ"
    ));
    // Authoritative — a 401 here must still log the user out:
    // trigger WRITES (enable/disable/create POST to the same path)…
    assert!(!is_composio_soft_auth_path(
        "POST",
        "/agent-integrations/composio/triggers"
    ));
    // …and every non-trigger backend route.
    assert!(!is_composio_soft_auth_path(
        "GET",
        "/agent-integrations/composio/connections"
    ));
    assert!(!is_composio_soft_auth_path(
        "GET",
        "/agent-integrations/composio/toolkits"
    ));
    assert!(!is_composio_soft_auth_path(
        "POST",
        "/agent-integrations/composio/execute"
    ));
    assert!(!is_composio_soft_auth_path(
        "POST",
        "/agent-integrations/parallel/search"
    ));
}

/// A 401 on the trigger-catalog read still carries the `SESSION_EXPIRED`
/// sentinel — the trigger panel classifies it (`CoreRpcError.kind ===
/// 'auth_expired'`) and renders the in-place "Sign in again" CTA. The
/// difference from the authoritative arm (no global `SessionExpired` publish)
/// is asserted via the path gate above; here we lock the wire contract the UI
/// depends on so a future refactor can't silently drop the sentinel and leave
/// the panel with no actionable error (#4281).
#[tokio::test]
async fn get_401_composio_triggers_keeps_sentinel_for_in_place_cta() {
    use crate::core::observability::{
        expected_error_kind, is_session_expired_message, ExpectedErrorKind,
    };

    let app = Router::new().route(
        "/agent-integrations/composio/triggers/available",
        get(|| async {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "success": false, "error": "Invalid token" })),
            )
                .into_response()
        }),
    );
    let base = start_mock_backend(app).await;
    let client = client_for(base);
    let err = client
        .get::<serde_json::Value>("/agent-integrations/composio/triggers/available?toolkit=gmail")
        .await
        .expect_err("401 must surface as Err");
    let msg = format!("{err:#}");

    assert!(
        msg.contains("SESSION_EXPIRED"),
        "triggers 401 must still carry the sentinel so the panel shows the CTA; got: {msg}"
    );
    assert_eq!(
        expected_error_kind(&msg),
        Some(ExpectedErrorKind::SessionExpired),
        "triggers 401 must still classify as SessionExpired (stays demoted from Sentry); got: {msg}"
    );
    assert!(
        is_session_expired_message(&msg),
        "triggers 401 must stay recognisable as session expiry for the frontend classifier; got: {msg}"
    );
}

// ── Unit: `sanitize_backend_url` (issue #2075) ────────────────────

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
