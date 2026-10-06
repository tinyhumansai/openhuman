//! `SessionHostBuilder` fluent API and the `OpenHumanSessionHost::from_config` factory.
//!
//! Everything in this module is about *constructing* an `OpenHumanSessionHost` — the
//! builder setters, the `build()` validator, and the `from_config()`
//! factory that wires together the real provider / memory / tool
//! registry from a loaded [`Config`]. Per-turn behaviour lives in
//! [`super::turn`]; accessors and run-helpers live in [`super::runtime`].

pub(crate) use factory::provider_role_for_definition;

mod builder_build;
mod dispatcher;
mod factory;
mod host_tools;
mod iteration_cap;
mod permanent_tool;
mod setters;

pub use host_tools::{HostTools, HostTurnTools, TurnContext};

#[cfg(test)]
mod builder_tests;

use crate::agent::harness::definition::{AgentDefinition, ToolScope};
use crate::tools::agent_policy::ToolPolicySession;
use std::sync::Arc;
use tinytools::{Tool, ToolSpec};

/// Drop entries with duplicate `name` fields, first occurrence wins.
///
/// Anthropic (and other strict providers) rejects a chat/completions
/// request that lists two tools with the same name — OpenHuman's own
/// backend and OpenAI silently accept duplicates, which hid the
/// underlying collision (a sub-agent's `delegate_name` shadowing a
/// same-named skill tool) until #1710's
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
    config: Option<&crate::config::Config>,
    definition: Option<&crate::agent::harness::definition::AgentDefinition>,
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
            spec.name == tinyagents_harness::tool::packs::USE_SKILL
                || tool_policy.is_allowed(&spec.name)
        })
        .cloned()
        .filter_map(|mut spec| {
            if spec.name == "spawn_async_subagent" {
                // Same narrowing for the spawn enum: advertise only the ids
                // this agent's `[subagents]` allowlist lets `execute` dispatch.
                // A resolved agent whose effective allowlist is empty (shipped
                // that way, or a saved override cleared it) may spawn nothing:
                // `execute` refuses every id, so drop the tool rather than fall
                // back to the unscoped whole-registry enum.
                match allowed_subagent_ids_for(&tool_policy.profile.agent_id, config, definition) {
                    Some(allowed) if allowed.is_empty() => return None,
                    Some(allowed) => {
                        crate::agent::orchestration::tools::scope_spawn_async_subagent_spec(
                            Arc::make_mut(&mut spec),
                            &allowed,
                        )
                    }
                    None => {}
                }
                return Some(spec);
            }
            if spec.name == tinyagents_harness::tool::packs::USE_SKILL {
                // `false` means no pack has a callable tool: an empty index and
                // an empty enum are not a tool, so drop it rather than ship one.
                // `Arc::make_mut`, not `&mut spec`: the three spec views share
                // their leaves, so rewriting the pack index through the `Arc`
                // would rewrite it for every view that holds this schema — and
                // `durable_tool_specs` is meant to stay the unscoped truth.
                // This copies exactly the one spec being rewritten and leaves
                // the other ~48 visible schemas shared.
                return tinyagents_harness::tool::packs::scope_use_skill_spec(
                    &crate::tools::toolpacks::CATALOG,
                    Arc::make_mut(&mut spec),
                    &is_callable,
                )
                .then_some(spec);
            }
            Some(spec)
        })
        .collect()
}

pub(super) fn session_definition_registry(
    config: Option<&crate::config::Config>,
    definition: Option<&crate::agent::harness::definition::AgentDefinition>,
) -> crate::agent::harness::definition::AgentDefinitionRegistry {
    let mut registry = crate::agent::harness::definition::AgentDefinitionRegistry::builtins_only();
    if let Some(config) = config {
        for entry in &config.agent_registry.entries {
            if entry.enabled
                && matches!(
                    entry.source,
                    crate::agent::registry::AgentRegistrySource::Custom
                )
            {
                registry.insert(crate::agent::registry::definition_from_registry_entry(
                    entry,
                ));
            }
        }
    }
    if let Some(definition) = definition {
        registry.insert(definition.clone());
    }
    registry
}

