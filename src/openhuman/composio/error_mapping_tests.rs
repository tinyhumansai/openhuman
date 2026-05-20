use super::{classify_composio_error, remap_transport_error, ComposioErrorClass};

#[test]
fn classifies_gmail_insufficient_scope() {
    let msg = "HTTP 403: Request had insufficient authentication scopes.";
    assert_eq!(
        classify_composio_error("GMAIL_FETCH_EMAILS", msg),
        ComposioErrorClass::InsufficientScope
    );
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
