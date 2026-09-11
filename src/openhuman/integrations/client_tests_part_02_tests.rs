use super::*;

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
    use super::super::is_composio_soft_auth_path;
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
