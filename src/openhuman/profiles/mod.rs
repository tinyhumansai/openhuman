//! Profiles domain — persistent, user-selectable agent "flavours".
//!
//! Each profile carries a custom name + SOUL.md, runtime defaults (model,
//! temperature, system-prompt suffix), and configurable allowlists for tools,
//! skills, MCP servers, connectors, and memory sources. Selecting a profile
//! changes how the agent introduces itself, what it remembers, and what it can
//! do. State persists under `<workspace>/agent_profiles.json`.
//!
//! Relocated from `openhuman::agent::profiles` / `::personality_paths` so the
//! domain is addressable on its own (`openhuman::profiles`).

pub mod ops;
pub mod paths;
pub mod prompt_section;
mod schemas;
pub mod store;
pub mod types;

pub use paths::{
    filter_integrations, memory_subdir_for_suffix, memory_tree_subdir_for_suffix,
    resolve_personality_memory_md, resolve_personality_soul, session_raw_subdir_for_suffix,
    HasToolkit, PersonalityContext,
};
pub use prompt_section::AgentProfilePromptSection;
pub use store::{built_in_profiles, load_profiles, AgentProfileStore};
pub use types::{profile_signature, AgentProfile, AgentProfilesState, DEFAULT_PROFILE_ID};

pub use schemas::{
    all_controller_schemas as all_profiles_controller_schemas,
    all_registered_controllers as all_profiles_registered_controllers,
};
