//! Unified read-only tool registry for discovery across OpenHuman tool surfaces.

pub mod ops;
mod schemas;
mod types;

pub use ops::{get_tool, list_tools, registry_entries};
pub use schemas::{
    all_controller_schemas as all_tool_registry_controller_schemas,
    all_registered_controllers as all_tool_registry_registered_controllers,
};
pub use types::{ToolRegistryEntry, ToolRegistryHealth, ToolRegistryList, ToolRegistryTransport};
