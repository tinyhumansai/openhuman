use super::*;

fn visible_names(agent_id: &str) -> std::collections::HashSet<String> {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let definition = crate::agent::harness::AgentDefinitionRegistry::builtins_only()
        .get(agent_id)
        .cloned()
        .unwrap_or_else(|| panic!("built-in agent definition not found: {agent_id}"));
    let agent = crate::agent::OpenHumanSessionHost::from_config_with_definition(
        &config,
        &definition,
        None,
        None,
    )
    .unwrap_or_else(|e| panic!("{agent_id} session build: {e}"));
    agent
        .visible_tool_specs_arc()
        .iter()
        .map(|spec| spec.name.clone())
        .collect()
}

#[test]
fn resetting_wildcard_visibility_keeps_collapsed_exposure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let definition = crate::agent::harness::AgentDefinitionRegistry::builtins_only()
        .get("tools_agent")
        .cloned()
        .expect("tools_agent built-in definition");
    let mut agent = crate::agent::OpenHumanSessionHost::from_config_with_definition(
        &config,
        &definition,
        None,
        None,
    )
    .expect("build tools agent");

    agent.set_visible_tool_names(std::collections::HashSet::new());

    let visible = agent.visible_tool_specs_arc();
    assert!(
        visible
            .iter()
            .any(|spec| spec.name == crate::memory::tools::MEMORY_TOOL_NAME)
    );
    assert!(!visible.iter().any(|spec| spec.name == "memory_store"));
}

#[test]
fn hiding_and_reseeding_wildcard_visibility_keeps_collapsed_exposure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let definition = crate::agent::harness::AgentDefinitionRegistry::builtins_only()
        .get("tools_agent")
        .cloned()
        .expect("tools_agent built-in definition");
    let mut agent = crate::agent::OpenHumanSessionHost::from_config_with_definition(
        &config,
        &definition,
        None,
        None,
    )
    .expect("build tools agent");

    agent.set_visible_tool_names(std::collections::HashSet::new());
    agent.hide_tools(&[crate::memory::tools::MEMORY_TOOL_NAME]);

    let visible = agent.visible_tool_specs_arc();
    assert!(
        !visible
            .iter()
            .any(|spec| spec.name == crate::memory::tools::MEMORY_TOOL_NAME)
    );
    assert!(!visible.iter().any(|spec| spec.name == "memory_store"));
}

/// A wildcard belt advertises the collapsed tool, never its `Hidden` members.
///
/// Both halves are asserted: the members gone AND the replacement present. A
/// belt that lost the memory surface entirely would pass a members-only check.
/// Regressed silently once already — `4efbea728` unregistered `memory` and
/// dropped the only production call to `strip_deferred_from_visible`, so every
/// wildcard agent shipped all eleven `memory_*` schemas plus `todo` beside the
/// eight `todo_*` tools it replaces.
#[test]
fn wildcard_belt_advertises_collapsed_tools_not_their_hidden_members() {
    let visible = visible_names("tools_agent");

    for collapsed in [
        crate::memory::tools::MEMORY_TOOL_NAME,
        "todo",
        crate::tools::implementations::meta::TOOL_SEARCH_NAME,
    ] {
        assert!(
            visible.contains(collapsed),
            "wildcard belt must advertise `{collapsed}`; got {visible:?}"
        );
    }
    let leaked: Vec<&String> = visible
        .iter()
        .filter(|name| {
            (name.starts_with("memory_") && name.as_str() != "memory_tree")
                || name.starts_with("todo_")
        })
        .collect();
    assert!(
        leaked.is_empty(),
        "Hidden members of `memory`/`todo` must not ship beside them: {leaked:?}"
    );
}

/// A hand-written `[tools] named` belt is left exactly as written: exposure is
/// only for the wildcard belt. `flow_memory_agent` names three read-only
/// `memory_*` tools; swapping them for `memory` would hand it `store`/`forget`.
#[test]
fn named_belt_keeps_its_legacy_members() {
    let visible = visible_names("flow_memory_agent");
    assert!(
        visible.contains("memory_recall"),
        "named belt must keep the members it lists; got {visible:?}"
    );
    assert!(
        !visible.contains(crate::memory::tools::MEMORY_TOOL_NAME),
        "named belt must not gain the collapsed tool; got {visible:?}"
    );
}
