use super::FleetToolSet;
use tinyagents_definition::ToolScope;

fn named(tools: &[&str]) -> ToolScope {
    ToolScope::Named(tools.iter().map(|t| t.to_string()).collect())
}

#[test]
fn named_scope_exposes_only_listed_fleet_tools() {
    let set = FleetToolSet::from_scope(
        &named(&[
            "spawn_async_subagent",
            "list_subagents",
            "continue_subagent",
            "shell",
        ]),
        &[],
    );
    assert!(set.has("list_subagents"));
    assert!(set.has("continue_subagent"));
    assert!(!set.has("wait_subagent"));
    assert!(!set.has("steer_subagent"));
    assert!(!set.has("wait_loop"));
    assert!(!set.can_wait());
}

#[test]
fn wildcard_scope_exposes_every_fleet_tool_minus_denylist() {
    let set = FleetToolSet::from_scope(&ToolScope::Wildcard, &["wait*".to_string()]);
    assert!(set.has("steer_subagent"));
    assert!(set.has("close_subagent"));
    assert!(!set.has("wait"));
    assert!(!set.has("wait_loop"));
    assert!(!set.has("wait_subagent"));
    assert!(!set.can_wait());
}

#[test]
fn all_is_the_full_vocabulary() {
    let set = FleetToolSet::all();
    for tool in [
        "steer_subagent",
        "wait_subagent",
        "wait",
        "wait_loop",
        "close_subagent",
        "continue_subagent",
        "list_subagents",
    ] {
        assert!(set.has(tool), "{tool}");
    }
    assert!(set.can_wait());
}

/// The shipped orchestrator definition is the case that motivated this
/// module: it must not be told about wait/steer/close tools.
#[test]
fn builtin_orchestrator_has_no_wait_or_steer() {
    let registry = crate::agent::harness::host_definition::AgentDefinitionRegistry::builtins_only();
    let def = registry.get("orchestrator").expect("built-in orchestrator");
    let set = FleetToolSet::from_scope(&def.tools, &def.disallowed_tools);
    assert!(set.has("list_subagents"));
    assert!(set.has("continue_subagent"));
    assert!(!set.can_wait());
    assert!(!set.has("steer_subagent"));
    assert!(!set.has("close_subagent"));
}
