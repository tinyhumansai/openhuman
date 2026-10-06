use super::*;
use crate::agent::harness::definition::{AgentDefinition, AgentDefinitionRegistry, SubagentEntry};
use crate::agent::registry::types::AgentSubagentPolicy;
use serde_json::Value;

fn entry(
    id: &str,
    source: AgentRegistrySource,
    enabled: bool,
    allowlist: &[&str],
) -> AgentRegistryEntry {
    AgentRegistryEntry {
        id: id.to_string(),
        name: id.to_string(),
        description: format!("{id} description"),
        source,
        enabled,
        model: None,
        system_prompt: Some("Do the work.".to_string()),
        tool_allowlist: vec!["web_search_tool".to_string()],
        tool_denylist: Vec::new(),
        subagents: AgentSubagentPolicy::from_allowlist(
            allowlist.iter().map(|s| s.to_string()).collect(),
        ),
        tags: Vec::new(),
        metadata: Value::Null,
    }
}

fn config_with(entries: Vec<AgentRegistryEntry>) -> Config {
    let mut config = Config::default();
    config.agent_registry.entries = entries;
    config
}

fn orchestrator() -> AgentDefinition {
    AgentDefinitionRegistry::builtins_only()
        .get("orchestrator")
        .expect("orchestrator ships as a built-in")
        .clone()
}

#[test]
fn default_override_is_only_a_default_sourced_entry_with_that_id() {
    let config = config_with(vec![
        entry("researcher", AgentRegistrySource::Custom, true, &[]),
        entry(
            "orchestrator",
            AgentRegistrySource::Default,
            true,
            &["researcher"],
        ),
    ]);

    assert!(default_override_in_config(&config, "orchestrator").is_some());
    assert!(default_override_in_config(&config, " orchestrator ").is_some());
    assert!(
        default_override_in_config(&config, "researcher").is_none(),
        "a custom entry is not an override of a shipped agent"
    );
    assert!(default_override_in_config(&config, "archivist").is_none());
}

#[test]
fn effective_allowlist_follows_a_saved_override_that_differs_from_the_shipped_list() {
    let definition = orchestrator();
    let shipped = definition.allowed_subagent_ids();
    assert!(!shipped.is_empty(), "fixture needs a shipped allowlist");

    let mut edited: Vec<&str> = shipped.iter().map(String::as_str).collect();
    edited.push("researcher");
    let config = config_with(vec![entry(
        "orchestrator",
        AgentRegistrySource::Default,
        true,
        &edited,
    )]);

    let effective = effective_subagent_allowlist(Some(&config), &definition);
    assert!(
        effective.iter().any(|id| id == "researcher"),
        "{effective:?}"
    );
    for id in &shipped {
        assert!(
            effective.contains(id),
            "shipped id {id} survives: {effective:?}"
        );
    }
    assert_eq!(effective.len(), shipped.len() + 1, "sorted and deduped");
}

#[test]
fn effective_allowlist_keeps_the_shipped_list_without_a_config_or_an_override() {
    let definition = orchestrator();
    let shipped = definition.allowed_subagent_ids();

    assert_eq!(effective_subagent_allowlist(None, &definition), shipped);

    let unrelated = config_with(vec![entry(
        "archivist",
        AgentRegistrySource::Default,
        true,
        &["researcher"],
    )]);
    assert_eq!(
        effective_subagent_allowlist(Some(&unrelated), &definition),
        shipped
    );
}

#[test]
fn an_override_equal_to_the_shipped_list_is_not_an_override() {
    // `agent_registry_update` stores a full copy of the default; an edit that
    // only touched `enabled` must not freeze the allowlist at that snapshot.
    let definition = orchestrator();
    let shipped = definition.allowed_subagent_ids();
    let mut reordered: Vec<&str> = shipped.iter().map(String::as_str).collect();
    reordered.reverse();
    let config = config_with(vec![entry(
        "orchestrator",
        AgentRegistrySource::Default,
        false,
        &reordered,
    )]);

    assert_eq!(
        effective_subagent_allowlist(Some(&config), &definition),
        shipped,
        "order-insensitive equality keeps the shipped list verbatim"
    );
}

#[test]
fn effective_allowlist_can_also_narrow_the_shipped_list() {
    let definition = orchestrator();
    let config = config_with(vec![entry(
        "orchestrator",
        AgentRegistrySource::Default,
        true,
        &["task_manager_agent"],
    )]);

    assert_eq!(
        effective_subagent_allowlist(Some(&config), &definition),
        vec!["task_manager_agent".to_string()]
    );
}

#[test]
fn resolve_prefers_the_harness_registry_and_falls_back_to_an_enabled_custom_entry() {
    let registry = AgentDefinitionRegistry::builtins_only();
    let config = config_with(vec![
        entry("researcher", AgentRegistrySource::Custom, true, &[]),
        entry("retired", AgentRegistrySource::Custom, false, &[]),
        // A custom entry that shadows a shipped id never replaces it.
        entry("orchestrator", AgentRegistrySource::Custom, true, &[]),
    ]);

    let shipped = resolve_spawnable_definition(&registry, Some(&config), "orchestrator")
        .expect("shipped agent resolves");
    assert_eq!(shipped.id, "orchestrator");
    assert!(
        !shipped.allowed_subagent_ids().is_empty(),
        "the harness definition won, not the empty custom shadow"
    );

    let custom = resolve_spawnable_definition(&registry, Some(&config), " researcher ")
        .expect("enabled custom agent resolves");
    assert_eq!(custom.id, "researcher");
    assert!(matches!(
        custom.source,
        crate::agent::harness::definition::DefinitionSource::CustomRegistry
    ));
    assert!(custom
        .subagents
        .iter()
        .all(|e| !matches!(e, SubagentEntry::AgentId(_))));

    assert!(
        resolve_spawnable_definition(&registry, Some(&config), "retired").is_none(),
        "a disabled custom agent is not spawnable"
    );
    assert!(resolve_spawnable_definition(&registry, Some(&config), "nobody").is_none());
    assert!(
        resolve_spawnable_definition(&registry, None, "researcher").is_none(),
        "without a config only the harness registry answers"
    );
}

#[test]
fn spawnable_ids_lists_harness_ids_then_enabled_customs_without_duplicates() {
    let registry = AgentDefinitionRegistry::builtins_only();
    let config = config_with(vec![
        entry("researcher", AgentRegistrySource::Custom, true, &[]),
        entry("retired", AgentRegistrySource::Custom, false, &[]),
        entry(
            "orchestrator",
            AgentRegistrySource::Default,
            true,
            &["researcher"],
        ),
    ]);

    let ids = spawnable_ids(&registry, Some(&config));
    let harness_len = registry.len();
    assert_eq!(ids.len(), harness_len + 1, "{ids:?}");
    assert_eq!(ids.last().map(String::as_str), Some("researcher"));
    assert!(!ids.iter().any(|id| id == "retired"));
    assert_eq!(
        ids.iter()
            .filter(|id| id.as_str() == "orchestrator")
            .count(),
        1
    );

    assert_eq!(spawnable_ids(&registry, None).len(), harness_len);
}
