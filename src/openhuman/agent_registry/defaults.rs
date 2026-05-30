//! Default registry entries derived from shipped harness definitions.

use serde_json::Value;

use crate::openhuman::agent::harness::definition::{
    AgentDefinition, ModelSpec, SubagentEntry, ToolScope,
};

use super::types::{AgentRegistryEntry, AgentRegistrySource};

pub fn default_agents() -> Vec<AgentRegistryEntry> {
    crate::openhuman::agent_registry::agents::load_builtins()
        .map(|defs| {
            defs.into_iter()
                .map(default_entry_from_definition)
                .collect()
        })
        .unwrap_or_else(|err| {
            tracing::warn!(
                error = %err,
                "[agent_registry] failed to load default agent definitions"
            );
            Vec::new()
        })
}

fn default_entry_from_definition(def: AgentDefinition) -> AgentRegistryEntry {
    AgentRegistryEntry {
        id: def.id.clone(),
        name: def.display_name().to_string(),
        description: def.when_to_use,
        source: AgentRegistrySource::Default,
        enabled: true,
        model: model_to_registry_value(&def.model),
        system_prompt: None,
        tool_allowlist: tools_to_allowlist(&def.tools, &def.extra_tools),
        tool_denylist: def.disallowed_tools,
        subagents: def
            .subagents
            .into_iter()
            .filter_map(|entry| match entry {
                SubagentEntry::AgentId(id) => Some(id),
                SubagentEntry::Skills(_) => None,
            })
            .collect(),
        tags: vec![def.agent_tier.as_str().to_string()],
        metadata: Value::Null,
    }
}

fn model_to_registry_value(model: &ModelSpec) -> Option<String> {
    match model {
        ModelSpec::Inherit => Some("inherit".to_string()),
        ModelSpec::Exact(value) => Some(value.clone()),
        ModelSpec::Hint(value) => Some(format!("hint:{value}")),
    }
}

fn tools_to_allowlist(scope: &ToolScope, extra_tools: &[String]) -> Vec<String> {
    let mut tools = match scope {
        ToolScope::Wildcard => vec!["*".to_string()],
        ToolScope::Named(names) => names.clone(),
    };
    for tool in extra_tools {
        if !tools.contains(tool) {
            tools.push(tool.clone());
        }
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_agents_include_core_personas() {
        let agents = default_agents();
        let ids: Vec<&str> = agents.iter().map(|agent| agent.id.as_str()).collect();
        assert!(ids.contains(&"orchestrator"));
        assert!(ids.contains(&"researcher"));
        assert!(ids.contains(&"code_executor"));
        assert!(agents
            .iter()
            .all(|agent| agent.source == AgentRegistrySource::Default));
    }
}
