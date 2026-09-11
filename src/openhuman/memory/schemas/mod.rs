//! RPC schemas and controller registration for the memory system.
//!
//! This module defines the metadata (schemas) for all memory-related RPC
//! functions and registers their corresponding handlers. It serves as the
//! bridge between the RPC system and the underlying memory operations.
//!
//! Internally the schemas are organised into family submodules that mirror
//! [`crate::openhuman::memory::ops`]:
//!
//! - [`documents`] — doc/namespace/recall/clear schemas + handlers. Partitioned
//!   three ways by capability family (core+recall / documents / ingest); see
//!   that module's header for why.
//! - [`kv_graph`] — key-value and knowledge-graph schemas + handlers.
//! - [`sync`] — `sync_channel`, `sync_all`, `ingestion_status`.
//! - [`learn`] — `learn_all`.
//! - [`provider`] — `provider_status` (the bound memory driver).
//! - [`files`] — file-based memory schemas + handlers.
//! - [`tool_memory`] — tool-scoped memory rules (#1400).
//!
//! Every family publishes its own `all_<family>_controller_schemas()` /
//! `all_<family>_registered_controllers()` pair; [`all_controller_schemas`] and
//! [`all_registered_controllers`] are thin fan-outs over the **nine** parts, in
//! a fixed order. The split exists so `core::all` can register (or decline to
//! register) one capability family at a time — the parts themselves skip
//! nothing.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

mod documents;
mod files;
mod kv_graph;
mod learn;
mod provider;
mod sync;
mod tool_memory;

// ---------------------------------------------------------------------------
// Per-family entry points
// ---------------------------------------------------------------------------
//
// Each capability family exposes its own `all_<family>_controller_schemas()` /
// `all_<family>_registered_controllers()` pair so a caller can register (or, in
// a later slice, decline to register) one family at a time. The aggregators
// below fan these out in a fixed order — core_recall, documents, ingest, files,
// kv_graph, sync, learn, provider, tool_memory. Do not reorder: `src/core/all.rs`
// pushes the nine parts in exactly this sequence and
// `registered_controller_order_is_pinned_to_the_capability_partition_snapshot` in
// `schemas_tests.rs` fails if it drifts.

/// Controller schemas for the mandatory core + recall surface. Never
/// capability-gated — see [`documents`]'s header.
pub fn all_core_recall_controller_schemas() -> Vec<ControllerSchema> {
    documents::FUNCTIONS_CORE_RECALL
        .iter()
        .map(|f| schemas(f))
        .collect()
}

/// Registered controllers for the mandatory core + recall surface.
pub fn all_core_recall_registered_controllers() -> Vec<RegisteredController> {
    documents::controllers_core_recall()
}

/// Controller schemas for the namespace-document tier
/// (`Capability::Documents`).
pub fn all_documents_controller_schemas() -> Vec<ControllerSchema> {
    documents::FUNCTIONS_DOCUMENTS
        .iter()
        .map(|f| schemas(f))
        .collect()
}

/// Registered controllers for the namespace-document tier.
pub fn all_documents_registered_controllers() -> Vec<RegisteredController> {
    documents::controllers_documents()
}

/// Controller schemas for driver-owned ingestion (`Capability::Ingest`).
pub fn all_ingest_controller_schemas() -> Vec<ControllerSchema> {
    documents::FUNCTIONS_INGEST
        .iter()
        .map(|f| schemas(f))
        .collect()
}

/// Registered controllers for driver-owned ingestion.
pub fn all_ingest_registered_controllers() -> Vec<RegisteredController> {
    documents::controllers_ingest()
}

/// Controller schemas for the file-backed memory family.
pub fn all_files_controller_schemas() -> Vec<ControllerSchema> {
    files::FUNCTIONS.iter().map(|f| schemas(f)).collect()
}

/// Registered controllers for the file-backed memory family.
pub fn all_files_registered_controllers() -> Vec<RegisteredController> {
    files::controllers()
}

/// Controller schemas for the key-value + knowledge-graph family.
pub fn all_kv_graph_controller_schemas() -> Vec<ControllerSchema> {
    kv_graph::FUNCTIONS.iter().map(|f| schemas(f)).collect()
}

/// Registered controllers for the key-value + knowledge-graph family.
pub fn all_kv_graph_registered_controllers() -> Vec<RegisteredController> {
    kv_graph::controllers()
}

/// Controller schemas for the channel/ingestion sync family.
pub fn all_sync_controller_schemas() -> Vec<ControllerSchema> {
    sync::FUNCTIONS.iter().map(|f| schemas(f)).collect()
}

/// Registered controllers for the channel/ingestion sync family.
pub fn all_sync_registered_controllers() -> Vec<RegisteredController> {
    sync::controllers()
}

/// Controller schemas for the `learn_all` family.
pub fn all_learn_controller_schemas() -> Vec<ControllerSchema> {
    learn::FUNCTIONS.iter().map(|f| schemas(f)).collect()
}

/// Registered controllers for the `learn_all` family.
pub fn all_learn_registered_controllers() -> Vec<RegisteredController> {
    learn::controllers()
}

/// Controller schemas for the bound-driver status family.
///
/// This family is the one that *reports* the driver's advertised capability
/// set, so it must never itself be gated on a capability — doing so would be
/// self-referential and would blind the UI, which reads the capability set
/// from `<subsystem>_status` (kernel.md §3.3).
pub fn all_provider_controller_schemas() -> Vec<ControllerSchema> {
    provider::FUNCTIONS.iter().map(|f| schemas(f)).collect()
}

/// Registered controllers for the bound-driver status family.
///
/// Never gated — see [`all_provider_controller_schemas`].
pub fn all_provider_registered_controllers() -> Vec<RegisteredController> {
    provider::controllers()
}

/// Controller schemas for the tool-scoped memory family (#1400).
pub fn all_tool_memory_controller_schemas() -> Vec<ControllerSchema> {
    tool_memory::FUNCTIONS.iter().map(|f| schemas(f)).collect()
}

/// Registered controllers for the tool-scoped memory family (#1400).
pub fn all_tool_memory_registered_controllers() -> Vec<RegisteredController> {
    tool_memory::controllers()
}

// ---------------------------------------------------------------------------
// Aggregated entry points
// ---------------------------------------------------------------------------

/// Returns all controller schemas for the memory system.
///
/// Thin fan-out over the nine per-family pairs above, in their pinned order.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    let mut out = Vec::new();
    out.extend(all_core_recall_controller_schemas());
    out.extend(all_documents_controller_schemas());
    out.extend(all_ingest_controller_schemas());
    out.extend(all_files_controller_schemas());
    out.extend(all_kv_graph_controller_schemas());
    out.extend(all_sync_controller_schemas());
    out.extend(all_learn_controller_schemas());
    out.extend(all_provider_controller_schemas());
    out.extend(all_tool_memory_controller_schemas());
    out
}

/// Returns all registered controllers for the memory system, mapping schemas to handlers.
///
/// Thin fan-out over the nine per-family pairs above, in their pinned order.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    let mut out = Vec::new();
    out.extend(all_core_recall_registered_controllers());
    out.extend(all_documents_registered_controllers());
    out.extend(all_ingest_registered_controllers());
    out.extend(all_files_registered_controllers());
    out.extend(all_kv_graph_registered_controllers());
    out.extend(all_sync_registered_controllers());
    out.extend(all_learn_registered_controllers());
    out.extend(all_provider_registered_controllers());
    out.extend(all_tool_memory_registered_controllers());
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
    if let Some(schema) = provider::schema(function) {
        return schema;
    }
    if let Some(schema) = tool_memory::schema(function) {
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
