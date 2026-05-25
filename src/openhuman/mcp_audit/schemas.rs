use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;

use super::store;
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
        let records = match store::list_writes(&config, &query) {
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn internal_controller_registers_expected_rpc_name() {
        let controllers = all_internal_controllers();
        assert_eq!(controllers.len(), 1);
        assert_eq!(controllers[0].schema.namespace, "mcp_audit");
        assert_eq!(controllers[0].schema.function, "list");
        assert_eq!(controllers[0].rpc_method_name(), "openhuman.mcp_audit_list");
    }

    #[test]
    fn domain_schema_exports_match_internal_controller() {
        let schemas = all_controller_schemas();
        let controllers = all_registered_controllers();

        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].namespace, "mcp_audit");
        assert_eq!(controllers.len(), 1);
        assert_eq!(controllers[0].schema.function, schemas[0].function);
    }

    #[tokio::test]
    async fn handle_list_returns_persisted_audit_records() {
        let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }

        let config = config_rpc::load_config_with_timeout()
            .await
            .expect("config");
        store::record_write(
            &config,
            crate::openhuman::mcp_audit::NewMcpWriteRecord {
                timestamp_ms: 10,
                client_info: "mcp:test".into(),
                tool_name: "memory.store".into(),
                args_summary: json!({ "title": "safe" }),
                resulting_chunk_id: Some("chunk-1".into()),
                success: true,
                error_message: None,
            },
        )
        .expect("record write");

        let value = handle_list(Map::new()).await.expect("handle list");
        let records = value["records"].as_array().expect("records array");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["tool_name"], "memory.store");
        assert_eq!(records[0]["client_info"], "mcp:test");

        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }
}
