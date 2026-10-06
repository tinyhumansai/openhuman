use super::*;
use crate::agent::registry::types::AgentSubagentPolicy;
use crate::agent::registry::{AgentRegistryEntry, AgentRegistrySource};

fn registry_entry(id: &str, source: AgentRegistrySource, allowlist: &[&str]) -> AgentRegistryEntry {
    AgentRegistryEntry {
        id: id.to_string(),
        name: id.to_string(),
        description: format!("{id} description"),
        source,
        enabled: true,
        model: None,
        system_prompt: Some("Research the topic.".to_string()),
        tool_allowlist: vec!["web_search_tool".to_string()],
        tool_denylist: Vec::new(),
        subagents: AgentSubagentPolicy::from_allowlist(
            allowlist.iter().map(|s| s.to_string()).collect(),
        ),
        tags: Vec::new(),
        metadata: serde_json::Value::Null,
    }
}

fn spawn_enum(specs: &[std::sync::Arc<tinytools::ToolSpec>]) -> Vec<String> {
    let spec = specs
        .iter()
        .find(|spec| spec.name == "spawn_async_subagent")
        .expect("the orchestrator advertises spawn_async_subagent");
    spec.parameters
        .pointer("/properties/agent_id/enum")
        .and_then(|value| value.as_array())
        .expect("scoped enum")
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

/// #6934: `agent_registry_update` on the orchestrator's `subagents.allowlist`
/// is persisted in `config.agent_registry` but was never read when the
/// session's `spawn_async_subagent` schema was scoped, so a registered custom
/// sub-agent stayed outside the enum (and `execute`'s gate) even after a
/// restart. The override must reach both the per-session spec view and the
/// scoped tool instance whose schema native tool calling puts on the wire.
#[test]
fn a_registry_override_of_the_subagents_allowlist_changes_the_scoped_spawn_spec() {
    let tmp = tempfile::TempDir::new().unwrap();
    crate::agent::harness::AgentDefinitionRegistry::init_global_builtins().unwrap();
    let shipped = builtin_def("orchestrator").allowed_subagent_ids();
    assert!(
        !shipped.iter().any(|id| id == "researcher"),
        "fixture: `researcher` is not a shipped sub-agent"
    );

    let mut config = test_config(&tmp);
    let mut edited: Vec<&str> = shipped.iter().map(String::as_str).collect();
    edited.push("researcher");
    config.agent_registry.entries = vec![
        registry_entry("researcher", AgentRegistrySource::Custom, &[]),
        registry_entry("orchestrator", AgentRegistrySource::Default, &edited),
    ];

    let agent = crate::agent::OpenHumanSessionHost::from_config_for_agent(&config, "orchestrator")
        .expect("orchestrator session build");

    // The provider-facing spec view.
    let visible = spawn_enum(&agent.visible_tool_specs_arc());
    assert!(visible.iter().any(|id| id == "researcher"), "{visible:?}");
    for id in &shipped {
        assert!(
            visible.contains(id),
            "shipped id {id} survives the override: {visible:?}"
        );
    }

    // The scoped tool instance (native tool calling reads its schema directly).
    let tool = agent
        .tools()
        .iter()
        .find(|tool| tool.name() == "spawn_async_subagent")
        .expect("spawn_async_subagent is registered");
    let on_wire: Vec<String> = tool
        .parameters_schema()
        .pointer("/properties/agent_id/enum")
        .and_then(|value| value.as_array())
        .expect("scoped enum on the tool instance")
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
    assert!(on_wire.iter().any(|id| id == "researcher"), "{on_wire:?}");
    assert_eq!(on_wire, visible, "both views agree");
}

/// The negative half: with no override saved, the enum is exactly the
/// shipped allowlist — a custom agent the orchestrator was never allowed to
/// reach does not leak in just because it exists.
#[test]
fn without_an_override_the_scoped_spawn_spec_is_the_shipped_allowlist() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.agent_registry.entries = vec![registry_entry(
        "researcher",
        AgentRegistrySource::Custom,
        &[],
    )];

    crate::agent::harness::AgentDefinitionRegistry::init_global_builtins().unwrap();
    let agent = crate::agent::OpenHumanSessionHost::from_config_for_agent(&config, "orchestrator")
        .expect("orchestrator session build");

    let mut visible = spawn_enum(&agent.visible_tool_specs_arc());
    let mut shipped = builtin_def("orchestrator").allowed_subagent_ids();
    visible.sort();
    shipped.sort();
    shipped.dedup();
    assert_eq!(visible, shipped);
}

/// A saved override that clears the allowlist means "no sub-agents"
/// (`AgentSubagentPolicy`: "Empty means no subagent calls"). The execute gate
/// already refuses every id for an empty set, so the spawn tool must not be
/// advertised with the unscoped whole-registry enum either: it leaves the
/// provider-facing view, and the parent's gate stays empty.
#[test]
fn an_empty_saved_subagents_allowlist_withdraws_the_spawn_tool() {
    let tmp = tempfile::TempDir::new().unwrap();
    crate::agent::harness::AgentDefinitionRegistry::init_global_builtins().unwrap();
    let definition = builtin_def("orchestrator");
    assert!(
        !definition.allowed_subagent_ids().is_empty(),
        "fixture: the shipped orchestrator may spawn sub-agents"
    );
    let mut config = test_config(&tmp);
    config.agent_registry.entries = vec![registry_entry(
        "orchestrator",
        AgentRegistrySource::Default,
        &[],
    )];

    let agent = crate::agent::OpenHumanSessionHost::from_config_for_agent(&config, "orchestrator")
        .expect("orchestrator session build");

    assert!(
        !agent
            .visible_tool_specs_arc()
            .iter()
            .any(|spec| spec.name == "spawn_async_subagent"),
        "an empty saved allowlist must not advertise spawn_async_subagent"
    );
    assert!(
        agent.effective_subagent_ids().is_empty(),
        "the execute-side gate is deny-all"
    );
    let native = agent
        .tools()
        .iter()
        .find(|tool| tool.name() == "spawn_async_subagent")
        .expect("the native tool remains registered for dispatch");
    assert_eq!(
        native.parameters_schema()["properties"]["agent_id"]["enum"],
        serde_json::json!([]),
        "native tool calling must receive an explicitly empty scope"
    );
}

#[test]
fn an_exact_registry_id_wins_over_a_parent_definition_prefix() {
    let mut parent = builtin_def("orchestrator");
    parent.id = "foo".to_string();

    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.agent_registry.entries =
        vec![registry_entry("foo_bar", AgentRegistrySource::Custom, &[])];

    let allowed = super::super::allowed_subagent_ids_for("foo_bar", Some(&config), Some(&parent))
        .expect("exact registry entry resolves");
    assert!(allowed.is_empty(), "the exact foo_bar definition wins");
}

#[test]
fn the_longest_registry_prefix_wins_over_a_shorter_session_definition() {
    let mut parent = builtin_def("orchestrator");
    parent.id = "foo".to_string();
    parent.subagents = vec![crate::agent::harness::definition::SubagentEntry::AgentId(
        "parent_child".into(),
    )];

    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.agent_registry.entries = vec![registry_entry(
        "foo_bar",
        AgentRegistrySource::Custom,
        &["specific_child"],
    )];

    let allowed =
        super::super::allowed_subagent_ids_for("foo_bar_thread", Some(&config), Some(&parent))
            .expect("matching registry prefix resolves");
    assert_eq!(allowed, vec!["specific_child"]);
}
