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
//! 3. Replaces the (unset) `system_prompt` with `PromptSource::Dynamic(prompt_fn)`
//!    and installs the optional `graph_fn` result as the turn graph.
//! 4. Stamps `source = DefinitionSource::Builtin` and checks the folder id
//!    matches the TOML `id`.
//! 5. Returns the full `Vec<AgentDefinition>`, in the order listed in [`BUILTINS`].
//!
//! Workspace-level overrides (`$OPENHUMAN_WORKSPACE/agents/*.toml`) are
//! handled separately by [`crate::agent::harness::definition_loader`] and merged
//! into the global registry, where they replace built-ins on `id`
//! collision.

use crate::agent::harness::agent_graph::AgentGraph;
use crate::agent::harness::definition::{
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
    /// with a populated [`crate::agent::harness::definition::PromptContext`]
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
    // `planner` and `critic` are not delegable from chat (the orchestrator does
    // not list them): they exist for the `parallel_research_cross_check`
    // workflow-run template (`orchestration/workflow_runs/ops.rs`), whose
    // read-only safety tier admits only read-only agents. Planning and
    // code review in chat are the orchestrator's own `## Plans` rules and
    // skill `coding`.
    BuiltinAgent {
        id: "planner",
        toml: include_str!("planner/agent.toml"),
        prompt_fn: super::planner::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "critic",
        toml: include_str!("critic/agent.toml"),
        prompt_fn: super::critic::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "task_manager_agent",
        toml: include_str!("task_manager_agent/agent.toml"),
        prompt_fn: super::task_manager_agent::prompt::build,
        graph_fn: None,
    },
    BuiltinAgent {
        id: "presentation_agent",
        toml: include_str!("presentation_agent/agent.toml"),
        prompt_fn: super::presentation_agent::prompt::build,
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
    // Skill agents — `#[cfg]` rather than stub: `include_str!` embeds the
    // agent TOML from disk regardless of module gating, so the entry itself
    // must disappear when the `skills` feature is off.
    #[cfg(feature = "skills")]
    BuiltinAgent {
        id: "skill_setup",
        toml: include_str!("../../../skills/catalog/agent/skill_setup/agent.toml"),
        prompt_fn: crate::skills::catalog::agent::skill_setup::prompt::build,
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
        prompt_fn: crate::flows::agents::workflow_builder::prompt::build,
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
        prompt_fn: crate::flows::agents::flow_discovery::prompt::build,
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
///   entries. (Skills wildcards are allowed: they expand to the connected
///   integrations' actions as searchable tools on the agent's own belt,
///   not to a spawn.)
///
/// Skills-wildcard entries (`{ skills = "*" }`) are intentionally
/// untouched: they name no agent, so there is no tier pair to check.
///
/// Called from [`load_builtins`] for the bundled archetype set and from
/// [`crate::agent::harness::definition::AgentDefinitionRegistry::load`]
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
                // Skills wildcards expand to searchable integration
                // actions, not to an agent — nothing to tier-check.
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
