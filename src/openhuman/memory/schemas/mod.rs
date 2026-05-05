//! RPC schemas and controller registration for the memory system.
//!
//! This module defines the metadata (schemas) for all memory-related RPC
//! functions and registers their corresponding handlers. It serves as the
//! bridge between the RPC system and the underlying memory operations.
//!
//! Internally the schemas are organised into family submodules that mirror
//! [`crate::openhuman::memory::ops`]:
//!
//! - [`documents`] — doc/namespace/recall/clear schemas + handlers.
//! - [`kv_graph`] — key-value and knowledge-graph schemas + handlers.
//! - [`sync`] — `sync_channel`, `sync_all`, `ingestion_status`.
//! - [`learn`] — `learn_all`.
//! - [`files`] — file-based memory schemas + handlers.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

mod documents;
mod files;
mod kv_graph;
mod learn;
mod sync;

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Returns all controller schemas for the memory system.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    let mut out = Vec::new();
    out.extend(documents::FUNCTIONS.iter().map(|f| schemas(f)));
    out.extend(files::FUNCTIONS.iter().map(|f| schemas(f)));
    out.extend(kv_graph::FUNCTIONS.iter().map(|f| schemas(f)));
    out.extend(sync::FUNCTIONS.iter().map(|f| schemas(f)));
    out.extend(learn::FUNCTIONS.iter().map(|f| schemas(f)));
    out
}

/// Returns all registered controllers for the memory system, mapping schemas to handlers.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    let mut out = Vec::new();
    out.extend(documents::controllers());
    out.extend(files::controllers());
    out.extend(kv_graph::controllers());
    out.extend(sync::controllers());
    out.extend(learn::controllers());
    out
}

/// Defines the schema for a specific memory controller function.
pub fn schemas(function: &str) -> ControllerSchema {
    if let Some(schema) = documents::schema(function) {
        return schema;
    }
    if let Some(schema) = files::schema(function) {
        return schema;
    }
    if let Some(schema) = kv_graph::schema(function) {
        return schema;
    }
    if let Some(schema) = sync::schema(function) {
        return schema;
    }
    if let Some(schema) = learn::schema(function) {
        return schema;
    }
    unknown_schema()
}

fn unknown_schema() -> ControllerSchema {
    ControllerSchema {
        namespace: "memory",
        function: "unknown",
        description: "Unknown memory controller function.",
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

// ---------------------------------------------------------------------------
// Helpers shared by every handler submodule
// ---------------------------------------------------------------------------

pub(super) fn parse_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

pub(super) fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "../schemas_tests.rs"]
mod tests;
