//! Controller-registry schemas for `openhuman.memory_sync_status_list`.
//!
//! Wired into `src/core/all.rs` via the `all_memory_sync_status_*`
//! re-exports in `super::mod`. Single method now — see `rpc.rs` and
//! `types.rs` for the simplified design (#1136 rewrite).

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

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

fn handle_status_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(rpc::handle_status_list(params))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_only_status_list() {
        let regs = all_registered_controllers();
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0].schema.function, "status_list");
    }

    #[test]
    fn schema_status_list_has_no_inputs_and_one_output() {
        let s = schemas("status_list");
        assert_eq!(s.namespace, "memory_sync");
        assert_eq!(s.function, "status_list");
        assert!(s.inputs.is_empty());
        assert_eq!(s.outputs.len(), 1);
        assert_eq!(s.outputs[0].name, "statuses");
    }

    #[test]
    #[should_panic(expected = "unknown memory_sync schema function")]
    fn schemas_panics_on_unknown_function() {
        schemas("nope");
    }
}
