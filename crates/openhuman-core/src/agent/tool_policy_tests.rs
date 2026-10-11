use super::*;

#[tokio::test]
async fn allow_all_policy_allows_every_call() {
    let policy = AllowAllToolPolicy;
    let request = ToolPolicyRequest::new(
        "echo",
        serde_json::json!({ "value": 1 }),
        ToolCallContext::session("session", "chat", "orchestrator", "call-1", 1),
    );

    assert_eq!(policy.check(&request).await, ToolPolicyDecision::Allow);
    assert_eq!(request.context.session_id, "session");
    assert_eq!(request.context.channel, "chat");
    assert_eq!(request.context.agent_definition_id, "orchestrator");
    assert_eq!(request.context.source, ToolCallSource::Session);
    assert_eq!(request.context.call_id, "call-1");
}

#[test]
fn debug_redacts_sensitive_context_fields() {
    let request = ToolPolicyRequest::new(
        "secrets.lookup",
        serde_json::json!({ "secret": "super-secret-token" }),
        ToolCallContext::session(
            "session-secret-123",
            "private-channel",
            "orchestrator",
            "call-1",
            1,
        ),
    );

    let rendered = format!("{request:?}");
    assert!(rendered.contains("sess..."));
    assert!(rendered.contains("priv..."));
    assert!(!rendered.contains("session-secret-123"));
    assert!(!rendered.contains("private-channel"));
    assert!(!rendered.contains("super-secret-token"));
}