/// Ensure the CCR recovery tool (`juice_retrieve`) is a member of a
/// non-empty visibility allowlist. Compaction runs on every agent's tool
/// output, so any agent with a curated `ToolScope::Named` list must still be
/// able to act on a `⟦tj:…⟧` marker. Only the live tool is added; the legacy
/// aliases in `RECOVERY_TOOL_NAMES` stay registered for transcript replay but
/// off the wire. An empty set already means "no filter" (all tools visible),
/// so it is left untouched — including the deliberately tool-less
/// `Named([])` case, which must stay tool-less.
///
/// `recovery_needed` is whether anything can hand this agent a recovery
/// pointer: the compaction router (`context.compaction_enabled`) or
/// TinyJuice's summary stage ([`summarizes_tool_output`]), whose footer names
/// the retrieve tool even when the router is off.
pub(super) fn ensure_recovery_tool_visible(
    visible: &mut std::collections::HashSet<String>,
    recovery_needed: bool,
) {
    // Nothing emits a `⟦tj:…⟧` marker or a summary footer, so the recovery
    // tool would be a schema with nothing to recover.
    if !recovery_needed {
        return;
    }
    // `is_empty_tool_scope`, not `is_empty`: a belt holding only
    // `NO_TOOLS_SENTINEL` is a deliberate zero-tool agent, and the compaction
    // recovery tool has nothing to recover for one — there are no tool outputs
    // to truncate. Adding it would turn "no tools" into "one tool" and put a
    // schema back on a turn whose whole point is that it stays flat.
    if !crate::agent::harness::definition::is_empty_tool_scope(visible) {
        for name in crate::inference::tokenjuice::RECOVERY_TOOL_VISIBLE {
            visible.insert((*name).to_string());
        }
    }
}

/// The recovery tool, plus the REPL tools the handle preview names, for a belt.
pub(super) fn ensure_tinyjuice_tools_visible(
    visible: &mut std::collections::HashSet<String>,
    agent_id: &str,
    config: &crate::config::Config,
) {
    ensure_recovery_tool_visible(
        visible,
        config.context.compaction_enabled || summarizes_tool_output(agent_id, config),
    );
    ensure_repl_tools_visible(
        visible,
        crate::inference::tokenjuice::repl_handle_active(config),
    );
}

/// Ensure the REPL tools (`juice_find`, `juice_extract`, `juice_summarize`) are
/// members of a non-empty visibility allowlist while large results are stored
/// behind a handle. The handle preview names them, so a curated
/// `ToolScope::Named` belt that lacked them would be told to call a tool it
/// cannot see. Same rules as [`ensure_recovery_tool_visible`]: an empty set
/// means "no filter", and a zero-tool belt stays zero-tool.
pub(super) fn ensure_repl_tools_visible(
    visible: &mut std::collections::HashSet<String>,
    handle_mode_active: bool,
) {
    if !handle_mode_active {
        return;
    }
    if !crate::agent::harness::definition::is_empty_tool_scope(visible) {
        for name in crate::inference::tokenjuice::REPL_TOOL_NAMES {
            visible.insert((*name).to_string());
        }
    }
}

/// Whether TinyJuice may summarize this agent's tool output. Only the
/// orchestrator gets a summary model, and a zero threshold turns it off.
pub(super) fn summarizes_tool_output(agent_id: &str, config: &crate::config::Config) -> bool {
    crate::inference::tokenjuice::summarizes_tool_output(agent_id, config)
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

/// The sub-agent ids `agent_id`'s registry entry allows it to spawn.
///
/// Tolerates the web channel's `orchestrator_<thread>` rename the same way the
/// orchestrator prompt does: exact match first, then the longest registry id
/// the name extends at an `_` boundary. `None` when the registry is not up or
/// the id resolves to nothing, which leaves the schema untouched; `Some(empty)`
/// is a resolved agent that may spawn nothing (deny-all, like the execute gate).
///
/// The shipped `[subagents]` list is replaced by a saved registry override
/// (`agent_registry_update` on this agent) when `config` carries one, so the
/// advertised enum follows the user's edit without a restart (#6934).
fn allowed_subagent_ids_for(
    agent_id: &str,
    config: Option<&crate::config::Config>,
    definition: Option<&crate::agent::harness::definition::AgentDefinition>,
) -> Option<Vec<String>> {
    let agent_id = agent_id.trim();
    let registry = session_definition_registry(config, definition);
    let resolved = registry.get(agent_id).or_else(|| {
        // Exact matches are handled above. For channel-renamed ids, choose
        // the most-specific registry definition before falling back to the
        // supplied session definition: a shorter session id must not shadow a
        // longer custom definition that also matches the renamed id.
        registry
            .list()
            .into_iter()
            .filter(|candidate| {
                agent_id
                    .strip_prefix(&candidate.id)
                    .is_some_and(|suffix| suffix.starts_with('_'))
            })
            .max_by_key(|candidate| candidate.id.len())
            .or_else(|| {
                definition.filter(|candidate| {
                    agent_id
                        .strip_prefix(&candidate.id)
                        .is_some_and(|suffix| suffix.starts_with('_'))
                })
            })
    })?;
    Some(crate::agent::registry::effective_subagent_allowlist(
        config, resolved,
    ))
}
