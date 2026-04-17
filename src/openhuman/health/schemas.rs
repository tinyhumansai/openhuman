use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("snapshot")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("snapshot"),
        handler: handle_snapshot,
    }]
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

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_returns_one() {
        assert_eq!(all_controller_schemas().len(), 1);
    }

    #[test]
    fn all_controllers_returns_one() {
        assert_eq!(all_registered_controllers().len(), 1);
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
    fn unknown_function_returns_unknown() {
        let s = schemas("bad");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.namespace, "health");
    }

    #[test]
    fn schemas_and_controllers_match() {
        let s = all_controller_schemas();
        let c = all_registered_controllers();
        assert_eq!(s[0].function, c[0].schema.function);
    }

    #[tokio::test]
    async fn handle_snapshot_returns_json_object() {
        let result = handle_snapshot(Map::new()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_object());
    }

    #[test]
    fn to_json_helper() {
        let outcome = RpcOutcome::single_log(serde_json::json!({"ok": true}), "log");
        assert!(to_json(outcome).is_ok());
    }
}
