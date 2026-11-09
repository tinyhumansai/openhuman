//! Built-in agent archetypes.
//!
//! Each submodule below is one shipped agent and generally has the same three
//! files, so none of the leaf `mod.rs` files need their own doc comment. The
//! trigger prompt builders live with the triage domain instead:
//!
//! * `agent.toml`  — id, `when_to_use`, model, tool scope, sandbox mode,
//!   iteration cap, tier, and `omit_*` flags. Parsed directly into
//!   [`crate::agent_registry::host_definition::AgentDefinition`]; ships without
//!   a `system_prompt`.
//! * `prompt.md`   — the static archetype body. `prompt.rs` embeds it with
//!   `include_str!`; nothing else reads it.
//! * `prompt.rs`   — exposes `pub fn build(&PromptContext) ->
//!   anyhow::Result<String>`, which appends runtime-dependent sections
//!   (rendered tool list, workspace) to the `prompt.md` body.
//!   [`BUILTINS`] pairs the parsed definition with its host prompt callback.
//!   The loader records `PromptSource::Dynamic(())` and stores that callback
//!   in the host-only prompt-builder extension. Most archetypes keep a
//!   `prompt_tests.rs` beside it.
//!
//! `loader.rs` holds [`BUILTINS`] and [`load_builtins`]. It delegates tier
//! validation to `tinyagents_definition::DefinitionSet`. The slice also
//! registers archetypes that live with other domains (`skills/*/agent/`,
//! `flows/agents/`), so this directory is not the full built-in set. The
//! package `README.md` one level up describes what each archetype does.

mod loader;

#[cfg(test)]
#[path = "fleet_prompt_tests.rs"]
mod fleet_prompt_tests;

#[path = "image_agent/prompt.rs"]
pub mod image_agent;
#[path = "morning_briefing/prompt.rs"]
pub mod morning_briefing;
#[path = "orchestrator/prompt.rs"]
pub mod orchestrator;
#[path = "presentation_agent/prompt.rs"]
pub mod presentation_agent;
#[path = "summarizer/prompt.rs"]
pub mod summarizer;
#[path = "task_manager_agent/prompt.rs"]
pub mod task_manager_agent;
pub use crate::triage::trigger_reactor_prompt as trigger_reactor;
pub use crate::triage::trigger_triage_prompt as trigger_triage;
#[path = "video_agent/prompt.rs"]
pub mod video_agent;
#[path = "vision_agent/prompt.rs"]
pub mod vision_agent;

#[cfg(feature = "flows")]
#[path = "flow_discovery/mod.rs"]
pub mod flow_discovery;
#[cfg(feature = "flows")]
mod flow_prompts;
#[cfg(feature = "flows")]
#[path = "workflow_builder/mod.rs"]
pub mod workflow_builder;

pub use loader::{load_builtins, BuiltinAgent, BUILTINS};
