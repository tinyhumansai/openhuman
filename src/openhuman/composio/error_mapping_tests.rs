use super::{
    classify_composio_error, derive_toolkit_slug, format_provider_error, remap_transport_error,
    ComposioErrorClass,
};

// ── derive_toolkit_slug (issue #2913 nitpick — shared slug extraction) ──

#[test]
fn derive_toolkit_slug_extracts_leading_segment_lowercased() {
    assert_eq!(derive_toolkit_slug("GMAIL_NEW_GMAIL_MESSAGE"), "gmail");
}

#[test]
fn derive_toolkit_slug_single_segment_is_lowercased() {
    assert_eq!(derive_toolkit_slug("SLACK"), "slack");
}

#[test]
fn derive_toolkit_slug_empty_input_returns_empty_not_fallback() {
    // Behavior-parity guard: `"".split('_').next()` yields `Some("")`, so the
    // `unwrap_or("integration")` fallback does NOT apply — preserve that exactly.
    assert_eq!(derive_toolkit_slug(""), "");
}

#[test]
fn classifies_gmail_insufficient_scope() {
    let msg = "HTTP 403: Request had insufficient authentication scopes.";
    assert_eq!(
        classify_composio_error("GMAIL_FETCH_EMAILS", msg),
        ComposioErrorClass::InsufficientScope
    );
}

#[test]
fn formats_gmail_insufficient_scope_as_missing_permissions_not_disconnected() {
    let mapped = format_provider_error(
        "GMAIL_SEND_EMAIL",
        "HTTP 403: Request had insufficient authentication scopes.",
    );
    assert!(mapped.contains("[composio:error:insufficient_scope]"));
    assert!(mapped.contains("connected gmail account is missing required permissions"));
    assert!(mapped.contains("Settings"));
    assert!(mapped.contains("Connections"));
    assert!(mapped.contains("gmail"));
    assert!(!mapped.contains("not connected"));
    assert!(!mapped.contains("Settings → Skills"));
}

#[test]
fn classifies_slack_rate_limit() {
    let msg = "Slack API error: ratelimited";
    assert_eq!(
        classify_composio_error("SLACK_FETCH_CONVERSATION_HISTORY", msg),
        ComposioErrorClass::RateLimited
    );
}

#[test]
fn embedded_provider_failure_in_502_body_is_not_gateway() {
    let raw = "Backend returned 502 Bad Gateway for POST https://api.example.com/agent-integrations/composio/execute: \
               timeMax must be RFC 3339 timestamp";
    let mapped = remap_transport_error("GOOGLECALENDAR_EVENTS_LIST", raw);
    assert!(
        mapped.contains("[composio:error:"),
        "expected classified prefix, got: {mapped}"
    );
    assert!(
        !mapped.contains("[composio:error:gateway]"),
        "provider-shaped 502 body must not be labeled gateway: {mapped}"
    );
}

#[test]
fn true_gateway_stays_gateway_class() {
    let raw = "Backend returned 502 Bad Gateway for POST https://api.example.com/x: upstream down";
    let mapped = remap_transport_error("GMAIL_SEND_EMAIL", raw);
    assert!(
        mapped.contains("[composio:error:gateway]"),
        "expected gateway class, got: {mapped}"
    );
}

// ── HTTP 404/410 action-not-found vs auth (issue #3219) ────────────────

#[test]
fn classifies_http_404_with_auth_phrase_as_action_not_found_not_platform() {
    // The exact #3219 shape: a 404 whose body carries the misleading
    // "connection error, try to authenticate" phrase. Status must win.
    let msg = "HTTP 404: connection error, try to authenticate";
    assert_eq!(
        classify_composio_error("GMAIL_SEND_EMAIL", msg),
        ComposioErrorClass::ActionNotFound
    );
}

#[test]
fn classifies_http_410_gone_as_action_not_found() {
    let msg = "HTTP 410: This endpoint is deprecated";
    assert_eq!(
        classify_composio_error("GOOGLEDOCS_UPDATE_DOCUMENT", msg),
        ComposioErrorClass::ActionNotFound
    );
}

