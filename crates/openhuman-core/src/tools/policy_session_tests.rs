use super::*;

#[test]
fn empty_tool_set_yields_an_unrestricted_session() {
    let session = build_policy_session(
        "orchestrator",
        "web",
        "session",
        &HashMap::new(),
        &[],
        &HashSet::new(),
    );
    assert!(!session.has_restrictions());
    assert!(session.allowed_tool_names.is_empty());
    assert_eq!(session.profile.agent_id, "orchestrator");
    assert_eq!(session.profile.channel, "web");
}

#[test]
fn matches_the_engine_for_a_restricted_channel() {
    let mut perms = HashMap::new();
    perms.insert("web".to_string(), "readonly".to_string());
    let ours = build_policy_session("a", "web", "session", &perms, &[], &HashSet::new());
    let engine = ToolPolicyEngine::build_session_from_refs(
        "a",
        "web",
        "session",
        &perms,
        &[],
        &HashSet::new(),
    );
    assert_eq!(
        ours.profile.allowed_permission,
        engine.profile.allowed_permission
    );
}
