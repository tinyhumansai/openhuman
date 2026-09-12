use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;

use super::types::McpWriteListQuery;

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => schema(),
        other => panic!("unknown mcp_audit controller schema `{other}`"),
    }
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("list")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    all_internal_controllers()
}

pub fn all_internal_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("list"),
        handler: handle_list,
    }]
}

fn schema() -> ControllerSchema {
    ControllerSchema {
        namespace: "mcp_audit",
        function: "list",
        description: "List MCP write-tool audit records, including successful writes and rejected or failed write attempts, from local workspace persistence.",
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
                comment: "When true, only return rows where the write attempt succeeded.",
                required: false,
            },
        ],
        outputs: vec![FieldSchema {
            name: "records",
            ty: TypeSchema::Array(Box::new(TypeSchema::Ref("McpWriteRecord"))),
            comment: "MCP write attempt audit records ordered by timestamp descending.",
            required: true,
        }],
    }
}

fn handle_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[mcp_audit] handle_list enter params={params:?}");
        log::trace!("[mcp_audit] handle_list loading config");
        let config = match config_rpc::load_config_with_timeout().await {
            Ok(config) => {
                log::trace!(
                    "[mcp_audit] handle_list config loaded workspace={}",
                    config.workspace_dir.display()
                );
                config
            }
            Err(err) => {
                log::warn!("[mcp_audit] handle_list config load failed error={err}");
                return Err(err);
            }
        };

        let query = match serde_json::from_value::<McpWriteListQuery>(Value::Object(params)) {
            Ok(query) => {
                log::trace!("[mcp_audit] handle_list parsed query={query:?}");
                query
            }
            Err(err) => {
                log::warn!("[mcp_audit] handle_list invalid params error={err}");
                return Err(format!("invalid params: {err}"));
            }
        };

        log::trace!(
            "[mcp_audit] handle_list querying store workspace={} query={query:?}",
            config.workspace_dir.display()
        );
        let records = match crate::openhuman::mcp::audit::list_writes(&config, &query) {
            Ok(records) => {
                log::trace!(
                    "[mcp_audit] handle_list store success records={}",
                    records.len()
                );
                records
            }
            Err(err) => {
                log::warn!("[mcp_audit] handle_list store failed query={query:?} error={err}");
                return Err(err.to_string());
            }
        };

        let count = records.len();
        let records_value = serde_json::to_value(records).map_err(|err| {
            log::warn!("[mcp_audit] handle_list serialize response failed error={err}");
            err.to_string()
        })?;
        log::debug!("[mcp_audit] handle_list exit records={count}");
        Ok(serde_json::json!({ "records": records_value }))
    })
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
