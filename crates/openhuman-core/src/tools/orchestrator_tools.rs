//! Dynamic orchestrator tool generation.
//!
//! The orchestrator agent is direct-first and only delegates specialised
//! work. Rather than exposing a single generic
//! `spawn_subagent(agent_id, prompt)` mega-tool, we synthesise one named
//! tool per [`SubagentEntry::AgentId`] in the orchestrator's
//! `[subagents] allowlist = [...]` TOML section, so the LLM's function-calling schema
//! contains discoverable, well-named tools like `manage_tasks`,
//! `create_image`, `build_workflow`, etc.
//!
//! For [`SubagentEntry::Skills`] wildcard expansions we synthesise one
//! `ToolExposure::Deferred` [`ComposioActionTool`] per action of every
//! connected Composio toolkit. Those never reach the wire: a belt that opted
//! into discovery (`tool_search` in its `[tools] named`) finds them through
//! the harness's `tool_search` bridge and calls them directly, so "send this
//! email" is one search and one call. There is no `delegate_to_integrations_
//! agent` any more — routing a single action through a sub-agent spawn cost a
//! blocking agentic round-trip and a second prompt for work the parent could
//! do in one call.
//!
//! Each synthesised delegation tool's description is pulled live from the
//! target agent's [`AgentDefinition::when_to_use`] — so descriptions
//! automatically stay in sync with the definitions and never drift from a
//! hardcoded table.
//!
//! Called from [`crate::agent::session_host::builder`] at
//! agent-build time, with the orchestrator's own definition, the global
//! registry (for delegation target lookups), and the current list of
//! connected Composio integrations.
//!
//! [`AgentDefinition::when_to_use`]: crate::agent::harness::definition::AgentDefinition::when_to_use
//! [`SubagentEntry::AgentId`]: crate::agent::harness::definition::SubagentEntry::AgentId
//! [`SubagentEntry::Skills`]: crate::agent::harness::definition::SubagentEntry::Skills

use crate::agent::harness::definition::{AgentDefinition, AgentDefinitionRegistry, SubagentEntry};
use crate::agent::prompts::ConnectedIntegration;
use crate::integrations::composio::ComposioActionTool;

// SpawnWorkerThreadTool import kept commented while the worker-thread spawn is
// temporarily disabled (see tinyhumansai/openhuman#1624).
use crate::agent::orchestration::tools::ArchetypeDelegationTool;
use crate::agent::orchestration::tools::DelegationTarget;
#[allow(unused_imports)]
use crate::agent::orchestration::tools::SpawnWorkerThreadTool;
use tinytools::Tool;

/// Synthesise the delegation tool list for an agent based on its
/// declarative `subagents` field.
///
/// Each [`SubagentEntry::AgentId`] is resolved against `registry` and
/// rendered as an [`ArchetypeDelegationTool`] whose `name()` defaults to
/// `delegate_{target.id}` (overridable via the target agent's
/// `delegate_name` field) and whose `description()` is the target's
/// `when_to_use` — so editing an agent's TOML description immediately
/// updates the tool schema the orchestrator LLM sees, with zero drift.
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
/// Returns an empty Vec when `definition.subagents` is empty — callers
/// (notably the builder) handle this by not extending the visible-tool
/// set, so non-delegating agents behave identically to how they did
/// before this module existed.
pub fn collect_orchestrator_tools(
    definition: &AgentDefinition,
    registry: &AgentDefinitionRegistry,
    connected_integrations: &[ConnectedIntegration],
) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    // Orchestrator-only tool: spawn_worker_thread.
    // Temporarily disabled — worker threads do not yet have a proper UI
    // showcase (see tinyhumansai/openhuman#1624). Re-enable once the
    // dedicated worker-thread surface lands.
    // if definition.id == "orchestrator" {
    //     tools.push(Box::new(SpawnWorkerThreadTool::new()));
    // }

    for entry in &definition.subagents {
        match entry {
            SubagentEntry::AgentId(agent_id) => {
                // Runtime-only sub-agents — the LLM must never see a
                // `delegate_*` tool for these because they're dispatched
                // directly by the runtime, not by an explicit LLM tool
                // call. Issue #574 introduced `summarizer` as the first
                // such sub-agent; future runtime-only agents should
                // join this filter.
                if agent_id == "summarizer" {
                    log::debug!(
                        "[orchestrator_tools] skipping runtime-only sub-agent '{}' \
                         (no delegation tool synthesised)",
                        agent_id
                    );
                    continue;
                }
                let Some(target) = registry.get(agent_id) else {
                    log::warn!(
                        "[orchestrator_tools] subagent '{}' referenced by '{}' is not in the registry — skipping",
                        agent_id,
                        definition.id
                    );
                    continue;
                };
                let tool_name = target
                    .delegate_name
                    .clone()
                    .unwrap_or_else(|| format!("delegate_{}", target.id));
                log::debug!(
                    "[orchestrator_tools] registering archetype delegation tool: {} -> {}",
                    tool_name,
                    target.id
                );
                // The description is the target's `when_to_use` verbatim.
                //
                // It used to be prefixed with "Use only when direct
                // response/direct tools are insufficient. " — 13 tokens
                // repeated once per delegate tool, ~250 per turn on the Master
                // Agent, restating a rule its prompt already carries as
                // "**Direct-first always**". A parent whose prompt does not
                // state that rule should gain it there, once, rather than
                // paying for it on every delegate schema on every turn.
                tools.push(Box::new(ArchetypeDelegationTool {
                    tool_name,
                    agent_id: DelegationTarget(target.id.clone()),
                    tool_description: target.when_to_use.clone(),
                }));
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
                // beside a collapsed `delegate_to_integrations_agent` tool
                // that spawned `integrations_agent` per toolkit; with the
                // actions searchable that spawn only added a blocking
                // sub-agent round-trip, so the delegation tool is gone.
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
        "[orchestrator_tools] assembled {} delegation tool(s) for agent '{}' ({} integrations connected)",
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
        let mut actions: Vec<&crate::agent::prompts::ConnectedIntegrationTool> =
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
#[path = "orchestrator_tools_tests.rs"]
mod tests;
