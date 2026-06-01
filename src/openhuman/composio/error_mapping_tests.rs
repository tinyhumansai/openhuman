use super::{
    classify_composio_error, format_provider_error, remap_transport_error, ComposioErrorClass,
};

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
