//! Built-in agent definitions.
//!
//! Every built-in agent lives in its own subfolder here, with these files:
//!
//! * `agent.toml`  — id, when_to_use, model, tool allowlist, sandbox,
//!   iteration cap, and the `omit_*` flags. Parsed
//!   directly into [`AgentDefinition`] via serde.
//! * `prompt.rs`   — a Rust module exporting `pub fn build(ctx: &PromptContext)
//!   -> anyhow::Result<String>` that returns the sub-agent's system
//!   prompt body. Dynamic: may branch on available tools, user profile,
//!   connected integrations, model hint, etc.
//! * `graph.rs`    — optional, only for agents with a bespoke
//!   [`AgentGraph`] runner. Agents without one use [`AgentGraph::Default`].
//!
//! Adding a new built-in agent = creating a new subfolder with the required
//! metadata/prompt files, declaring the module, and appending one entry to
//! [`BUILTINS`] below. There are no match arms to update, no enum variants to
//! add, and no `include_str!` paths scattered across the harness.
//!
//! ## Flow
//!
//! 1. [`load_builtins`] walks [`BUILTINS`].
//! 2. For each entry, parses `agent.toml` into an [`AgentDefinition`].
//! 3. Replaces the (unset) `system_prompt` with `PromptSource::Inline(prompt.md contents)`.
//! 4. Stamps `source = DefinitionSource::Builtin`.
//! 5. Returns the full `Vec<AgentDefinition>`, in the order listed in [`BUILTINS`].
//!
//! The synthetic `fork` definition is *not* listed here — it's a
//! byte-stable replay of the parent and has no standalone prompt. It is
//! added by [`crate::openhuman::agent::harness::builtin_definitions::all`] on top of the
//! loader output.
//!
//! Workspace-level overrides (`$OPENHUMAN_WORKSPACE/agents/*.toml`) are
//! handled separately by [`crate::openhuman::agent::harness::definition_loader`] and merged
//! into the global registry, where they replace built-ins on `id`
//! collision.

use crate::openhuman::agent::harness::agent_graph::AgentGraph;
use crate::openhuman::agent::harness::definition::{
    validate_tier_transition, AgentDefinition, AgentTier, DefinitionSource, PromptBuilder,
    PromptSource, SubagentEntry,
};
use anyhow::{Context, Result};
use std::collections::HashMap;

/// A single built-in agent: its id plus the metadata TOML and a
/// function-driven prompt builder.
///
/// Kept as a static slice (rather than e.g. `include_dir!`) so the
/// compile-time file-existence check is explicit and grep-friendly.
pub struct BuiltinAgent {
    pub id: &'static str,
    pub toml: &'static str,
    /// Prompt builder. Invoked at spawn time by the sub-agent runner
    /// with a populated [`crate::openhuman::agent::harness::definition::PromptContext`]
    /// so the returned body can branch on runtime state.
    pub prompt_fn: PromptBuilder,
    /// Optional turn-graph selector. `None` means [`AgentGraph::Default`].
    /// Bespoke agents expose a `graph.rs::graph()` returning
    /// [`AgentGraph::Custom`] and set this field to `Some(...)`.
    pub graph_fn: Option<fn() -> AgentGraph>,
}

