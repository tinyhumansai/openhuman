//! Controller schemas + handlers for the `emergency` namespace.
//! Wires `emergency_stop`, `emergency_resume`, `emergency_status` into the
//! global registry consumed by `src/core/all.rs`.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::ops;

pub fn all_emergency_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("stop"), schemas("resume"), schemas("status")]
}

pub fn all_emergency_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("stop"),
            handler: handle_stop,
        },
        RegisteredController {
            schema: schemas("resume"),
            handler: handle_resume,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "stop" => ControllerSchema {
            namespace: "emergency",
            function: "stop",
            description: "Engage the emergency stop: halt all desktop automation and block further actions until resumed.",
            inputs: vec![FieldSchema {
                name: "reason",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional human-readable reason for the halt.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "state",
                ty: TypeSchema::Ref("HaltState"),
                comment: "Switch snapshot after engaging.",
                required: true,
            }],
        },
        "resume" => ControllerSchema {
            namespace: "emergency",
            function: "resume",
            description: "Clear the emergency stop so automation may resume.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "state",
                ty: TypeSchema::Ref("HaltState"),
                comment: "Switch snapshot after clearing.",
                required: true,
            }],
        },
        "status" => ControllerSchema {
            namespace: "emergency",
            function: "status",
            description: "Read the current emergency-stop switch state.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "state",
                ty: TypeSchema::Ref("HaltState"),
                comment: "Current switch snapshot.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "emergency",
            function: "unknown",
            description: "Unknown emergency function.",
            inputs: vec![],
            outputs: vec![FieldSchema { name: "error", ty: TypeSchema::String, comment: "Schema not defined.", required: true }],
        },
    }
}

fn handle_stop(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let reason = match params.get("reason") {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        };
        to_json(ops::emergency_stop(reason, "user").await)
    })
}

fn handle_resume(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(ops::emergency_resume("user").await) })
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(ops::emergency_status().await) })
}

fn to_json<T: serde::Serialize>(outcome: crate::rpc::RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_controllers_match_schemas() {
        let c = all_emergency_registered_controllers();
        assert_eq!(c.len(), 3);
        let names: Vec<_> = c.iter().map(|c| c.schema.function).collect();
        assert_eq!(names, vec!["stop", "resume", "status"]);
    }

    #[test]
    fn stop_schema_has_optional_reason() {
        let s = schemas("stop");
        assert_eq!(s.namespace, "emergency");
        assert_eq!(s.inputs[0].name, "reason");
        assert!(!s.inputs[0].required);
    }
}
