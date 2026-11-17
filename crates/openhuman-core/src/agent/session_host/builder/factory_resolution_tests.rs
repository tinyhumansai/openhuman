use super::*;

#[test]
fn legacy_orchestrator_fallback_defaults_to_chat() {
    assert_eq!(provider_role_for(Some("chat-v1")), "chat");
    assert_eq!(provider_role_for(None), "chat");
    // A legacy heavy default_model tier still falls through to chat.
    assert_eq!(provider_role_for(Some("reasoning-v1")), "chat");
}

#[test]
fn explicit_hints_route_to_workload() {
    assert_eq!(provider_role_for(Some("hint:agentic")), "agentic");
    assert_eq!(provider_role_for(Some("hint:reasoning")), "reasoning");
    assert_eq!(provider_role_for(Some("hint:coding")), "coding");
}
