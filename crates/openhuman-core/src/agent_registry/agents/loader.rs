//! Built-in agent definitions.
//!
//! Every built-in agent lives in its own subfolder here, with these files:
//!
//! * `agent.toml`  — id, when_to_use, model, tool allowlist, sandbox,
//!   iteration cap, and the `omit_*` flags. Parsed into [`AgentDefinition`]
//!   through the portable definition loader.
//! * `prompt.rs`   — a Rust module exporting `pub fn build(ctx: &PromptContext)
//!   -> anyhow::Result<String>` that returns the sub-agent's system
//!   prompt body. Dynamic: may branch on available tools, user profile,
//!   connected integrations, model hint, etc.
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
//! 3. Sets the `PromptSource::Dynamic(())` marker and stores the callback in
//!    the host-only `extensions.prompt_builder` field.
//! 4. Stamps `source = DefinitionSource::Builtin` and checks the folder id
//!    matches the TOML `id`.
//! 5. Returns the full `Vec<AgentDefinition>`, in the order listed in [`BUILTINS`].
//!
//! Workspace-level overrides (`$OPENHUMAN_WORKSPACE/agents/*.toml`) are
//! handled separately by [`crate::agent_registry::host_definition::AgentDefinitionRegistry`]
//! and merged into the global registry, where they replace built-ins on id
//! collision.

use crate::agent_registry::host_definition::{AgentDefinition, PromptBuilder};
use anyhow::{Context, Result};
use tinyagents_definition::{parse_str, DefinitionSet, DefinitionSource, PromptSource};

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
}

/// Every built-in agent, in stable display order.
///
/// **This is the only list you touch when adding a new built-in agent.**
pub const BUILTINS: &[BuiltinAgent] = &[
    BuiltinAgent {
        id: "orchestrator",
        toml: include_str!("orchestrator/agent.toml"),
        prompt_fn: super::orchestrator::build,
    },
    BuiltinAgent {
        id: "task_manager_agent",
        toml: include_str!("task_manager_agent/agent.toml"),
        prompt_fn: super::task_manager_agent::build,
    },
    BuiltinAgent {
        id: "presentation_agent",
        toml: include_str!("presentation_agent/agent.toml"),
        prompt_fn: super::presentation_agent::build,
    },
    BuiltinAgent {
        id: "vision_agent",
        toml: include_str!("vision_agent/agent.toml"),
        prompt_fn: super::vision_agent::build,
    },
    BuiltinAgent {
        id: "image_agent",
        toml: include_str!("image_agent/agent.toml"),
        prompt_fn: super::image_agent::build,
    },
    BuiltinAgent {
        id: "video_agent",
        toml: include_str!("video_agent/agent.toml"),
        prompt_fn: super::video_agent::build,
    },
    BuiltinAgent {
        id: "trigger_triage",
        toml: include_str!("trigger_triage/agent.toml"),
        prompt_fn: super::trigger_triage::build,
    },
    BuiltinAgent {
        id: "trigger_reactor",
        toml: include_str!("trigger_reactor/agent.toml"),
        prompt_fn: super::trigger_reactor::build,
    },
    BuiltinAgent {
        id: "morning_briefing",
        toml: include_str!("morning_briefing/agent.toml"),
        prompt_fn: super::morning_briefing::build,
    },
    BuiltinAgent {
        id: "summarizer",
        toml: include_str!("summarizer/agent.toml"),
        prompt_fn: super::summarizer::build,
    },
    // Skill agents — `#[cfg]` rather than stub: `include_str!` embeds the
    // agent TOML from disk regardless of module gating, so the entry itself
    // must disappear when the `skills` feature is off.
    #[cfg(feature = "skills")]
    BuiltinAgent {
        id: "skill_setup",
        toml: include_str!("../../skills/catalog/agent/skill_setup/agent.toml"),
        prompt_fn: crate::skills::catalog::agent::skill_setup::prompt::build,
    },
    // Workflow-authoring specialist (Phase 5a): builds tinyflows automation
    // graphs from natural language and returns a validated PROPOSAL — it never
    // persists or enables a flow. Deliberately narrow propose-or-read tool belt.
    // Gated with `flows`: a slim build must not advertise an agent whose entire
    // tool belt is absent, so the entry (and its `include_str!`) is stripped.
    #[cfg(feature = "flows")]
    BuiltinAgent {
        id: "workflow_builder",
        toml: include_str!("workflow_builder/agent.toml"),
        prompt_fn: super::workflow_builder::prompt::build,
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
        toml: include_str!("flow_discovery/agent.toml"),
        prompt_fn: super::flow_discovery::prompt::build,
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
    let mut definitions = DefinitionSet::new();
    for definition in &defs {
        definitions.insert(definition.clone());
    }
    definitions
        .validate_tier_hierarchy()
        .context("built-in agents violate the spawn-hierarchy contract")?;
    Ok(defs)
}

/// Compile-time gate for built-ins whose deck/document tool is feature-gated.
///
/// `presentation_agent` creates decks with `generate_presentation`,
/// which only registers under the `documents` feature (see `tools::ops`). In a
/// slim build without `documents`, the agent would have no tool able to
/// produce a deck, so it is dropped from the registry in lockstep with its tool.
fn builtin_enabled(_b: &BuiltinAgent) -> bool {
    #[cfg(not(feature = "documents"))]
    if _b.id == "presentation_agent" {
        return false;
    }
    true
}

/// Parse a single [`BuiltinAgent`] triple into a finished [`AgentDefinition`].
fn parse_builtin(b: &BuiltinAgent) -> Result<AgentDefinition> {
    // The TOML ships without `system_prompt` — serde falls back to
    // `defaults::empty_inline_prompt` — and the loader installs a dynamic
    // host-owned prompt builder below. Most builders include a sibling
    // `prompt.md`; flow builders receive their archetype text from TinyFlows.
    let mut def: AgentDefinition = parse_str(b.toml, DefinitionSource::Builtin)
        .with_context(|| format!("parsing built-in agent `{}` TOML", b.id))?;

    // Install the host-owned function-driven prompt builder.
    def.system_prompt = PromptSource::Dynamic(());
    def.extensions.prompt_builder = Some(b.prompt_fn);
    def.extensions.tinyflows_prompt_owner = match b.id {
        "workflow_builder" | "flow_discovery" => Some(b.id.to_owned()),
        _ => None,
    };

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
