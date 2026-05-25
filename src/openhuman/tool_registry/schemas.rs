use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

/// Declared controller schemas for the `tool_registry` namespace.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("list"), schemas("get"), schemas("diagnostics")]
}

/// Registered controller handlers for the `tool_registry` namespace.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("get"),
            handler: handle_get,
        },
        RegisteredController {
            schema: schemas("diagnostics"),
            handler: handle_diagnostics,
        },
    ]
}

/// Return the schema for one `tool_registry` function.
pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "tool_registry",
            function: "list",
            description: "List the unified read-only tool registry across MCP stdio tools and controller-backed tools.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "tools",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Registry entries with tool id, version, route, input/output schemas, tags, enabled state, allowed agents, and health.",
                required: true,
            }],
        },
        "get" => ControllerSchema {
            namespace: "tool_registry",
            function: "get",
            description: "Look up one tool registry entry by stable tool_id.",
            inputs: vec![FieldSchema {
                name: "tool_id",
                ty: TypeSchema::String,
                comment: "Stable registry id, for example `memory.search` or `tools.web_search`.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "tool",
                ty: TypeSchema::Json,
                comment: "One registry entry.",
                required: true,
            }],
        },
        "diagnostics" => ControllerSchema {
            namespace: "tool_registry",
            function: "diagnostics",
            description: "Return redacted tool inventory and policy visibility diagnostics.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "diagnostics",
                ty: TypeSchema::Json,
                comment: "Counts and redacted tool ids useful for policy/conformance checks.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "tool_registry",
            function: "unknown",
            description: "Unknown tool registry controller function.",
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

fn handle_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "[tool_registry] rpc list requested param_count={}",
            params.len()
        );
        to_json(crate::openhuman::tool_registry::ops::list_tools())
    })
}

fn handle_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let tool_id = required_tool_id(&params)?;
        log::debug!("[tool_registry] rpc get requested tool_id={tool_id}");
        to_json(crate::openhuman::tool_registry::ops::get_tool(tool_id)?)
    })
}

fn handle_diagnostics(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "[tool_registry] rpc diagnostics requested param_count={}",
            params.len()
        );
        let result = to_json(crate::openhuman::tool_registry::ops::diagnostics());
        log::debug!(
            "[tool_registry] rpc diagnostics completed success={}",
            result.is_ok()
        );
        result
    })
}

fn required_tool_id(params: &Map<String, Value>) -> Result<&str, String> {
    params
        .get("tool_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "tool_id must be a non-empty string".to_string())
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schemas_cover_registered_controllers() {
        let schemas = all_controller_schemas();
        let controllers = all_registered_controllers();

        assert_eq!(schemas.len(), 3);
        assert_eq!(controllers.len(), 3);
        assert_eq!(schemas[0].function, controllers[0].schema.function);
        assert_eq!(schemas[1].function, controllers[1].schema.function);
        assert_eq!(schemas[2].function, controllers[2].schema.function);
    }

    #[test]
    fn list_schema_has_no_inputs() {
        let schema = schemas("list");
        assert_eq!(schema.namespace, "tool_registry");
        assert_eq!(schema.function, "list");
        assert!(schema.inputs.is_empty());
        assert_eq!(schema.outputs[0].name, "tools");
    }

    #[test]
    fn get_schema_requires_tool_id() {
        let schema = schemas("get");
        assert_eq!(schema.inputs[0].name, "tool_id");
        assert!(schema.inputs[0].required);
    }

    #[test]
    fn diagnostics_schema_has_no_inputs() {
        let schema = schemas("diagnostics");
        assert_eq!(schema.namespace, "tool_registry");
        assert_eq!(schema.function, "diagnostics");
        assert!(schema.inputs.is_empty());
        assert_eq!(schema.outputs[0].name, "diagnostics");
    }

    #[test]
    fn required_tool_id_rejects_wrong_type() {
        let mut params = Map::new();
        params.insert("tool_id".to_string(), json!(10));

        let err = required_tool_id(&params).expect_err("numeric id should fail");
        assert!(err.contains("non-empty string"));
    }

    #[tokio::test]
    async fn handle_list_returns_registry_object() {
        let value = handle_list(Map::new()).await.expect("list json");
        let tools = value
            .get("tools")
            .and_then(Value::as_array)
            .expect("tools array");
        assert!(tools
            .iter()
            .any(|tool| { tool.get("tool_id").and_then(Value::as_str) == Some("memory.search") }));
    }

    #[tokio::test]
    async fn handle_get_returns_one_registry_entry() {
        let mut params = Map::new();
        params.insert("tool_id".to_string(), json!("tools.web_search"));

        let value = handle_get(params).await.expect("get json");
        assert_eq!(
            value.get("tool_id").and_then(Value::as_str),
            Some("tools.web_search")
        );
    }

    #[tokio::test]
    async fn handle_diagnostics_returns_counts() {
        let value = handle_diagnostics(Map::new())
            .await
            .expect("diagnostics json");
        let diagnostics = value.get("diagnostics").unwrap_or(&value);
        assert!(diagnostics
            .get("total_tools")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0));
        assert!(diagnostics
            .get("policy_surfaces")
            .and_then(Value::as_array)
            .is_some());
    }
}
