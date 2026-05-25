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
