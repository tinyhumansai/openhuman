//! The registry as a running session must see it.
//!
//! Two stores describe an agent: the harness [`AgentDefinitionRegistry`]
//! (shipped definitions plus `<workspace>/agents/*.toml`, loaded once per
//! process) and `config.agent_registry.entries` (user-authored `Custom`
//! agents, and a `Default`-sourced entry for every shipped agent the user has
//! edited through `agent_registry_update`). The harness registry never reads
//! the config side, so without this module an edit to the orchestrator's
//! `subagents.allowlist` was persisted and listed by `agent_registry_get` but
//! never reached the `spawn_async_subagent` enum or the execute-side gate, and
//! a custom agent could not be spawned as a sub-agent at all (#6934).
//!
//! Everything here is a pure function over the two stores, so a caller that
//! holds the session's config snapshot gets the live answer without a
//! process restart.

use crate::agent::harness::definition::{AgentDefinition, AgentDefinitionRegistry};
use crate::agent::registry::defaults::definition_from_registry_entry;
use crate::agent::registry::ops::find_custom_in_config;
use crate::agent::registry::types::{AgentRegistryEntry, AgentRegistrySource};
use crate::config::Config;

/// The saved override for a shipped agent, if the user edited it.
///
/// `agent_registry_update` on a shipped id copies the default entry into
/// `config.agent_registry.entries` with `source: Default` and patches it in
/// place, so a `Default`-sourced entry exists exactly when the user has
/// edited that agent.
pub fn default_override_in_config<'c>(
    config: &'c Config,
    id: &str,
) -> Option<&'c AgentRegistryEntry> {
    let id = id.trim();
    config
        .agent_registry
        .entries
        .iter()
        .find(|entry| entry.id == id && matches!(entry.source, AgentRegistrySource::Default))
}

/// The sub-agent ids `definition` may dispatch, honouring a saved override.
///
/// The shipped list (`definition.allowed_subagent_ids()`) is replaced by the
/// override's `subagents.allowlist` only when the two differ. An override
/// entry carries a full copy of the default as it was when the user first
/// edited the agent, so an edit that touched something else (`enabled`, the
/// model) would otherwise freeze the allowlist at that snapshot and silently
/// ignore sub-agents a later release adds; an unchanged list is treated as
/// "not overridden" instead. Without a config, or for an agent the user never
/// edited, the shipped list is returned unchanged.
pub fn effective_subagent_allowlist(
    config: Option<&Config>,
    definition: &AgentDefinition,
) -> Vec<String> {
    let shipped = definition.allowed_subagent_ids();
    let Some(entry) = config.and_then(|config| default_override_in_config(config, &definition.id))
    else {
        return shipped;
    };
    if same_ids(&entry.subagents.allowlist, &shipped) {
        return shipped;
    }
    let mut ids = entry.subagents.allowlist.clone();
    ids.sort();
    ids.dedup();
    ids
}

/// Resolves a sub-agent id to a runnable definition: the harness registry
/// first, then an **enabled** `Custom` entry in `config.agent_registry`,
/// synthesised through [`definition_from_registry_entry`] — the same order
/// the agent factory and the TinyAgents host catalogue use. A disabled or
/// unknown id is `None`.
pub fn resolve_spawnable_definition(
    registry: &AgentDefinitionRegistry,
    config: Option<&Config>,
    id: &str,
) -> Option<AgentDefinition> {
    let id = id.trim();
    if let Some(definition) = registry.get(id) {
        return Some(definition.clone());
    }
    let entry = find_custom_in_config(config?, id)?;
    Some(definition_from_registry_entry(&entry))
}

/// Every id [`resolve_spawnable_definition`] would accept, for an
/// "unknown agent" error to list: harness definitions in registry order,
/// then enabled custom agents the harness does not already shadow.
pub fn spawnable_ids(registry: &AgentDefinitionRegistry, config: Option<&Config>) -> Vec<String> {
    let mut ids: Vec<String> = registry.list().iter().map(|d| d.id.clone()).collect();
    if let Some(config) = config {
        for entry in &config.agent_registry.entries {
            if !ids.iter().any(|known| known == &entry.id)
                && find_custom_in_config(config, &entry.id).is_some()
            {
                ids.push(entry.id.clone());
            }
        }
    }
    ids
}

/// Order-insensitive equality of two id lists.
fn same_ids(left: &[String], right: &[String]) -> bool {
    let mut left: Vec<&str> = left.iter().map(String::as_str).collect();
    let mut right: Vec<&str> = right.iter().map(String::as_str).collect();
    left.sort_unstable();
    left.dedup();
    right.sort_unstable();
    right.dedup();
    left == right
}

#[cfg(test)]
#[path = "effective_tests.rs"]
mod tests;