/// Every built-in agent, in stable display order.
///
/// **This is the only list you touch when adding a new built-in agent.**
pub const BUILTINS: &[BuiltinAgent] = &[
    BuiltinAgent {
        id: "orchestrator",
        toml: include_str!("orchestrator/agent.toml"),
        prompt_fn: super::orchestrator::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "planner",
        toml: include_str!("planner/agent.toml"),
        prompt_fn: super::planner::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "code_executor",
        toml: include_str!("code_executor/agent.toml"),
        prompt_fn: super::code_executor::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "integrations_agent",
        toml: include_str!("integrations_agent/agent.toml"),
        prompt_fn: super::integrations_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "crypto_agent",
        toml: include_str!("crypto_agent/agent.toml"),
        prompt_fn: super::crypto_agent::prompt::build,
        graph_fn: None,
    },
    // General-purpose read-only context/memory retrieval specialist for
    // automation flows. A flow `agent` node routes here via `config.agent_ref`
    // for ANY context/style/history/people need — not a fixed list of
    // cases — looping across several retrievals in one turn when the step
    // needs it. Strictly read-only (see agent.toml); `context_scout` remains
    // the right choice only for its structured `[context_bundle]` output.
    // `#[cfg(feature = "flows")]`: this agent exists only to be routed to from
    // a flow `agent` node's `config.agent_ref`. With flows compiled out there
    // is no engine, no `workflow_builder`, and no agent_ref path — it would be
    // dead registry surface — so gate it like the other flow agents
    // (`workflow_builder`, `flow_discovery`) and let a slim build drop the
    // whole flow-specific surface (AGENTS.md compile-time-gate convention).
    #[cfg(feature = "flows")]
    BuiltinAgent {
        id: "flow_memory_agent",
        toml: include_str!("flow_memory_agent/agent.toml"),
        prompt_fn: super::flow_memory_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "tools_agent",
        toml: include_str!("tools_agent/agent.toml"),
        prompt_fn: super::tools_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "task_manager_agent",
        toml: include_str!("task_manager_agent/agent.toml"),
        prompt_fn: super::task_manager_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "settings_agent",
        toml: include_str!("settings_agent/agent.toml"),
        prompt_fn: super::settings_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "profile_memory_agent",
        toml: include_str!("profile_memory_agent/agent.toml"),
        prompt_fn: super::profile_memory_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "scheduler_agent",
        toml: include_str!("scheduler_agent/agent.toml"),
        prompt_fn: super::scheduler_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "presentation_agent",
        toml: include_str!("presentation_agent/agent.toml"),
        prompt_fn: super::presentation_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "tool_maker",
        toml: include_str!("tool_maker/agent.toml"),
        prompt_fn: super::tool_maker::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "skill_creator",
        toml: include_str!("skill_creator/agent.toml"),
        prompt_fn: super::skill_creator::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "researcher",
        toml: include_str!("researcher/agent.toml"),
        prompt_fn: super::researcher::prompt::build,
        graph_fn: Some(super::researcher::graph::graph),
    },
    BuiltinAgent {
        id: "context_scout",
        toml: include_str!("context_scout/agent.toml"),
        prompt_fn: super::context_scout::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "critic",
        toml: include_str!("critic/agent.toml"),
        prompt_fn: super::critic::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "vision_agent",
        toml: include_str!("vision_agent/agent.toml"),
        prompt_fn: super::vision_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "image_agent",
        toml: include_str!("image_agent/agent.toml"),
        prompt_fn: super::image_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "video_agent",
        toml: include_str!("video_agent/agent.toml"),
        prompt_fn: super::video_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "archivist",
        toml: include_str!("archivist/agent.toml"),
        prompt_fn: super::archivist::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "goals_agent",
        toml: include_str!("goals_agent/agent.toml"),
        prompt_fn: super::goals_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "trigger_triage",
        toml: include_str!("trigger_triage/agent.toml"),
        prompt_fn: super::trigger_triage::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "trigger_reactor",
        toml: include_str!("trigger_reactor/agent.toml"),
        prompt_fn: super::trigger_reactor::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "morning_briefing",
        toml: include_str!("morning_briefing/agent.toml"),
        prompt_fn: super::morning_briefing::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "summarizer",
        toml: include_str!("summarizer/agent.toml"),
        prompt_fn: super::summarizer::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "help",
        toml: include_str!("help/agent.toml"),
        prompt_fn: super::help::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "mcp_setup",
        toml: include_str!("mcp_setup/agent.toml"),
        prompt_fn: super::mcp_setup::prompt::build,
        graph_fn: None,
    },
    // Connected-server execution specialist. Compiled out with the `mcp`
    // feature, which drops the `delegate_use_mcp_server` tool from the
    // orchestrator's synthesised belt.
    //
    // The orchestrator's `agent.toml` still lists `mcp_agent` in `subagents`
    // (TOML is data — it cannot be `cfg`'d, and forking it per-feature would
    // invite exactly the data drift this gate is meant to avoid). That
    // dangling reference is SAFE and already handled: `collect_orchestrator_tools`
    // logs a warn and skips subagent ids that are not in the registry, and
    // `validate_tier_hierarchy` explicitly `continue`s past unknown ids rather
    // than failing the boot. `orchestrator_tolerates_absent_mcp_agent` in the
    // test module below pins that contract so a future "strict unknown
    // subagent" change cannot silently break the slim build's boot.
    #[cfg(feature = "mcp")]
    BuiltinAgent {
        id: "mcp_agent",
        toml: include_str!("mcp_agent/agent.toml"),
        prompt_fn: super::mcp_agent::prompt::build,
        graph_fn: None,
    },
    // Skill agents — `#[cfg]` rather than stub: `include_str!` embeds the
    // agent TOML from disk regardless of module gating, so the entry itself
    // must disappear when the `skills` feature is off.
    #[cfg(feature = "skills")]
    BuiltinAgent {
        id: "skill_setup",
        toml: include_str!("../../../skills/catalog/agent/skill_setup/agent.toml"),
        prompt_fn: crate::openhuman::skills::catalog::agent::skill_setup::prompt::build,
        graph_fn: None,
    },
    #[cfg(feature = "skills")]
    BuiltinAgent {
        id: "skill_executor",
        toml: include_str!("../../../skills/runtime/agent/skill_executor/agent.toml"),
        prompt_fn: crate::openhuman::skills::runtime::agent::skill_executor::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "agent_memory",
        toml: include_str!("../../../memory/agent/agent/agent.toml"),
        prompt_fn: crate::openhuman::memory::agent::agent::prompt::build,
        graph_fn: None,
    },
    // Workflow-authoring specialist (Phase 5a): builds tinyflows automation
    // graphs from natural language and returns a validated PROPOSAL — it never
    // persists or enables a flow. Deliberately narrow propose-or-read tool belt.
    // Gated with `flows`: a slim build must not advertise an agent whose entire
    // tool belt is absent, so the entry (and its `include_str!`) is stripped.
    #[cfg(feature = "flows")]
    BuiltinAgent {
        id: "workflow_builder",
        toml: include_str!("../../../flows/agents/workflow_builder/agent.toml"),
        prompt_fn: crate::openhuman::flows::agents::workflow_builder::prompt::build,
        graph_fn: None,
    },
    // Workflow-discovery specialist (the "Flow Scout"): reads the user's
    // memory/threads/people/connections/flows read-only and ends by calling
    // `suggest_workflows` to record concrete, buildable automation ideas for
    // the Flows page "Suggested for you" section. It never persists or enables
    // a flow — the read-only counterpart to `workflow_builder`, which turns a
    // picked suggestion into a real graph proposal. Gated with `flows` (same
    // reasoning as `workflow_builder` above).
    #[cfg(feature = "flows")]
    BuiltinAgent {
        id: "flow_discovery",
        toml: include_str!("../../../flows/agents/flow_discovery/agent.toml"),
        prompt_fn: crate::openhuman::flows::agents::flow_discovery::prompt::build,
        graph_fn: None,
    },
];

