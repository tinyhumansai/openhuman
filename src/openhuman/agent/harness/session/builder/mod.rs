//! `AgentBuilder` fluent API and the `Agent::from_config` factory.
//!
//! Everything in this module is about *constructing* an `Agent` — the
//! builder setters, the `build()` validator, and the `from_config()`
//! factory that wires together the real provider / memory / tool
//! registry from a loaded [`Config`]. Per-turn behaviour lives in
//! [`super::turn`]; accessors and run-helpers live in [`super::runtime`].

mod factory;
mod helpers;
mod setters;

#[cfg(test)]
mod builder_tests;

use crate::openhuman::agent::harness::definition::{AgentDefinition, ToolScope};
use crate::openhuman::agent_tool_policy::ToolPolicySession;
use crate::openhuman::tools::ToolSpec;

/// Drop entries with duplicate `name` fields, first occurrence wins.
///
/// Anthropic (and other strict providers) rejects a chat/completions
/// request that lists two tools with the same name — OpenHuman's own
/// backend and OpenAI silently accept duplicates, which hid the
/// underlying collision (researcher sub-agent's `delegate_name =
/// "research"` shadowing a same-named skill tool) until #1710's
/// per-role routing started sending the same tool list to Anthropic.
///
/// Called from every place that materialises the visible tool spec
/// list — initial build, post-composio refresh, scope-filter change —
/// so the request the provider sees is always name-unique regardless
/// of which path produced it.
pub(crate) fn dedup_visible_tool_specs(specs: Vec<ToolSpec>) -> Vec<ToolSpec> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut deduped: Vec<ToolSpec> = Vec::with_capacity(specs.len());
    let mut dropped: Vec<String> = Vec::new();
    for spec in specs {
        if seen.insert(spec.name.clone()) {
            deduped.push(spec);
        } else {
            dropped.push(spec.name);
        }
    }
    if !dropped.is_empty() {
        log::warn!(
            "[agent] dropped {} duplicate tool spec(s) before sending to provider: {:?}",
            dropped.len(),
            dropped
        );
    }
    deduped
}

pub(super) fn visible_tool_specs_for_policy(
    tool_specs: &[ToolSpec],
    visible_names: &std::collections::HashSet<String>,
    tool_policy: &ToolPolicySession,
) -> Vec<ToolSpec> {
    tool_specs
        .iter()
        .filter(|spec| {
            (visible_names.is_empty() || visible_names.contains(&spec.name))
                && tool_policy.is_allowed(&spec.name)
        })
        .cloned()
        .collect()
}

/// Ensure the CCR recovery tool (`retrieve_tool_output`) is a member of a
/// non-empty visibility allowlist. Compaction runs on every agent's tool
/// output, so any agent with a curated `ToolScope::Named` list must still be
/// able to act on a `retrieve_tool_output("…")` footer. An empty set already
/// means "no filter" (all tools visible), so it is left untouched — including
/// the deliberately tool-less `Named([])` case, which must stay tool-less.
pub(super) fn ensure_recovery_tool_visible(visible: &mut std::collections::HashSet<String>) {
    if !visible.is_empty() {
        for name in crate::openhuman::tokenjuice::RECOVERY_TOOL_NAMES {
            visible.insert((*name).to_string());
        }
    }
}

pub(super) fn should_synthesize_delegation_tools(def: &AgentDefinition) -> bool {
    match &def.tools {
        ToolScope::Wildcard => !def.subagents.is_empty(),
        ToolScope::Named(names) => names.iter().any(|name| {
            matches!(
                name.as_str(),
                "spawn_subagent"
                    | "spawn_async_subagent"
                    | "spawn_parallel_agents"
                    | "spawn_worker_thread"
            )
        }),
    }
}
