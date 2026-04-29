//! Controller schemas + JSON-RPC handler dispatch for the Slack
//! ingestion path.
//!
//! Registered JSON-RPC methods (namespace `slack_memory`):
//! - `openhuman.slack_memory_sync_trigger` — run the Composio-backed
//!   `SlackProvider::sync()` once per active Slack connection.
//! - `openhuman.slack_memory_sync_status`  — list per-connection
//!   cursor + dedup + budget state.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::slack_ingestion::rpc as slack_rpc;
use crate::rpc::RpcOutcome;

const NAMESPACE: &str = "slack_memory";

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("sync_trigger"), schemas("sync_status")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("sync_trigger"),
            handler: handle_sync_trigger,
        },
        RegisteredController {
            schema: schemas("sync_status"),
            handler: handle_sync_status,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "sync_trigger" => ControllerSchema {
            namespace: NAMESPACE,
            function: "sync_trigger",
            description: "Run the Composio-backed Slack provider sync once per active \
                 Slack connection. When `connection_id` is provided, only that one \
                 connection is synced.",
            inputs: vec![FieldSchema {
                name: "connection_id",
                ty: TypeSchema::String,
                comment: "Optional — restrict the trigger to one Composio connection id.",
                required: false,
            }],
            outputs: vec![
                FieldSchema {
                    name: "outcomes",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("SyncOutcome"))),
                    comment: "Per-connection SyncOutcome records returned by SlackProvider::sync.",
                    required: true,
                },
                FieldSchema {
                    name: "connections_considered",
                    ty: TypeSchema::I64,
                    comment: "Number of active Slack connections evaluated in this call.",
                    required: true,
                },
                FieldSchema {
                    name: "connections_synced",
                    ty: TypeSchema::I64,
                    comment: "Number of connections whose sync completed without error.",
                    required: true,
                },
            ],
        },
        "sync_status" => ControllerSchema {
            namespace: NAMESPACE,
            function: "sync_status",
            description: "List per-connection Slack ingestion state (cursors, synced-id \
                 count, daily budget).",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "connections",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("ConnectionStatus"))),
                comment: "One row per active Slack Composio connection.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: NAMESPACE,
            function: "unknown",
            description: "Unknown slack_memory controller function.",
            inputs: vec![FieldSchema {
                name: "function",
                ty: TypeSchema::String,
                comment: "Unknown function requested for schema lookup.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_sync_trigger(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<slack_rpc::SyncTriggerRequest>(Value::Object(params))?;
        to_json(slack_rpc::sync_trigger_rpc(&config, req).await?)
    })
}

fn handle_sync_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<slack_rpc::SyncStatusRequest>(Value::Object(params))?;
        to_json(slack_rpc::sync_status_rpc(&config, req).await?)
    })
}

fn parse_value<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
