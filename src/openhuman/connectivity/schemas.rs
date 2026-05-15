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
mod tests {
    use super::*;

    #[test]
    fn lists_single_diag_controller() {
        let schemas = all_controller_schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].namespace, "connectivity");
        assert_eq!(schemas[0].function, "diag");
    }

    #[test]
    fn registered_count_matches_schema_count() {
        assert_eq!(
            all_controller_schemas().len(),
            all_registered_controllers().len()
        );
    }

    #[test]
    fn diag_schema_has_no_inputs() {
        assert!(schemas("diag").inputs.is_empty());
    }

    #[test]
    fn diag_schema_outputs_a_diag_payload_field() {
        let s = schemas("diag");
        assert_eq!(s.outputs.len(), 1);
        assert_eq!(s.outputs[0].name, "diag");
    }

    #[test]
    fn unknown_function_returns_unknown_fallback() {
        let s = schemas("no_such");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.namespace, "connectivity");
    }

    #[tokio::test]
    async fn handle_diag_returns_json_object() {
        let value = handle_diag(Map::new()).await.expect("diag handler ok");
        assert!(value.is_object(), "payload should be a JSON object");
    }
}
