//! User-facing agent registry.
//!
//! This high-level domain owns the product registry of agents: shipped
//! defaults, user-authored custom agents, enable/disable state, and tool
//! visibility policy. The lower-level `openhuman::agent` module remains the
//! execution harness and prompt/runtime implementation.

pub mod agents;
mod defaults;
mod ops;
mod rpc;
mod schemas;
pub mod tools;
pub mod types;

pub use defaults::default_agents;
pub use ops::{
    get_agent, list_agents, merge_entries, remove_agent, set_agent_enabled, update_agent,
    upsert_custom_agent,
};
pub use schemas::{
    all_controller_schemas as all_agent_registry_controller_schemas,
    all_registered_controllers as all_agent_registry_registered_controllers,
};
pub use types::{AgentRegistryConfig, AgentRegistryEntry, AgentRegistryPatch, AgentRegistrySource};
