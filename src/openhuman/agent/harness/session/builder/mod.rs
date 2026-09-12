//! `AgentBuilder` fluent API and the `Agent::from_config` factory.
//!
//! Everything in this module is about *constructing* an `Agent` — the
//! builder setters, the `build()` validator, and the `from_config()`
//! factory that wires together the real provider / memory / tool
//! registry from a loaded [`Config`]. Per-turn behaviour lives in
//! [`super::turn`]; accessors and run-helpers live in [`super::runtime`].

mod builder_build;
mod factory;
mod helpers;
mod setters;

#[cfg(test)]
mod builder_tests;

use crate::openhuman::agent::harness::definition::{AgentDefinition, ToolScope};
use crate::openhuman::tools::agent_policy::ToolPolicySession;
use crate::openhuman::tools::{Tool, ToolSpec};
use std::sync::Arc;

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
///
/// Generic over the element type so the two carriers of a spec list share one
/// implementation: the main agent holds `Arc<ToolSpec>` (the three spec views
/// share their leaves), while the sub-agent assembly still materialises owned
/// `ToolSpec`s for the public `AgentTurnRequest`.
pub(crate) fn dedup_visible_tool_specs<S: std::borrow::Borrow<ToolSpec>>(specs: Vec<S>) -> Vec<S> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut deduped: Vec<S> = Vec::with_capacity(specs.len());
    let mut dropped: Vec<String> = Vec::new();
    for spec in specs {
        let name = spec.borrow().name.clone();
        if seen.insert(name.clone()) {
            deduped.push(spec);
        } else {
            dropped.push(name);
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

/// Drop every synthesised delegation tool whose name a durable tool already
/// owns.
///
/// The durable registry and the synthesised set are advertised, classified and
/// dispatched as one surface, and every reader resolves a name to its first
/// match with the durable set enumerated first. A synthesised entry that
/// collides can therefore never be reached — keeping it would put one tool's
/// schema on the wire and run another's (a `delegate_name = "research"`
/// beside a same-named skill tool, #1710). Applied when the session is built
/// and again on every refresh, so the two sets are disjoint by construction
/// and a collision resolves the same way at both sites.
pub(crate) fn drop_synthesized_name_collisions(
    durable: &[Box<dyn Tool>],
    synthesized: Vec<Box<dyn Tool>>,
) -> Vec<Box<dyn Tool>> {
    let taken: std::collections::HashSet<&str> = durable.iter().map(|t| t.name()).collect();
    let mut kept: Vec<Box<dyn Tool>> = Vec::with_capacity(synthesized.len());
    let mut dropped: Vec<String> = Vec::new();
    for tool in synthesized {
        if taken.contains(tool.name()) {
            dropped.push(tool.name().to_string());
        } else {
            kept.push(tool);
        }
    }
    if !dropped.is_empty() {
        log::warn!(
            "[agent] dropped {} synthesised delegation tool(s) whose name a durable tool already owns: {:?}",
            dropped.len(),
            dropped
        );
    }
    kept
}

pub(super) fn visible_tool_specs_for_policy(
    tool_specs: &[Arc<ToolSpec>],
    visible_names: &std::collections::HashSet<String>,
    tool_policy: &ToolPolicySession,
) -> Vec<Arc<ToolSpec>> {
    // `use_skill`'s description carries the pack index, and its `skill` enum
    // carries the pack ids. Both are built once in `UseSkillTool::new`, before
    // any session exists, so every agent was told all ten packs were loadable —
    // including ones it can call nothing in. The model went, found out, and came
    // back.
    //
    // This is the only place that turns tools into the specs a provider sees AND
    // holds the session, so it is where the invitation is made to agree with the
    // gate. The same predicate the gate uses (`decision_for(..).is_denied()`),
    // not `is_allowed` — matching the filter above would re-introduce the two
    // sources of truth this whole fix exists to collapse.
    // `blocks_execution`, not `is_denied`: a withheld packed tool is
    // `HideFromPrompt` and perfectly callable through `use_skill`. Using
    // `is_denied` here would drop every pack from the index and then drop
    // `use_skill` itself — the exact capability the pack mechanism exists to
    // preserve.
    let is_callable = |name: &str| !tool_policy.decision_for(name).blocks_execution();
    tool_specs
        .iter()
        .filter(|spec| {
            if !(visible_names.is_empty() || visible_names.contains(&spec.name)) {
                return false;
            }
            // `use_skill`'s own listing action (`skill` named, no `tool`) is
            // always `ReadOnly` — see `UseSkillTool::permission_level_with_args`.
            // Its argument-less `permission_level()` instead reports the max
            // over every packed tool, because that IS the honest ceiling for a
            // *named* call — but `tool_policy.is_allowed` was built from that
            // same argument-less ceiling, so one Dangerous packed tool anywhere
            // made the whole proxy fail this filter and vanish from the wire,
            // hiding every other pack's tools along with it. `scope_use_skill_spec`
            // below already does the real per-tool narrowing via `is_callable`
            // (and drops the spec entirely when nothing survives), so this
            // filter only needs to gate *other* tools on the static ceiling.
            spec.name == crate::openhuman::tools::toolpacks::USE_SKILL
                || tool_policy.is_allowed(&spec.name)
        })
        .cloned()
        .filter_map(|mut spec| {
            if spec.name == crate::openhuman::tools::toolpacks::USE_SKILL {
                // `false` means no pack has a callable tool: an empty index and
                // an empty enum are not a tool, so drop it rather than ship one.
                // `Arc::make_mut`, not `&mut spec`: the three spec views share
                // their leaves, so rewriting the pack index through the `Arc`
                // would rewrite it for every view that holds this schema — and
                // `durable_tool_specs` is meant to stay the unscoped truth.
                // This copies exactly the one spec being rewritten and leaves
                // the other ~48 visible schemas shared.
                return crate::openhuman::tools::toolpacks::scope_use_skill_spec(
                    Arc::make_mut(&mut spec),
                    &is_callable,
                )
                .then_some(spec);
            }
            Some(spec)
        })
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
        for name in crate::openhuman::inference::tokenjuice::RECOVERY_TOOL_NAMES {
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
