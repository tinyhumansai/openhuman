//! Safe, user-facing agent library projection.

mod ops;
mod types;

pub use ops::{list_definition_metadata, metadata_from_definition};
pub use types::{AgentDefinitionDisplay, AgentDefinitionModel, AgentDefinitionSource};
