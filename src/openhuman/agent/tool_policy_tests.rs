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
    #[allow(deprecated)]
    {
        assert_eq!(request.session_id, request.context.session_id);
        assert_eq!(request.channel, request.context.channel);
        assert_eq!(
            request.agent_definition_id,
            request.context.agent_definition_id
        );
    }
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

fn generated_request() -> ToolPolicyRequest {
    ToolPolicyRequest::new(
        "email.send",
        serde_json::json!({ "to": "user@example.com" }),
        ToolCallContext::session("session", "chat", "orchestrator", "call-1", 1),
    )
    .with_generated_tool_context(GeneratedToolRuntimeContext {
        provider_id: "mail.runtime".to_string(),
        capability_id: "email.send".to_string(),
        risk: GeneratedToolRuntimeRisk::ExternalWrite,
        source_digest: Some("sha256:abc".to_string()),
        approval_id: None,
    })
}

#[tokio::test]
async fn generated_runtime_policy_allows_when_disabled() {
    let policy = GeneratedToolRuntimePolicy::new(GeneratedToolRuntimePolicyConfig::default());

    assert_eq!(
        policy.check(&generated_request()).await,
        ToolPolicyDecision::Allow
    );
}

#[tokio::test]
async fn generated_runtime_policy_allows_when_enabled_but_missing_context() {
    let policy = GeneratedToolRuntimePolicy::new(GeneratedToolRuntimePolicyConfig {
        enabled: true,
        ..Default::default()
    });

    let request = ToolPolicyRequest::new(
        "echo",
        serde_json::json!({ "value": 1 }),
        ToolCallContext::session("session", "chat", "orchestrator", "call-1", 1),
    );

    assert_eq!(policy.check(&request).await, ToolPolicyDecision::Allow);
}

#[tokio::test]
async fn generated_runtime_policy_denies_revoked_provider() {
    let policy = GeneratedToolRuntimePolicy::new(GeneratedToolRuntimePolicyConfig {
        enabled: true,
        revoked_providers: BTreeSet::from(["mail.runtime".to_string()]),
        ..Default::default()
    });

    let decision = policy.check(&generated_request()).await;
    assert!(matches!(decision, ToolPolicyDecision::Deny { .. }));
    assert!(decision.blocking_reason().unwrap().contains("revoked"));
}

#[tokio::test]
async fn generated_runtime_policy_denies_revoked_capability() {
    let policy = GeneratedToolRuntimePolicy::new(GeneratedToolRuntimePolicyConfig {
        enabled: true,
        revoked_capabilities: BTreeSet::from(["email.send".to_string()]),
        ..Default::default()
    });

    let decision = policy.check(&generated_request()).await;
    assert!(matches!(decision, ToolPolicyDecision::Deny { .. }));
    assert!(decision.blocking_reason().unwrap().contains("capability"));
}

#[tokio::test]
async fn generated_runtime_policy_requires_approval_by_risk() {
    let policy = GeneratedToolRuntimePolicy::new(GeneratedToolRuntimePolicyConfig {
        enabled: true,
        risk_actions: BTreeMap::from([(
            GeneratedToolRuntimeRisk::ExternalWrite,
            RuntimeToolPolicyAction::RequireApproval,
        )]),
        ..Default::default()
    });

    let decision = policy.check(&generated_request()).await;
    assert!(matches!(
        decision,
        ToolPolicyDecision::RequireApproval { .. }
    ));
}