/// Parse every entry in [`BUILTINS`] into an [`AgentDefinition`].
///
/// Errors out of the whole call on any parse failure — built-in TOML is
/// baked into the binary and therefore must always be valid. Unit tests
/// below keep that invariant honest.
pub fn load_builtins() -> Result<Vec<AgentDefinition>> {
    let defs: Vec<AgentDefinition> = BUILTINS
        .iter()
        .filter(|b| builtin_enabled(b))
        .map(parse_builtin)
        .collect::<Result<_>>()?;
    validate_tier_hierarchy(&defs)
        .context("built-in agents violate the spawn-hierarchy contract")?;
    Ok(defs)
}

/// Compile-time gate for built-ins whose deck/document tool is feature-gated.
///
/// `presentation_agent` delegates deck creation to `generate_presentation`,
/// which only registers under the `documents` feature (see `tools::ops`). In a
/// slim build without `documents`, the agent would still be advertised as
/// `make_presentation` while its filtered tool surface no longer contains any
/// tool able to produce a deck, so it is dropped from the registry in lockstep
/// with its tool.
fn builtin_enabled(_b: &BuiltinAgent) -> bool {
    #[cfg(not(feature = "documents"))]
    if _b.id == "presentation_agent" {
        return false;
    }
    true
}

/// Validate the cross-agent spawn-hierarchy contract documented on
/// [`AgentTier`].
///
/// Rules enforced here:
///
/// * `Chat` agents MUST NOT list another `Chat` agent in `subagents`.
/// * `Reasoning` agents MUST NOT list another `Reasoning` agent in
///   `subagents`.
/// * `Worker` agents MUST NOT list any [`SubagentEntry::AgentId`]
///   entries. (Workflow wildcards are allowed: they expand to the generic
///   `integrations_agent`, which is itself a `Worker`, and the call
///   happens via a single delegation tool rather than recursive spawn.)
///
/// Workflow-wildcard entries (`{ skills = "*" }`) are intentionally
/// untouched: they collapse to one `delegate_to_integrations_agent`
/// tool whose target is a `Worker` and whose use sites are well
/// understood. Mis-tiering of the `integrations_agent` itself is still
/// caught because it appears as a normal entry elsewhere.
///
/// Called from [`load_builtins`] for the bundled archetype set and from
/// [`crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::load`]
/// after workspace-local TOML overrides are merged, so custom user
/// agents that violate the contract fail the boot rather than crashing
/// at spawn time.
pub fn validate_tier_hierarchy(defs: &[AgentDefinition]) -> Result<()> {
    let tier_by_id: HashMap<&str, AgentTier> =
        defs.iter().map(|d| (d.id.as_str(), d.agent_tier)).collect();

    for def in defs {
        for entry in &def.subagents {
            let child_id = match entry {
                SubagentEntry::AgentId(id) => id.as_str(),
                // Workflow wildcards always route to `integrations_agent`
                // (a Worker) via a single collapsed delegation tool —
                // not subject to the tier-mismatch rule.
                SubagentEntry::Skills(_) => continue,
            };

            // Worker leaves: no open-ended spawn surface.
            if def.agent_tier == AgentTier::Worker {
                anyhow::bail!(
                    "agent `{parent}` is a `worker` tier and must not list `{child}` in its \
                     subagents — workers are leaf executors.",
                    parent = def.id,
                    child = child_id,
                );
            }

            let Some(child_tier) = tier_by_id.get(child_id).copied() else {
                // Unknown id — that's a separate `subagents` integrity
                // concern (covered by existing tests / runtime spawn
                // resolution); don't mask it as a tier error.
                continue;
            };

            // Same-tier delegation is forbidden for chat and reasoning.
            // (Chat→Chat would defeat the whole point of the fast tier;
            // Reasoning→Reasoning produces a depth-blowing recursion of
            // slow models.) The pair-rule lives in `validate_tier_transition`
            // (the single source of truth shared with the runtime spawn gate
            // in `run_subagent`); here we wrap its reason with the offending
            // agent ids + tiers for a boot-time-friendly diagnostic.
            if let Err(reason) = validate_tier_transition(def.agent_tier, child_tier) {
                anyhow::bail!(
                    "agent `{parent}` ({ptier}) lists `{child}` ({ctier}) in subagents — {reason}",
                    parent = def.id,
                    ptier = def.agent_tier.as_str(),
                    child = child_id,
                    ctier = child_tier.as_str(),
                );
            }
        }
    }

    Ok(())
}

