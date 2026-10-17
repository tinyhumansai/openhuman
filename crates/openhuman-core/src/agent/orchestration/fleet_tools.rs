//! Which fleet-control tools a parent agent can actually call.
//!
//! The `[async_subagent_ref]` envelope and the ambient `[active_subagents]`
//! roster used to hard-code a full fleet vocabulary — `wait_subagent`,
//! `steer_subagent`, `wait_loop`, `close_subagent` — while the orchestrator's
//! definition deliberately dropped most of it (#5701: a sub-agent result is
//! delivered back automatically on a later turn, so nothing needs to block).
//! The model was told to call tools it did not have, spent an iteration
//! reasoning about the mismatch, and improvised (`shell echo "waiting for
//! subagent"`). Every delegation paid a full extra model call for nothing.
//!
//! This module reads the parent's definition once per render and answers
//! "does this parent see tool X?", so both texts only ever name tools that
//! are in the caller's belt.

use crate::agent::harness::host_definition::AgentDefinitionRegistry;
use tinyagents_definition::ToolScope;

/// The fleet-control tools whose availability shapes the delegation texts.
const FLEET_TOOLS: &[&str] = &[
    "steer_subagent",
    "wait_subagent",
    "wait",
    "wait_loop",
    "close_subagent",
    "continue_subagent",
    "list_subagents",
];

/// The subset of [`FLEET_TOOLS`] a given parent definition exposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FleetToolSet {
    available: Vec<&'static str>,
}

impl FleetToolSet {
    /// Every fleet tool — the pre-#5701 assumption, used when the parent's
    /// definition cannot be resolved so the texts degrade to their old shape
    /// rather than to silence.
    pub(crate) fn all() -> Self {
        Self {
            available: FLEET_TOOLS.to_vec(),
        }
    }

    /// Resolve the set for `agent_definition_id` from the global registry.
    pub(crate) fn for_parent(agent_definition_id: &str) -> Self {
        let Some(registry) = AgentDefinitionRegistry::current() else {
            return Self::all();
        };
        let Some(definition) = registry.get(agent_definition_id) else {
            log::debug!(
                "[fleet_tools] parent definition '{}' not in registry; assuming full fleet vocabulary",
                agent_definition_id
            );
            return Self::all();
        };
        Self::from_scope(&definition.tools, &definition.disallowed_tools)
    }

    /// Resolve the set from a turn's *effective* visible tool names — the
    /// live, already-filtered membership set (hides, named restrictions, and
    /// policy narrowing all applied), as opposed to [`Self::for_parent`]'s
    /// static read of the parent's registered definition. Prefer this
    /// whenever the caller already has that snapshot: a hide or restriction
    /// applied mid-session can narrow a turn's real tool surface below what
    /// the definition alone would suggest, and offering a control absent
    /// from this snapshot invites a denied tool call.
    pub(crate) fn from_visible_tool_names(names: &std::collections::HashSet<String>) -> Self {
        let available = FLEET_TOOLS
            .iter()
            .copied()
            .filter(|name| names.contains(*name))
            .collect();
        Self { available }
    }

    /// Derive the set from a definition's tool scope and denylist. A
    /// `Named` scope exposes exactly the fleet tools it lists; `Wildcard`
    /// exposes all of them. `disallowed_tools` (exact or trailing-`*`
    /// prefix) removes entries from either.
    pub(crate) fn from_scope(scope: &ToolScope, disallowed: &[String]) -> Self {
        let denied = |name: &str| {
            disallowed
                .iter()
                .any(|entry| match entry.strip_suffix('*') {
                    Some(prefix) => name.starts_with(prefix),
                    None => entry == name,
                })
        };
        let available = FLEET_TOOLS
            .iter()
            .copied()
            .filter(|name| match scope {
                ToolScope::Wildcard => true,
                ToolScope::Named(named) => named.iter().any(|n| n == name),
            })
            .filter(|name| !denied(name))
            .collect();
        Self { available }
    }

    pub(crate) fn has(&self, tool: &str) -> bool {
        self.available.contains(&tool)
    }

    /// Whether the parent can block on or poll a worker at all.
    pub(crate) fn can_wait(&self) -> bool {
        self.has("wait_subagent")
    }
}

#[cfg(test)]
#[path = "fleet_tools_tests.rs"]
mod tests;
