//! Data-driven agent definitions.
//!
//! An [`AgentDefinition`] fully specifies a sub-agent: its core prompt, model,
//! allowed tool set, runtime limits, and which sections of the parent system
//! prompt to omit. Built-in definitions live in
//! [`crate::agent::registry::agents`] — one subfolder per agent, each
//! holding an `agent.toml` (metadata) and `prompt.md` (system prompt). A
//! thin wrapper in [`super::builtin_definitions`] loads them and appends
//! the synthetic `fork` definition. Users can ship custom definitions as
//! TOML files under `$OPENHUMAN_WORKSPACE/agents/*.toml` (with a fallback
//! to `~/.openhuman/agents/*.toml` for user-global specialists) which
//! override built-ins on id collision. See [`super::definition_loader`]
//! for the directory scan + TOML parsing contract.
//!
//! Sub-agents are dispatched at runtime by the `spawn_subagent` tool, which
//! looks up an [`AgentDefinition`] by id in the global
//! [`AgentDefinitionRegistry`] and hands it to
//! [`super::subagent_runner::run_subagent`].
//!
//! This file intentionally has zero references to the rest of the agent
//! runtime — it is pure data so the model can be unit-tested in isolation
//! and serialised straight from disk.

#[cfg(test)]
#[path = "definition_tests.rs"]
mod tests;

mod agent_definition;
mod execution_spec;
mod prompt_source;
mod registry;
mod source;
mod subagents;
mod tier;

pub use agent_definition::{
    AgentDefinition, EXTENDED_MAX_TOOL_ITERATIONS, IterationPolicy, TriggerMemoryAgent,
};
pub use execution_spec::{ModelSpec, SandboxMode, ToolScope};
pub use prompt_source::{PromptBuilder, PromptSource};
pub use registry::AgentDefinitionRegistry;
pub use source::DefinitionSource;
pub use subagents::{SkillsWildcard, SubagentEntry};
pub use tier::{AgentTier, validate_tier_transition};

/// Sentinel used to represent an explicit zero-tool scope.
pub const NO_TOOLS_SENTINEL: &str = "__no_tools__";

/// Returns whether a visible-tool set represents an explicit zero-tool scope.
pub fn is_empty_tool_scope(visible: &std::collections::HashSet<String>) -> bool {
    visible.is_empty() || (visible.len() == 1 && visible.contains(NO_TOOLS_SENTINEL))
}
