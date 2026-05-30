//! Config-backed operations for the user-facing agent registry.

use std::collections::HashMap;

use crate::openhuman::agent::harness::AgentDefinitionRegistry;
use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc as config_rpc;

use super::defaults::default_agents;
use super::types::{AgentRegistryEntry, AgentRegistryPatch, AgentRegistrySource, AgentToolInfo};

const ORCHESTRATOR_AGENT_ID: &str = "orchestrator";

/// Wildcard agent whose tool surface is the complete built-in tool catalog.
/// Used as the source for [`available_tools`] — the orchestrator's curated
/// `named` list is only a subset, so it can't back a general tool picker.
const TOOLS_CATALOG_AGENT_ID: &str = "tools_agent";

pub async fn list_agents(include_disabled: bool) -> Result<Vec<AgentRegistryEntry>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    Ok(merge_entries(
        &config.agent_registry.entries,
        include_disabled,
    ))
}

pub async fn get_agent(id: &str) -> Result<Option<AgentRegistryEntry>, String> {
    let id = id.trim();
    Ok(list_agents(true)
        .await?
        .into_iter()
        .find(|agent| agent.id == id))
}

pub async fn upsert_custom_agent(
    mut entry: AgentRegistryEntry,
) -> Result<AgentRegistryEntry, String> {
    entry.source = AgentRegistrySource::Custom;
    entry.validate()?;

    if default_agents().iter().any(|agent| agent.id == entry.id) {
        return Err(format!(
            "agent '{}' is a default agent; use update_agent to override it",
            entry.id
        ));
    }

    let mut config = config_rpc::load_config_with_timeout().await?;
    match config
        .agent_registry
        .entries
        .iter_mut()
        .find(|agent| agent.id == entry.id)
    {
        Some(existing) => *existing = entry.clone(),
        None => config.agent_registry.entries.push(entry.clone()),
    }
    config
        .save()
        .await
        .map_err(|e| format!("failed to save config: {e:#}"))?;
    Ok(entry)
}

pub async fn update_agent(
    id: &str,
    patch: AgentRegistryPatch,
) -> Result<AgentRegistryEntry, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("id is required".to_string());
    }

    let defaults = default_agents();
    let mut config = config_rpc::load_config_with_timeout().await?;
    let entry = match config
        .agent_registry
        .entries
        .iter_mut()
        .find(|agent| agent.id == id)
    {
        Some(existing) => existing,
        None => {
            let base = defaults
                .iter()
                .find(|agent| agent.id == id)
                .cloned()
                .ok_or_else(|| format!("agent '{id}' not found"))?;
            config.agent_registry.entries.push(base);
            config
                .agent_registry
                .entries
                .last_mut()
                .expect("pushed entry")
        }
    };

    apply_patch(entry, patch);
    entry.validate()?;
    ensure_orchestrator_enabled(entry)?;
    let updated = entry.clone();
    config
        .save()
        .await
        .map_err(|e| format!("failed to save config: {e:#}"))?;
    Ok(updated)
}

pub async fn set_agent_enabled(id: &str, enabled: bool) -> Result<AgentRegistryEntry, String> {
    update_agent(
        id,
        AgentRegistryPatch {
            enabled: Some(enabled),
            ..AgentRegistryPatch::default()
        },
    )
    .await
}

pub async fn remove_agent(id: &str) -> Result<bool, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("id is required".to_string());
    }

    let mut config = config_rpc::load_config_with_timeout().await?;
    let before = config.agent_registry.entries.len();
    config.agent_registry.entries.retain(|agent| agent.id != id);
    let removed = config.agent_registry.entries.len() < before;
    if removed {
        config
            .save()
            .await
            .map_err(|e| format!("failed to save config: {e:#}"))?;
    }
    Ok(removed)
}

/// List every assignable agent tool, with descriptions, for the editor's
/// tool picker.
///
/// Built from the wildcard [`TOOLS_CATALOG_AGENT_ID`] agent's `tool_specs()`:
/// its `ToolScope::Wildcard` definition resolves to the full built-in tool
/// catalog, so the names returned here are exactly the identifiers a
/// `tool_allowlist` is matched against. (The orchestrator uses a curated
/// `named` subset, so it would yield an incomplete catalog.) Connected-
/// integration / delegation tools are intentionally not fetched — the picker
/// surfaces the stable built-in surface only. Sorted + deduped by name for a
/// stable picker UI.
pub async fn available_tools() -> Result<Vec<AgentToolInfo>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .map_err(|e| format!("failed to initialise AgentDefinitionRegistry: {e}"))?;
    let agent = Agent::from_config_for_agent(&config, TOOLS_CATALOG_AGENT_ID)
        .map_err(|e| format!("failed to build tools-catalog agent: {e}"))?;

    let mut tools: Vec<AgentToolInfo> = agent
        .tool_specs()
        .iter()
        .map(|spec| AgentToolInfo {
            name: spec.name.clone(),
            description: spec.description.clone(),
        })
        .collect();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools.dedup_by(|a, b| a.name == b.name);
    Ok(tools)
}

pub fn merge_entries(
    configured: &[AgentRegistryEntry],
    include_disabled: bool,
) -> Vec<AgentRegistryEntry> {
    let mut default_order = Vec::new();
    let mut merged: HashMap<String, AgentRegistryEntry> = HashMap::new();
    for agent in default_agents() {
        default_order.push(agent.id.clone());
        merged.insert(agent.id.clone(), agent);
    }

    let mut custom_order = Vec::new();
    for entry in configured {
        if matches!(entry.source, AgentRegistrySource::Custom) && !merged.contains_key(&entry.id) {
            custom_order.push(entry.id.clone());
        }
        merged.insert(entry.id.clone(), entry.clone());
    }

    let mut result = Vec::new();
    for id in default_order.into_iter().chain(custom_order) {
        if let Some(agent) = merged.remove(&id) {
            if include_disabled || agent.enabled {
                result.push(agent);
            }
        }
    }
    result
}

fn apply_patch(entry: &mut AgentRegistryEntry, patch: AgentRegistryPatch) {
    if let Some(name) = patch.name {
        entry.name = name;
    }
    if let Some(description) = patch.description {
        entry.description = description;
    }
    if let Some(enabled) = patch.enabled {
        entry.enabled = enabled;
    }
    if let Some(model) = patch.model {
        entry.model = Some(model);
    }
    if let Some(system_prompt) = patch.system_prompt {
        entry.system_prompt = Some(system_prompt);
    }
    if let Some(tool_allowlist) = patch.tool_allowlist {
        entry.tool_allowlist = tool_allowlist;
    }
    if let Some(tool_denylist) = patch.tool_denylist {
        entry.tool_denylist = tool_denylist;
    }
    if let Some(subagents) = patch.subagents {
        entry.subagents = subagents;
    }
    if let Some(tags) = patch.tags {
        entry.tags = tags;
    }
    if let Some(metadata) = patch.metadata {
        entry.metadata = metadata;
    }
}

fn ensure_orchestrator_enabled(entry: &AgentRegistryEntry) -> Result<(), String> {
    if entry.id == ORCHESTRATOR_AGENT_ID && !entry.enabled {
        return Err("orchestrator agent cannot be disabled".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

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
            subagents: Vec::new(),
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
            subagents: Vec::new(),
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
    fn ensure_orchestrator_enabled_rejects_disabled_orchestrator() {
        let mut entry = custom_agent("orchestrator", false);
        entry.source = AgentRegistrySource::Default;
        assert_eq!(
            ensure_orchestrator_enabled(&entry).unwrap_err(),
            "orchestrator agent cannot be disabled"
        );
    }
}
