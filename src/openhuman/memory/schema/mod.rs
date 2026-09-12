//! The memory subsystem's JSON-RPC controller schemas.
//!
//! Stayed in the host through the extraction: every item here names
//! `ControllerSchema`, `FieldSchema` or `TypeSchema`, and controller
//! registration is host surface by the tinymemory README's split.

mod definitions;
mod handlers;
mod registry;

pub use definitions::schemas;
pub use registry::{all_controller_schemas, all_registered_controllers};

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
