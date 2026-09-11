//! Config-backed operations for the user-facing agent registry.

use std::collections::HashMap;

use crate::openhuman::agent::harness::AgentDefinitionRegistry;
use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;

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

/// Synchronous, config-only lookup for a user-authored (`Custom`-source),
/// **enabled** agent registry entry by id.
///
/// Used by the agent factory (`Agent::from_config_for_agent` family, see
/// `agent::harness::session::builder::factory`) on a harness-registry lookup
/// miss so a custom agent can be synthesized into a real
/// `AgentDefinition` (via `definition_from_registry_entry`) and run with its
/// real tool belt, instead of erroring (chat/task-dispatcher) or degrading to
/// a persona-only completion (flows). Deliberately sync — unlike
/// [`get_agent`]/[`list_agents`] — because the factory already holds a
/// `&Config` in scope and must not spawn an async config reload mid-build.
///
/// Only `AgentRegistrySource::Custom` entries match: a `Default`-sourced
/// override (a user edit to a shipped agent, e.g. via `update_agent`) is
/// already resolvable through the harness `AgentDefinitionRegistry` by id —
/// that agent ships an `agent.toml`/builtin definition — so it never reaches
/// this fallback path.
///
/// A **disabled** custom entry is deliberately treated as a miss (`None`),
/// same as an unknown id — never synthesized into a runnable definition here.
/// Every caller of this function (chat, task-dispatcher, flows' registry
/// routing) resolves an agent id directly to "runnable or not"; without this
/// filter a disabled custom agent referenced by an existing profile or a
/// direct caller could still run through the harness path, silently
/// bypassing the disabled flag the flows path already enforces explicitly.
pub fn find_custom_in_config(config: &Config, id: &str) -> Option<AgentRegistryEntry> {
    let id = id.trim();
    config
        .agent_registry
        .entries
        .iter()
        .find(|entry| {
            entry.id == id && entry.enabled && matches!(entry.source, AgentRegistrySource::Custom)
        })
        .cloned()
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
#[path = "ops_tests.rs"]
mod tests;
