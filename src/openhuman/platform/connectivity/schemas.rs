//! Controller schemas + RPC handlers for the `connectivity` namespace.
//!
//! Surface is intentionally minimal — a single `connectivity_diag` read-only
//! controller. Restart / mutate operations live in the Tauri shell (see
//! `restart_core_process` in `app/src-tauri/src/lib.rs`) because they touch
//! the host process tree and can't be answered from inside the sidecar
//! itself.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("diag")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("diag"),
        handler: handle_diag,
    }]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "diag" => ControllerSchema {
            namespace: "connectivity",
            function: "diag",
            description: "Return a diagnostic snapshot of the local sidecar's reachability \
                 and the backend Socket.IO connection state. Cheap — safe to poll.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "diag",
                ty: TypeSchema::Json,
                comment: "Snapshot containing socket_state, last_ws_error, \
                          sidecar_pid, listen_port, listen_port_in_use.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "connectivity",
            function: "unknown",
            description: "Unknown connectivity controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_diag(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::diag().await?.into_cli_compatible_json() })
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
