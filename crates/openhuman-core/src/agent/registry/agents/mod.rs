//! Built-in agent archetypes.
//!
//! Each submodule below is one shipped agent and owns the same three
//! files, so none of the 29 leaf `mod.rs` files need their own doc
//! comment:
//!
//! * `agent.toml`  — id, `when_to_use`, model, tool scope, sandbox mode,
//!   iteration cap, tier, and `omit_*` flags. Parsed directly into
//!   [`crate::agent::harness::definition::AgentDefinition`]; ships without
//!   a `system_prompt`.
//! * `prompt.md`   — the static archetype body. `prompt.rs` embeds it with
//!   `include_str!`; nothing else reads it.
//! * `prompt.rs`   — exposes `pub fn build(&PromptContext) ->
//!   anyhow::Result<String>`, which appends runtime-dependent sections
//!   (rendered tool list, user files, workspace) to the `prompt.md` body.
//!   [`BUILTINS`] installs it as `PromptSource::Dynamic` on the parsed
//!   definition. Most archetypes keep a `prompt_tests.rs` beside it.
//!
//! `researcher` additionally owns a `graph.rs` exposing
//! `fn graph() -> AgentGraph` for a bespoke turn graph; see
//! [`BuiltinAgent::graph_fn`].
//!
//! `loader.rs` holds [`BUILTINS`], [`load_builtins`] and
//! [`validate_tier_hierarchy`]. The slice also registers archetypes that
//! live with other domains (`memory/agent/agent/`, `skills/*/agent/`,
//! `flows/agents/`), so this directory is not the full built-in set. The
//! package `README.md` one level up describes what each archetype does.

mod loader;

#[cfg(test)]
#[path = "fleet_prompt_tests.rs"]
mod fleet_prompt_tests;

pub mod archivist;
pub mod code_executor;
pub mod context_scout;
pub mod critic;
pub mod crypto_agent;
#[cfg(feature = "flows")]
pub mod flow_memory_agent;
pub mod goals_agent;
pub mod help;
pub mod image_agent;
pub mod integrations_agent;
#[cfg(feature = "mcp")]
pub mod mcp_agent;
pub mod mcp_setup;
pub mod morning_briefing;
pub mod orchestrator;
pub mod planner;
pub mod presentation_agent;
pub mod profile_memory_agent;
pub mod researcher;
pub mod scheduler_agent;
pub mod settings_agent;
pub mod skill_creator;
pub mod summarizer;
pub mod task_manager_agent;
pub mod tool_maker;
pub mod tools_agent;
pub mod trigger_reactor;
pub mod trigger_triage;
pub mod video_agent;
pub mod vision_agent;

pub use loader::{load_builtins, validate_tier_hierarchy, BuiltinAgent, BUILTINS};
