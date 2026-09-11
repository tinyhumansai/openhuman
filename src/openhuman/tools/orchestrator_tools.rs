//! Dynamic orchestrator tool generation.
//!
//! The orchestrator agent is direct-first and only delegates specialised
//! work. Rather than exposing a single generic
//! `spawn_subagent(agent_id, prompt)` mega-tool, we synthesise one named
//! tool per [`SubagentEntry::AgentId`] in the orchestrator's
//! `[subagents] allowlist = [...]` TOML section, so the LLM's function-calling schema
//! contains discoverable, well-named tools like `research`, `plan`,
//! `run_code`, etc.
//!
//! For [`SubagentEntry::Skills`] wildcard expansions (#1335) we synthesise
//! a single collapsed `delegate_to_integrations_agent` tool that takes the
//! toolkit slug as an argument — keeping the orchestrator's schema cost
//! constant in the integration dimension instead of scaling with the
//! number of connected toolkits.
//!
//! Each synthesised tool's description is pulled live from the target
//! agent's [`AgentDefinition::when_to_use`] (for
//! [`SubagentEntry::AgentId`]) or from the connected Composio toolkit
//! metadata (for [`SubagentEntry::Skills`] wildcard expansions) — so
//! descriptions automatically stay in sync with the definitions and
//! never drift from a hardcoded table.
//!
//! Called from [`crate::openhuman::agent::harness::session::builder`] at
//! agent-build time, with the orchestrator's own definition, the global
//! registry (for delegation target lookups), and the current list of
//! connected Composio integrations.
//!
//! [`AgentDefinition::when_to_use`]: crate::openhuman::agent::harness::definition::AgentDefinition::when_to_use
//! [`SubagentEntry::AgentId`]: crate::openhuman::agent::harness::definition::SubagentEntry::AgentId
//! [`SubagentEntry::Skills`]: crate::openhuman::agent::harness::definition::SubagentEntry::Skills

use crate::openhuman::agent::context::prompt::ConnectedIntegration;
use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, SubagentEntry,
};

// SpawnWorkerThreadTool import kept commented while the worker-thread spawn is
// temporarily disabled (see tinyhumansai/openhuman#1624).
#[allow(unused_imports)]
use super::SpawnWorkerThreadTool;
use super::{ArchetypeDelegationTool, SkillDelegationTool, Tool};
use crate::openhuman::agent::orchestration::tools::DelegationTarget;

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
/// Each [`SubagentEntry::Skills`] wildcard expands to a single
/// collapsed [`SkillDelegationTool`] named
/// `delegate_to_integrations_agent` whose `toolkit` argument selects
/// among the slugs of every connected Composio integration in
/// `connected_integrations`. The tool routes to the generic
/// `integrations_agent` with the chosen toolkit's slug passed as
/// `skill_filter`. The collapsed form keeps the orchestrator's
/// function-calling schema constant in the integration dimension
/// (#1335).
///
/// Entries that reference unknown agent ids (not in the registry) are
/// logged at `warn` and skipped — the orchestrator still builds, just
/// without the broken delegation. Entries that reference Skills wildcards
/// with an empty `connected_integrations` slice produce zero tools, which
/// is the correct behaviour when the user has not yet connected any
/// integrations (the LLM should not see a `delegate_to_integrations_agent`
/// tool with an empty enum).
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
                // Collapsed delegation tool (#1335). Previously this loop
                // emitted one `delegate_<toolkit>` tool per connected
                // integration. Every one of those tools dispatched to the
                // same `integrations_agent` with a different `skill_filter`,
                // so the fan-out cost the orchestrator schema bytes without
                // buying any new routing capability. We now emit at most
                // one `delegate_to_integrations_agent` tool that takes the
                // toolkit slug as an argument; the description enumerates
                // the connected toolkits so the orchestrator still
                // discovers which integrations are routable.
                // `sanitise_slug` is lossy — `Slack.Bot` and `Slack-Bot`
                // both collapse to `slack_bot`. Once the raw id is
                // discarded, one upstream integration would silently
                // shadow the other. Detect the collision here, drop
                // every duplicate after the first, and warn so routing
                // stays unambiguous (the first arrival keeps the slug;
                // later arrivals are unreachable through this enum and
                // safer to omit than silently re-target).
                let mut connected: Vec<(String, String)> = Vec::new();
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                for integration in connected_integrations {
                    if !integration.connected {
                        log::debug!(
                            "[orchestrator_tools] skipping unconnected integration: {}",
                            integration.toolkit
                        );
                        continue;
                    }
                    // Slug the toolkit name into a tool-name-safe
                    // (and argument-safe) form so the LLM-facing
                    // enum stays predictable across odd toolkit
                    // names (dashes, dots, spaces, mixed case).
                    let slug = sanitise_slug(&integration.toolkit);
                    if !seen.insert(slug.clone()) {
                        log::warn!(
                            "[orchestrator_tools] duplicate sanitised slug '{slug}' from raw \
                             toolkit '{raw}' — dropping to keep collapsed delegation routing \
                             unambiguous",
                            raw = integration.toolkit
                        );
                        continue;
                    }
                    // Empty integration descriptions otherwise render as a
                    // bare ` - slug` line in the collapsed tool description,
                    // which gives the orchestrator LLM no hint about what
                    // the toolkit actually does. Fall back to the
                    // generic per-toolkit phrasing the old fan-out path
                    // used so brand-new or under-populated toolkits stay
                    // informative.
                    let description = if integration.description.trim().is_empty() {
                        format!(
                            "External integration via {} — see the toolkit docs for available actions.",
                            integration.toolkit
                        )
                    } else {
                        integration.description.clone()
                    };
                    connected.push((slug, description));
                }
                // Order the enum by slug, because the order it arrives in is
                // not a contract and the order it is *advertised* in is.
                //
                // This tool's schema and description both enumerate the
                // toolkits, and the tool block is rendered ahead of the
                // conversation in every provider's cached prefix — so a
                // backend that returns the same integrations in a different
                // order would otherwise re-write the schema, and with it
                // invalidate the whole prefix including the system prompt the
                // turn loop freezes for exactly that reason. The rest of the
                // pipeline already treats order as meaningless:
                // `connected_set_hash` sorts before hashing, which is what
                // stops a reordering from reaching a reconcile at the turn
                // boundary in the first place. Sorting here makes the
                // advertised surface agree with that view instead of
                // contradicting it a layer down.
                //
                // Sorting AFTER the dedup loop, never before: the collision
                // rule above is "the first arrival keeps the slug", which is a
                // statement about arrival order and would change meaning if the
                // list were sorted first.
                connected.sort_by(|(a, _), (b, _)| a.cmp(b));
                match SkillDelegationTool::for_connected(connected) {
                    Some(tool) => {
                        log::debug!(
                            "[orchestrator_tools] registering collapsed integrations delegation tool ({} toolkits)",
                            tool.connected_toolkits.len()
                        );
                        tools.push(Box::new(tool));
                    }
                    None => {
                        log::debug!(
                            "[orchestrator_tools] no connected integrations — collapsed delegation tool omitted"
                        );
                    }
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

/// Produce a tool-name-safe slug from a free-form integration id.
/// Allows ASCII alphanumerics and underscores; everything else becomes
/// an underscore. OpenAI-style function names only accept
/// `[a-zA-Z0-9_-]{1,64}`, so this is the conservative subset.
///
/// Used both when synthesising `delegate_*` tools and when rendering the
/// delegation guide in prompts — they must agree on slug canonicalisation
/// so the prompt always references a tool name that actually exists.
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
