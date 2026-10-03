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
//!   (rendered tool list, workspace) to the `prompt.md` body.
//!   [`BUILTINS`] installs it as `PromptSource::Dynamic` on the parsed
//!   definition. Most archetypes keep a `prompt_tests.rs` beside it.
//!
//! An archetype may additionally own a `graph.rs` exposing
//! `fn graph() -> AgentGraph` for a bespoke turn graph; see
//! [`BuiltinAgent::graph_fn`].
//!
//! `loader.rs` holds [`BUILTINS`], [`load_builtins`] and
//! [`validate_tier_hierarchy`]. The slice also registers archetypes that
//! live with other domains (`skills/*/agent/`,
//! `flows/agents/`), so this directory is not the full built-in set. The
//! package `README.md` one level up describes what each archetype does.

mod loader;

#[cfg(test)]
#[path = "fleet_prompt_tests.rs"]
mod fleet_prompt_tests;

pub mod critic;
pub mod image_agent;
pub mod morning_briefing;
pub mod orchestrator;
pub mod planner;
pub mod presentation_agent;
pub mod summarizer;
pub mod task_manager_agent;
pub mod trigger_reactor;
pub mod trigger_triage;
pub mod video_agent;
pub mod vision_agent;

pub use loader::{load_builtins, validate_tier_hierarchy, BuiltinAgent, BUILTINS};
