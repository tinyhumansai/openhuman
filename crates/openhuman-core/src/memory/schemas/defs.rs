//! [`ControllerSchema`] definitions for the `memory` namespace
//! (`openhuman.memory_*`, see `docs/specs/memory-v2.md`).

/// Every function of the namespace, in spec order.
pub const FUNCTIONS: [&str; 35] = [
    "engines_list",
    "engine_get",
    "engine_set",
    "policy_get",
    "policy_set",
    "pack_preview",
    "recall",
    "fetch",
    "learn",
    "forget",
    "erase_all",
    "items_list",
    "explore",
    "items_get",
    "agents_list",
    "conversations_backfill_status",
    "conversations_backfill_start",
    "brain_sources",
    "brain_search",
    "brain_ingest",
    "brain_forget",
    "sources_list",
    "sources_add",
    "sources_remove",
    "sources_sync",
    "jobs_list",
    "jobs_run",
    "import_scan",
    "import_start",
    "import_status",
    "import_retry_failed",
    "migration_scan",
    "migration_start",
    "migration_status",
    "migration_retry",
];

mod brain;
mod common;
mod conversations;
mod engine;
mod import;
mod jobs;
mod migration;
mod policy;
mod retrieval;
mod sources;

use crate::core::{ControllerSchema, TypeSchema};
use common::req;

/// The schema of `function`; an unknown function gets the namespace's
/// `unknown` placeholder, as every namespace does.
pub fn schema(function: &str) -> ControllerSchema {
    engine::schema(function)
        .or_else(|| policy::schema(function))
        .or_else(|| retrieval::schema(function))
        .or_else(|| conversations::schema(function))
        .or_else(|| brain::schema(function))
        .or_else(|| sources::schema(function))
        .or_else(|| jobs::schema(function))
        .or_else(|| import::schema(function))
        .or_else(|| migration::schema(function))
        .unwrap_or_else(|| ControllerSchema {
            namespace: "memory",
            function: "unknown",
            description: "Unknown memory controller function.",
            inputs: vec![],
            outputs: vec![req("error", TypeSchema::String, "Lookup error details.")],
        })
}
