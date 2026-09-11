//! Schema definitions for every `memory_tree` JSON-RPC method.
//!
//! The [`schemas`] function is the single source of truth for each
//! controller's input/output field descriptions. Handlers delegate to
//! [`super::handlers`]; the registry lists are in [`super::registry`].

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub(crate) const NAMESPACE: &str = "memory_tree";

/// Lookup the [`ControllerSchema`] for a single `memory_tree` function name.
#[path = "schema_schema_part_01.rs"]
mod schema_schema_part_01;
#[path = "schema_schema_part_02.rs"]
mod schema_schema_part_02;

pub fn schemas(function: &str) -> ControllerSchema {
    if let Some(schema) = schema_schema_part_01::lookup(function) {
        return schema;
    }
    if let Some(schema) = schema_schema_part_02::lookup(function) {
        return schema;
    }
    ControllerSchema {
        namespace: NAMESPACE,
        function: "unknown",
        description: "Unknown memory_tree controller function.",
        inputs: vec![FieldSchema {
            name: "function",
            ty: TypeSchema::String,
            comment: "Unknown function requested for schema lookup.",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "error",
            ty: TypeSchema::String,
            comment: "Lookup error details.",
            required: true,
        }],
    }
}
