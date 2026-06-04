pub mod engine;
pub mod global;
pub mod prompt;
pub mod reflection;
pub mod reflection_store;
mod schemas;
pub mod situation_report;
pub mod source_chunk;
pub mod store;
pub mod types;

#[cfg(test)]
mod integration_tests;

pub use engine::SubconsciousEngine;
pub use reflection::{Reflection, ReflectionKind, MAX_REFLECTIONS_PER_TICK};
pub use schemas::{
    all_controller_schemas as all_subconscious_controller_schemas,
    all_registered_controllers as all_subconscious_registered_controllers,
};
pub use source_chunk::SourceChunk;
pub use types::{SubconsciousStatus, TickResult};
