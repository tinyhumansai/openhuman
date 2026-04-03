//! Heartbeat loop — periodic scheduler that delegates to the subconscious
//! engine for task-driven evaluation via local model inference.
//!
//! HEARTBEAT.md in the workspace defines the task checklist.
//! The subconscious engine evaluates tasks against workspace state
//! (memory, graph, skills) using the local Ollama model.

pub mod engine;
mod schemas;
pub use schemas::{
    all_controller_schemas as all_heartbeat_controller_schemas,
    all_registered_controllers as all_heartbeat_registered_controllers,
};
