//! Controller-registry schemas for `openhuman.memory_sync_status_list`.
//!
//! Wired into `src/core/all.rs` via the `all_memory_sync_status_*`
//! re-exports in `super::mod`. Single method now — see `rpc.rs` for the
//! simplified design (#1136 rewrite).
//!
//! The `MemorySyncStatus` this schema names as its output type is
//! [`super::MemorySyncStatus`] — the host's own wire type. It was the engine's
//! until #5560; the row now comes from the driver's `sync_statuses` and is
//! carried across by `rpc::into_wire`, so the schema and the type it advertises
//! live in the same module.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::ops::load_config_with_timeout;
use crate::rpc::RpcOutcome;

use super::rpc;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("status_list")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("status_list"),
        handler: handle_status_list,
    }]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "status_list" => ControllerSchema {
            namespace: "memory_sync",
            function: "status_list",
            description:
                "List one row per data-source kind that has chunks in the memory tree. Counts \
                 are pulled live from `mem_tree_chunks` so the snapshot is always exact.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "statuses",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("MemorySyncStatus"))),
                comment: "One row per `source_kind` with chunk count + freshness label.",
                required: true,
            }],
        },
        other => panic!("unknown memory_sync schema function: {other}"),
    }
}

fn handle_status_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config_with_timeout().await?;
        to_json(rpc::status_list_rpc(&config).await?)
    })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