#[test]
fn action_not_found_message_does_not_recommend_reauth() {
    let mapped = format_provider_error(
        "GMAIL_SEND_EMAIL",
        "HTTP 404: connection error, try to authenticate",
    );
    let lower = mapped.to_lowercase();
    assert!(mapped.contains("[composio:error:action_not_found]"));
    assert!(lower.contains("still connected"));
    // Must NOT echo the misleading provider phrase or nudge re-auth/reconnect.
    assert!(
        !lower.contains("authenticate"),
        "must not surface the re-auth phrase: {mapped}"
    );
    assert!(
        !lower.contains("reconnect"),
        "must not tell the user to reconnect a healthy connection: {mapped}"
    );
    assert!(
        !mapped.contains("Settings → Connections"),
        "must not show the re-auth CTA: {mapped}"
    );
}

#[test]
fn genuine_auth_phrase_without_http_status_stays_platform() {
    // Same phrase, but with NO 4xx status — a real platform/connection issue
    // must still classify as ComposioPlatform.
    let msg = "connection error, try to authenticate";
    assert_eq!(
        classify_composio_error("GMAIL_SEND_EMAIL", msg),
        ComposioErrorClass::ComposioPlatform
    );
}

#[test]
fn wrapped_v3_v2_404_fallback_classifies_as_action_not_found() {
    // The real transport string after both v3 and v2 fail — the wrapped form
    // produced by `execute_action`, fed through `remap_transport_error`.
    let raw = "Composio execute failed on v3 (Composio v3 action execution failed: \
               HTTP 404: connection error, try to authenticate) and v2 fallback \
               (Composio v2 action execution failed: HTTP 410: Gone)";
    let mapped = remap_transport_error("GMAIL_SEND_EMAIL", raw);
    assert!(
        mapped.contains("[composio:error:action_not_found]"),
        "wrapped 404/410 must map to action_not_found, got: {mapped}"
    );
    assert!(
        !mapped.to_lowercase().contains("authenticate"),
        "wrapped path must not surface the re-auth phrase: {mapped}"
    );
}

// ── Trigger-permission denial (issue #2913) ───────────────────────────

#[test]
fn classifies_trigger_permission_from_403_without_scope() {
    // The backend 403 body does NOT contain the word "scope", so it must be
    // classified as TriggerPermission rather than InsufficientScope or Other.
    let raw = "Backend returned 403 Forbidden for POST \
               https://api.example.com/agent-integrations/composio/triggers: \
               You do not have permission to enable triggers on this connection";
    assert_eq!(
        classify_composio_error("GMAIL_NEW_GMAIL_MESSAGE", raw),
        ComposioErrorClass::TriggerPermission
    );
}

#[test]
fn trigger_permission_is_not_classified_as_insufficient_scope() {
    let raw = "403 Forbidden: You do not have permission to enable triggers on this connection";
    // Regression guard: the scope heuristic requires the literal "scope" token,
    // which this message lacks — so it must not be InsufficientScope.
    assert_ne!(
        classify_composio_error("GMAIL_NEW_GMAIL_MESSAGE", raw),
        ComposioErrorClass::InsufficientScope
    );
}

#[test]
fn formats_trigger_permission_as_actionable_reconnect_guidance() {
    let raw = "Backend returned 403 Forbidden for POST \
               https://api.example.com/agent-integrations/composio/triggers: \
               You do not have permission to enable triggers on this connection";
    let mapped = format_provider_error("GMAIL_NEW_GMAIL_MESSAGE", raw);
    assert!(
        mapped.contains("[composio:error:trigger_permission]"),
        "expected trigger_permission class prefix, got: {mapped}"
    );
    // Branded, actionable copy that points the user at reconnecting.
    assert!(
        mapped.contains("gmail"),
        "expected toolkit branding: {mapped}"
    );
    assert!(
        mapped.contains("Settings"),
        "expected reconnect guidance: {mapped}"
    );
    assert!(
        mapped.contains("Connections"),
        "expected reconnect guidance: {mapped}"
    );
    assert!(
        mapped.to_lowercase().contains("permission"),
        "expected permission wording: {mapped}"
    );
    // Must not leak the raw backend blob as the message.
    assert!(
        !mapped.contains("Backend returned 403"),
        "raw backend blob leaked: {mapped}"
    );
}

#[test]
fn generic_403_without_trigger_context_is_not_trigger_permission() {
    // A 403 with no "trigger" context must not be miscategorised.
    let raw = "403 Forbidden: you do not have permission to read this file";
    assert_ne!(
        classify_composio_error("GMAIL_FETCH_EMAILS", raw),
        ComposioErrorClass::TriggerPermission
    );
}
