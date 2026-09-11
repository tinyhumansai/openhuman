use super::*;

#[test]
fn all_definitions_present() {
    let defs = all();
    // Count enabled built-ins, accounting for feature gates (e.g., presentation_agent only when documents is on).
    let enabled_builtins = crate::openhuman::agent::registry::agents::BUILTINS
        .iter()
        .filter(|_b| {
            #[cfg(not(feature = "documents"))]
            if _b.id == "presentation_agent" {
                return false;
            }
            true
        })
        .count();
    // +3 for the cfg(test) default parent and inherit-based test defs appended by all().
    let expected = enabled_builtins + 3;
    assert_eq!(
        defs.len(),
        expected,
        "Expected {} definitions but got {} (enabled BUILTINS={}, +3 test overrides)",
        expected,
        defs.len(),
        enabled_builtins
    );
}

#[test]
fn test_main_allows_test_inherit_workers() {
    use super::super::definition::SubagentEntry;
    let def = all()
        .into_iter()
        .find(|d| d.id == "main")
        .expect("test-only main agent must be registered in test builds");
    let allowed = def
        .subagents
        .iter()
        .filter_map(|entry| match entry {
            SubagentEntry::AgentId(id) => Some(id.as_str()),
            SubagentEntry::Skills(_) => None,
        })
        .collect::<std::collections::HashSet<_>>();
    assert!(allowed.contains("__test_inherit_echo"));
    assert!(allowed.contains("__test_inherit_parallel_worker"));
}

#[test]
fn test_inherit_echo_is_present_and_inherits() {
    use super::super::definition::ModelSpec;
    let def = all()
        .into_iter()
        .find(|d| d.id == "__test_inherit_echo")
        .expect("test-only inherit agent must be registered in test builds");
    assert!(
        matches!(def.model, ModelSpec::Inherit),
        "must be Inherit so the sub-agent uses the parent's (mock) provider"
    );
}

#[test]
fn test_inherit_parallel_worker_is_present_and_inherits() {
    use super::super::definition::{ModelSpec, ToolScope};
    let def = all()
        .into_iter()
        .find(|d| d.id == "__test_inherit_parallel_worker")
        .expect("test-only parallel worker must be registered in test builds");
    assert!(
        matches!(def.model, ModelSpec::Inherit),
        "must be Inherit so the sub-agent uses the parent's (mock) provider"
    );
    assert!(
        matches!(def.tools, ToolScope::Named(ref names) if names == &vec!["fixture_step".to_string()]),
        "parallel worker must expose only the fixture_step tool"
    );
}

#[test]
fn all_builtin_ids_are_stamped_builtin_source() {
    for def in all() {
        assert_eq!(
            def.source,
            DefinitionSource::Builtin,
            "{} should be Builtin",
            def.id
        );
    }
}

#[test]
fn expected_builtin_ids_are_present() {
    let ids: Vec<String> = all().into_iter().map(|d| d.id).collect();
    for expected in [
        "orchestrator",
        "planner",
        "code_executor",
        "integrations_agent",
        "task_manager_agent",
        "settings_agent",
        "profile_memory_agent",
        "tool_maker",
        "skill_creator",
        "researcher",
        "critic",
        "archivist",
        "summarizer",
        // Gated with `flows` (#4797) — absent from a slim build.
        #[cfg(feature = "flows")]
        "workflow_builder",
        #[cfg(feature = "flows")]
        "flow_discovery",
    ] {
        assert!(ids.contains(&expected.to_string()), "missing {expected}");
    }
}
