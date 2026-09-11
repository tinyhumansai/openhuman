use super::*;
use serde_json::json;

#[test]
fn subagent_policy_accepts_legacy_array() {
    let entry: AgentRegistryEntry = serde_json::from_value(json!({
        "id": "planner",
        "name": "Planner",
        "description": "Plans work.",
        "source": "custom",
        "enabled": true,
        "subagents": ["researcher", "critic"]
    }))
    .expect("legacy subagents array should parse");

    assert_eq!(
        entry.subagents.allowlist,
        vec!["researcher".to_string(), "critic".to_string()]
    );
}

#[test]
fn subagent_policy_serializes_as_section() {
    let entry = AgentRegistryEntry {
        id: "planner".to_string(),
        name: "Planner".to_string(),
        description: "Plans work.".to_string(),
        source: AgentRegistrySource::Custom,
        enabled: true,
        model: None,
        system_prompt: None,
        tool_allowlist: Vec::new(),
        tool_denylist: Vec::new(),
        subagents: AgentSubagentPolicy::from_allowlist(vec!["researcher".to_string()]),
        tags: Vec::new(),
        metadata: Value::Null,
    };

    let value = serde_json::to_value(entry).expect("serialize entry");
    assert_eq!(
        value.get("subagents"),
        Some(&json!({ "allowlist": ["researcher"] }))
    );
}
