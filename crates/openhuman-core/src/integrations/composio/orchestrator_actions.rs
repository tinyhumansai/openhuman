//! Dynamic orchestrator tool generation.
//!
//! The orchestrator delegates specialist work through the single
//! `spawn_subagent(agent_id, prompt)` tool. `AgentId` entries in its
//! `[subagents]` allowlist constrain that tool; they do not create additional
//! model-facing tools.
//!
//! For [`SubagentEntry::Skills`] wildcard expansions we synthesize one
//! `ToolExposure::Deferred` [`ComposioActionTool`] per action of every
//! connected Composio toolkit. Those never reach the wire: a belt that opted
//! into discovery (`tool_search` in its `[tools] named`) finds them through
//! the harness's `tool_search` bridge and calls them directly.
//!
//! Called from [`crate::agent::session_host::builder`] at
//! agent-build time, with the orchestrator's own definition and the current
//! list of connected Composio integrations.
//! [`SubagentEntry::AgentId`]: tinyagents_definition::SubagentEntry::AgentId
//! [`SubagentEntry::Skills`]: tinyagents_definition::SubagentEntry::Skills

use crate::agent_registry::host_definition::AgentDefinition;
use crate::integrations::composio::ComposioActionTool;
use crate::integrations::ConnectedIntegration;
use tinyagents_definition::SubagentEntry;

use tinytools::Tool;

/// Return deferred integration actions for the agent's skills wildcard.
///
/// `AgentId` entries only configure the `spawn_subagent` allowlist; they do
/// not create additional model-facing tools.
///
/// Each [`SubagentEntry::Skills`] wildcard expands to the connected
/// integrations' actions as `Deferred` tools
/// ([`collect_deferred_integration_actions`]): off the wire, reachable
/// through the harness's `tool_search`, and callable directly by the agent
/// that found them. No delegation tool is synthesised for the wildcard.
///
/// Entries that reference unknown agent ids (not in the registry) are
/// logged at `warn` and skipped — the orchestrator still builds, just
/// without the broken delegation. A Skills wildcard with an empty
/// `connected_integrations` slice produces zero tools, which is the correct
/// behaviour when the user has not yet connected any integrations.
///
/// Returns an empty Vec when there are no connected integration actions.
pub fn collect_orchestrator_tools(
    definition: &AgentDefinition,
    connected_integrations: &[ConnectedIntegration],
) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    for entry in &definition.subagents {
        match entry {
            SubagentEntry::AgentId(agent_id) => {
                log::debug!(
                    "[orchestrator_tools] agent '{}' is allowed through spawn_subagent(agent_id)",
                    agent_id
                );
            }
            SubagentEntry::Skills(wildcard) => {
                if !wildcard.matches_all() {
                    log::warn!(
                        "[orchestrator_tools] subagent skills wildcard '{}' referenced by '{}' is not supported (only \"*\") — skipping",
                        wildcard.skills,
                        definition.id
                    );
                    continue;
                }
                // The connected toolkits' actions, one `Deferred` tool each.
                // Never on the wire: a belt that opted into discovery reaches
                // them through the harness's `tool_search`, so one clear
                // action is a search and a call. A belt that did not opt in
                // never sees them — the session builder leaves them
                // prompt-hidden, which the direct-call gate refuses. Approval
                // and channel permission apply per call. This used to sit
                // beside a per-toolkit sub-agent route; searchable actions
                // are directly callable through the harness.
                let actions = collect_deferred_integration_actions(connected_integrations);
                if !actions.is_empty() {
                    log::debug!(
                        "[orchestrator_tools] registering {} deferred integration action tool(s)",
                        actions.len()
                    );
                    tools.extend(actions);
                }
            }
        }
    }

    log::info!(
        "[orchestrator_tools] assembled {} deferred integration tool(s) for agent '{}' ({} integrations connected)",
        tools.len(),
        definition.id,
        connected_integrations.len()
    );

    tools
}

/// One `ToolExposure::Deferred` [`ComposioActionTool`] per action of every
/// connected integration, sorted by toolkit then action so the synthesised
/// set — and with it the tool specs a session freezes — is stable across
/// reconciles whatever order the backend returns.
///
/// Gated actions are left out: the model cannot call them and the prompt's
/// Connected Integrations section already explains how to unlock them.
/// A collision on an action slug across two toolkits keeps the first
/// arrival.
pub fn collect_deferred_integration_actions(
    connected_integrations: &[ConnectedIntegration],
) -> Vec<Box<dyn Tool>> {
    let mut integrations: Vec<&ConnectedIntegration> = connected_integrations
        .iter()
        .filter(|integration| integration.connected)
        .collect();
    integrations.sort_by(|a, b| a.toolkit.cmp(&b.toolkit));
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    for integration in integrations {
        let mut actions: Vec<&crate::integrations::ConnectedIntegrationTool> =
            integration.tools.iter().collect();
        actions.sort_by(|a, b| a.name.cmp(&b.name));
        for action in actions {
            if action.name.trim().is_empty() || !seen.insert(action.name.as_str()) {
                continue;
            }
            tools.push(Box::new(ComposioActionTool::deferred(
                &integration.toolkit,
                action.name.clone(),
                action.description.clone(),
                action.parameters.clone(),
            )));
        }
    }
    tools
}

/// Produce a tool-name-safe slug from a free-form integration id.
/// Allows ASCII alphanumerics and underscores; everything else becomes
/// an underscore. OpenAI-style function names only accept
/// `[a-zA-Z0-9_-]{1,64}`, so this is the conservative subset.
///
/// Used when rendering integration slugs in prompts so the prompt and any
/// argument-facing enum agree on slug canonicalisation.
pub(crate) fn sanitise_slug(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "orchestrator_actions_tests.rs"]
mod tests;
