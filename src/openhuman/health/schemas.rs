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
                FieldSchema {
                    name: "pid",
                    ty: TypeSchema::String,
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
    Box::pin(async { to_json(crate::openhuman::health::rpc::health_snapshot()) })
}

fn handle_system_info(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(crate::openhuman::health::rpc::system_info()) })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_returns_two() {
        assert_eq!(all_controller_schemas().len(), 2);
    }

    #[test]
    fn all_controllers_returns_two() {
        assert_eq!(all_registered_controllers().len(), 2);
    }

    #[test]
    fn snapshot_schema() {
        let s = schemas("snapshot");
        assert_eq!(s.namespace, "health");
        assert_eq!(s.function, "snapshot");
        assert!(s.inputs.is_empty());
        assert!(!s.outputs.is_empty());
    }

    #[test]
    fn system_info_schema() {
        let s = schemas("system_info");
        assert_eq!(s.namespace, "health");
        assert_eq!(s.function, "system_info");
        assert!(s.inputs.is_empty());
        // version, os, arch, pid
        assert_eq!(s.outputs.len(), 4);
    }

    #[test]
    fn unknown_function_returns_unknown() {
        let s = schemas("bad");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.namespace, "health");
    }

    #[test]
    fn schemas_and_controllers_match() {
        let s = all_controller_schemas();
        let c = all_registered_controllers();
        assert_eq!(s.len(), c.len());
        for (schema, controller) in s.iter().zip(c.iter()) {
            assert_eq!(schema.function, controller.schema.function);
        }
    }

    #[tokio::test]
    async fn handle_snapshot_returns_json_object() {
        let result = handle_snapshot(Map::new()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_object());
    }

    #[tokio::test]
    async fn handle_system_info_returns_json_object() {
        let result = handle_system_info(Map::new()).await;
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.is_object());
        assert!(json["version"].as_str().is_some());
        assert!(json["os"].as_str().is_some());
        assert!(json["arch"].as_str().is_some());
        assert!(json["pid"].as_u64().is_some());
    }

    #[test]
    fn to_json_helper() {
        let outcome = RpcOutcome::single_log(serde_json::json!({"ok": true}), "log");
        assert!(to_json(outcome).is_ok());
    }
}
