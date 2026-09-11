use serde_json::Value;

use super::*;
use crate::openhuman::agent::registry::types::AgentSubagentPolicy;

fn custom_agent(id: &str, enabled: bool) -> AgentRegistryEntry {
    AgentRegistryEntry {
        id: id.to_string(),
        name: "Custom".to_string(),
        description: "Handles custom work.".to_string(),
        source: AgentRegistrySource::Custom,
        enabled,
        model: Some("reasoning-v1".to_string()),
        system_prompt: Some("Do custom work.".to_string()),
        tool_allowlist: vec!["memory.search".to_string()],
        tool_denylist: Vec::new(),
        subagents: AgentSubagentPolicy::default(),
        tags: vec!["custom".to_string()],
        metadata: Value::Null,
    }
}

#[test]
fn merge_entries_applies_default_overrides_and_filters_disabled() {
    let configured = vec![AgentRegistryEntry {
        id: "researcher".to_string(),
        name: "Researcher".to_string(),
        description: "Disabled for this workspace.".to_string(),
        source: AgentRegistrySource::Default,
        enabled: false,
        model: None,
        system_prompt: None,
        tool_allowlist: vec!["*".to_string()],
        tool_denylist: Vec::new(),
        subagents: AgentSubagentPolicy::default(),
        tags: Vec::new(),
        metadata: Value::Null,
    }];

    let visible = merge_entries(&configured, false);
    assert!(!visible.iter().any(|agent| agent.id == "researcher"));

    let all = merge_entries(&configured, true);
    let researcher = all.iter().find(|agent| agent.id == "researcher").unwrap();
    assert!(!researcher.enabled);
}

#[test]
fn merge_entries_appends_custom_agents() {
    let configured = vec![custom_agent("finance_analyst", true)];
    let merged = merge_entries(&configured, true);
    assert!(merged.iter().any(|agent| agent.id == "orchestrator"));
    assert_eq!(merged.last().unwrap().id, "finance_analyst");
}

#[test]
fn find_custom_in_config_returns_matching_custom_entry() {
    let mut config = Config::default();
    config.agent_registry.entries = vec![custom_agent("finance_analyst", true)];

    let found = find_custom_in_config(&config, "finance_analyst");
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, "finance_analyst");
}

#[test]
fn find_custom_in_config_ignores_default_source_entries() {
    // A `Default`-sourced override (a user edit to a shipped agent) must
    // NOT be picked up here — it already resolves via the harness
    // `AgentDefinitionRegistry`, so this fallback should stay a miss.
    let mut config = Config::default();
    config.agent_registry.entries = vec![AgentRegistryEntry {
        source: AgentRegistrySource::Default,
        ..custom_agent("researcher", true)
    }];

    assert!(find_custom_in_config(&config, "researcher").is_none());
}

#[test]
fn find_custom_in_config_misses_unknown_id() {
    let mut config = Config::default();
    config.agent_registry.entries = vec![custom_agent("finance_analyst", true)];

    assert!(find_custom_in_config(&config, "totally_unknown").is_none());
}

#[test]
fn find_custom_in_config_ignores_disabled_custom_entries() {
    // Regression test (P2 review comment on this PR): a disabled custom
    // agent must be treated as a miss here, exactly like an unknown id —
    // otherwise a direct factory caller (chat, task-dispatcher) that
    // references a disabled custom agent's id (e.g. via an existing
    // profile) would still synthesize it into a runnable definition,
    // bypassing the disabled flag that the flows path already enforces
    // explicitly via `route_custom_entry_lookup`.
    let mut config = Config::default();
    config.agent_registry.entries = vec![custom_agent("finance_analyst", false)];

    assert!(
        find_custom_in_config(&config, "finance_analyst").is_none(),
        "a disabled custom entry must not be returned as a runnable custom agent"
    );
}

#[test]
fn ensure_orchestrator_enabled_rejects_disabled_orchestrator() {
    let mut entry = custom_agent("orchestrator", false);
    entry.source = AgentRegistrySource::Default;
    assert_eq!(
        ensure_orchestrator_enabled(&entry).unwrap_err(),
        "orchestrator agent cannot be disabled"
    );
}
