//! Agent routing and tool-scoping for channel dispatch turns.
//!
//! Contains:
//! * [`AgentScoping`] — per-turn scoping fields derived from the active agent.
//! * [`resolve_target_agent`] — reads config and registry to pick the active
//!   agent and synthesise its delegation tool surface.
//! * [`build_visible_tool_set`] — union of named tools + extra (delegation) tools.

use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, ToolScope,
};
use crate::openhuman::composio::fetch_connected_integrations;
use crate::openhuman::config::Config;
use crate::openhuman::tools::{orchestrator_tools, Tool};
use std::collections::HashSet;
use std::time::Duration;

/// Per-turn scoping fields derived from the active agent definition.
///
/// Carries the three new fields that get spliced into [`AgentTurnRequest`]
/// in [`super::processor::process_channel_message`]. Constructed by
/// [`resolve_target_agent`] after reading `config.onboarding_completed`,
/// looking up the matching definition in [`AgentDefinitionRegistry`], and
/// synthesising any per-turn delegation tools the agent needs.
pub(super) struct AgentScoping {
    pub(super) target_agent_id: Option<String>,
    pub(super) visible_tool_names: Option<HashSet<String>>,
    pub(super) extra_tools: Vec<Box<dyn Tool>>,
}

impl AgentScoping {
    /// Empty scoping — preserves the legacy "every tool in the global
    /// registry is visible" behaviour. Returned when the registry isn't
    /// initialised yet (early startup) or when the target agent
    /// definition isn't found, so the channel layer never crashes the
    /// runtime over a routing miss.
    pub(super) fn unscoped() -> Self {
        Self {
            target_agent_id: None,
            visible_tool_names: None,
            extra_tools: Vec::new(),
        }
    }
}

/// Decide which agent should run for this channel turn and build the
/// matching tool-scoping payload.
///
/// All channel turns route directly to the `orchestrator` agent. The
/// welcome agent has been removed; the Joyride walkthrough in the
/// frontend handles onboarding UI instead.
///
/// On any failure path (missing registry, missing definition, missing
/// orchestrator delegation targets) the function logs and returns
/// [`AgentScoping::unscoped`], which lets the turn run with the legacy
/// unfiltered behaviour rather than failing the whole message.
pub(super) async fn resolve_target_agent(channel: &str) -> AgentScoping {
    let config = match Config::load_or_init().await {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!(
                channel = %channel,
                error = %err,
                "[dispatch::routing] failed to load config — falling back to unscoped turn"
            );
            return AgentScoping::unscoped();
        }
    };

    let target_id = "orchestrator";

    tracing::info!(
        channel = %channel,
        target_agent = target_id,
        ui_onboarding_completed = config.onboarding_completed,
        "[dispatch::routing] selected target agent"
    );

    let registry = match AgentDefinitionRegistry::global() {
        Some(reg) => reg,
        None => {
            tracing::warn!(
                channel = %channel,
                target_agent = target_id,
                "[dispatch::routing] AgentDefinitionRegistry not initialised — falling back to unscoped turn"
            );
            return AgentScoping::unscoped();
        }
    };

    let definition = match registry.get(target_id) {
        Some(def) => def,
        None => {
            tracing::warn!(
                channel = %channel,
                target_agent = target_id,
                "[dispatch::routing] target agent not in registry — falling back to unscoped turn"
            );
            return AgentScoping::unscoped();
        }
    };

    // Synthesise per-turn delegation tools when the target agent has a
    // `subagents = [...]` field. Today only the orchestrator does, but
    // the helper is agent-agnostic so future agents that delegate
    // (e.g. a custom workspace-override planner that subdivides work)
    // pick this up for free.
    //
    // Wrap the Composio fetch in a 3-second timeout so a slow/unresponsive
    // Composio API can never block turn dispatch indefinitely.
    const COMPOSIO_FETCH_TIMEOUT_SECS: u64 = 3;
    let extra_tools = if !definition.subagents.is_empty() {
        let connected = match tokio::time::timeout(
            Duration::from_secs(COMPOSIO_FETCH_TIMEOUT_SECS),
            fetch_connected_integrations(&config),
        )
        .await
        {
            Ok(list) => list,
            Err(_) => {
                tracing::warn!(
                    channel = %channel,
                    target_agent = target_id,
                    "[dispatch::routing] Composio fetch timed out after {}s — proceeding without connected integrations",
                    COMPOSIO_FETCH_TIMEOUT_SECS
                );
                Vec::new()
            }
        };
        tracing::debug!(
            channel = %channel,
            target_agent = target_id,
            connected_integration_count = connected.len(),
            "[dispatch::routing] fetched connected integrations for delegation expansion"
        );
        orchestrator_tools::collect_orchestrator_tools(definition, registry, &connected)
    } else {
        Vec::new()
    };

    let visible_tool_names = build_visible_tool_set(definition, &extra_tools);

    tracing::debug!(
        channel = %channel,
        target_agent = target_id,
        named_tool_count = match &definition.tools {
            ToolScope::Named(names) => names.len(),
            ToolScope::Wildcard => 0,
        },
        extra_tool_count = extra_tools.len(),
        visible_tool_count = visible_tool_names.as_ref().map(|s| s.len()).unwrap_or(0),
        "[dispatch::routing] assembled tool scoping for turn"
    );

    AgentScoping {
        target_agent_id: Some(target_id.to_string()),
        visible_tool_names,
        extra_tools,
    }
}

/// Build the visible-tool whitelist for an agent.
///
/// The set is the union of:
/// * every tool name in the agent's `[tools] named = [...]` list
///   (when the scope is [`ToolScope::Named`]); and
/// * every name produced by the per-turn synthesised delegation tools
///   in `extra_tools` (e.g. `research`, `plan`,
///   `delegate_to_integrations_agent`).
///
/// When the agent's tool scope is [`ToolScope::Wildcard`] **and** there
/// are no `extra_tools`, returns `None` to preserve the legacy
/// "everything visible" semantics — a `Wildcard` agent that delegates
/// nothing should still see the full registry. When `Wildcard` is
/// combined with non-empty extras (an unusual but legal combination),
/// the legacy unfiltered behaviour also wins because the wildcard
/// implicitly covers anything in the registry plus the extras.
pub(super) fn build_visible_tool_set(
    definition: &AgentDefinition,
    extra_tools: &[Box<dyn Tool>],
) -> Option<HashSet<String>> {
    match &definition.tools {
        ToolScope::Wildcard => None,
        ToolScope::Named(names) => {
            let mut set: HashSet<String> = names.iter().cloned().collect();
            for tool in extra_tools {
                set.insert(tool.name().to_string());
            }
            Some(set)
        }
    }
}