/// Parse a single [`BuiltinAgent`] triple into a finished [`AgentDefinition`].
fn parse_builtin(b: &BuiltinAgent) -> Result<AgentDefinition> {
    // The TOML ships without `system_prompt` — serde falls back to
    // `defaults::empty_inline_prompt` — and the loader injects the
    // rendered sibling `prompt.md` immediately below.
    let mut def: AgentDefinition = toml::from_str(b.toml)
        .with_context(|| format!("parsing built-in agent `{}` TOML", b.id))?;

    // Install the function-driven prompt builder and stamp the source.
    def.system_prompt = PromptSource::Dynamic(b.prompt_fn);
    def.source = DefinitionSource::Builtin;

    // Install the agent's turn-graph selection (issue #4249) — the runtime
    // analogue of the prompt builder above. Default agents leave `graph_fn`
    // unset and use `AgentGraph::Default` from `AgentDefinition`.
    def.graph = b.graph_fn.map(|graph| graph()).unwrap_or_default();

    // Sanity check: file layout id must match declared TOML id. This
    // catches copy-paste mistakes where someone forgets to update the
    // `id` field after duplicating a folder.
    anyhow::ensure!(
        def.id == b.id,
        "built-in agent folder `{}` declares mismatched TOML id `{}`",
        b.id,
        def.id
    );

    Ok(def)
}

#[cfg(test)]
#[path = "loader_tests.rs"]
mod tests;
