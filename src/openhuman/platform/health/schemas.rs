use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("snapshot"), schemas("system_info")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("snapshot"),
            handler: handle_snapshot,
        },
        RegisteredController {
            schema: schemas("system_info"),
            handler: handle_system_info,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "snapshot" => ControllerSchema {
            namespace: "health",
            function: "snapshot",
            description: "Return process and component health snapshot.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "snapshot",
                ty: TypeSchema::Json,
                comment: "Serialized health snapshot payload.",
                required: true,
            }],
        },
        "system_info" => ControllerSchema {
            namespace: "health",
            function: "system_info",
            description:
                "Return static system information: app version, OS, architecture, and PID.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "version",
                    ty: TypeSchema::String,
                    comment: "Running core binary version (CARGO_PKG_VERSION).",
                    required: true,
                },
                FieldSchema {
                    name: "os",
                    ty: TypeSchema::String,
                    comment: "Host operating system name (linux, macos, windows, …).",
                    required: true,
                },
                FieldSchema {
                    name: "arch",
                    ty: TypeSchema::String,
                    comment: "CPU architecture (x86_64, aarch64, …).",
                    required: true,
                },
                // `SystemInfo.pid` is a `u32` and serializes as a JSON number,
                // so the declaration is `U64`, not `String` (#6074). Output
                // fields are not validated at dispatch, but this schema is what
                // generates the frontend's RPC types and what the agent tool
                // surface advertises to a model, so a wrong type here is wrong
                // at both consumers.
                FieldSchema {
                    name: "pid",
                    ty: TypeSchema::U64,
                    comment: "Current process ID.",
                    required: true,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: "health",
            function: "unknown",
            description: "Unknown health controller function.",
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

fn handle_snapshot(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(crate::openhuman::platform::health::rpc::health_snapshot()) })
}

fn handle_system_info(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(crate::openhuman::platform::health::rpc::system_info()) })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
