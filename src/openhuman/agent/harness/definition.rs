//! Data-driven agent definitions.
//!
//! An [`AgentDefinition`] fully specifies a sub-agent: its core prompt, model,
//! allowed tool set, runtime limits, and which sections of the parent system
//! prompt to omit. Built-in definitions live in
//! [`crate::openhuman::agent::registry::agents`] — one subfolder per agent, each
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
include!("definition_part_01.rs");
include!("definition_part_02.rs");
