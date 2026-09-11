use super::*;

#[test]
fn context_scout_skips_role_contract_suffix() {
    // The scout owns its own [context_bundle] output contract; the generic
    // result contract must not be appended (it conflicts).
    let base = "scout prompt body".to_string();
    let out = append_subagent_role_contract(base.clone(), "context_scout");
    assert_eq!(out, base);
    assert!(!out.contains("Sub-agent Result Contract"));
}

#[test]
fn other_agents_get_role_contract_suffix() {
    let out = append_subagent_role_contract("body".to_string(), "researcher");
    assert!(out.contains("Sub-agent Result Contract"));
    assert!(out.contains("Recommended next step"));
}
