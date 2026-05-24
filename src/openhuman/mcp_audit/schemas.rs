use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;

use super::store;
use super::types::McpWriteListQuery;

pub fn all_internal_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schema(),
        handler: handle_list,
    }]
}

fn schema() -> ControllerSchema {
    ControllerSchema {
        namespace: "mcp_audit",
        function: "list",
        description: "List MCP write-tool audit records from local workspace persistence.",
        inputs: vec![
            FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum number of rows to return (default 50, max 500).",
                required: false,
            },
            FieldSchema {
                name: "offset",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Number of rows to skip from the newest-first result set.",
                required: false,
            },
            FieldSchema {
                name: "since_ms",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Only return rows at or after this Unix timestamp in milliseconds.",
                required: false,
            },
            FieldSchema {
                name: "client_filter",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Exact client_info filter, for example `mcp:claude-desktop`.",
                required: false,
            },
            FieldSchema {
                name: "tool_filter",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Exact tool_name filter, for example `memory.store`.",
                required: false,
            },
            FieldSchema {
                name: "success_only",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "When true, only return successful writes.",
                required: false,
            },
        ],
        outputs: vec![FieldSchema {
            name: "records",
            ty: TypeSchema::Array(Box::new(TypeSchema::Ref("McpWriteRecord"))),
            comment: "MCP write audit records ordered by timestamp descending.",
            required: true,
        }],
    }
}

fn handle_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let query = serde_json::from_value::<McpWriteListQuery>(Value::Object(params))
            .map_err(|err| format!("invalid params: {err}"))?;
        let records = store::list_writes(&config, &query).map_err(|err| err.to_string())?;
        serde_json::to_value(records).map_err(|err| err.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_controller_registers_expected_rpc_name() {
        let controllers = all_internal_controllers();
        assert_eq!(controllers.len(), 1);
        assert_eq!(controllers[0].schema.namespace, "mcp_audit");
        assert_eq!(controllers[0].schema.function, "list");
        assert_eq!(controllers[0].rpc_method_name(), "openhuman.mcp_audit_list");
    }
}
