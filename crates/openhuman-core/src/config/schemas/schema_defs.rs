//! Controller schema definitions for the `config` namespace, grouped by
//! settings area. `schemas` looks a function up across every group.

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

mod agent;
mod inference;
mod integrations;
mod workspace;

#[cfg(test)]
#[path = "schema_defs_tests.rs"]
mod tests;

pub fn schemas(function: &str) -> ControllerSchema {
    inference::lookup(function)
        .or_else(|| agent::lookup(function))
        .or_else(|| workspace::lookup(function))
        .or_else(|| integrations::lookup(function))
        .unwrap_or_else(|| ControllerSchema {
            namespace: "config",
            function: "unknown",
            description: "Unknown config controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        })
}
