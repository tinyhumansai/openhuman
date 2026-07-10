//! RPC endpoints for the `subconscious_triggers` domain.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

use super::ops;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("status")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("status"),
        handler: handle_status,
    }]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "subconscious_triggers",
            function: "status",
            description: "Status of the event-driven subconscious trigger pipeline.",
            inputs: vec![],
            outputs: vec![field(
                "result",
                TypeSchema::Json,
                "Trigger pipeline status.",
            )],
        },
        _other => ControllerSchema {
            namespace: "subconscious_triggers",
            function: "unknown",
            description: "Unknown subconscious_triggers function.",
            inputs: vec![],
            outputs: vec![field("error", TypeSchema::String, "Error details.")],
        },
    }
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let status = ops::build_status().await?;
        to_json(RpcOutcome::single_log(
            status,
            "subconscious_triggers status",
        ))
    })
}

fn field(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
