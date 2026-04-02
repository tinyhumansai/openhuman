use serde_json::{Map, Value};

use super::global::get_or_init_engine;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("status"), schemas("trigger"), schemas("actions")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("trigger"),
            handler: handle_trigger,
        },
        RegisteredController {
            schema: schemas("actions"),
            handler: handle_actions,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "subconscious",
            function: "status",
            description: "Get the current subconscious loop status.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Loop status including last tick, decision counts.",
                required: true,
            }],
        },
        "trigger" => ControllerSchema {
            namespace: "subconscious",
            function: "trigger",
            description: "Manually trigger a subconscious tick.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Tick result with decision, reason, and actions.",
                required: true,
            }],
        },
        "actions" => ControllerSchema {
            namespace: "subconscious",
            function: "actions",
            description: "List stored subconscious actions/notifications.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum number of actions to return (default: 20).",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "actions",
                ty: TypeSchema::Json,
                comment: "Array of stored action entries with timestamps.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "subconscious",
            function: "unknown",
            description: "Unknown subconscious controller function.",
            inputs: vec![FieldSchema {
                name: "function",
                ty: TypeSchema::String,
                comment: "Unknown function requested.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Error details.",
                required: true,
            }],
        },
    }
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let lock = get_or_init_engine().await?;
        let guard = lock.lock().await;
        let engine = guard
            .as_ref()
            .ok_or_else(|| "engine not initialized".to_string())?;
        let status = engine.status().await;
        to_json(RpcOutcome::single_log(status, "subconscious status"))
    })
}

fn handle_trigger(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let lock = get_or_init_engine().await?;
        let guard = lock.lock().await;
        let engine = guard
            .as_ref()
            .ok_or_else(|| "engine not initialized".to_string())?;
        let result = engine.tick().await.map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(
            result,
            "subconscious tick completed",
        ))
    })
}

fn handle_actions(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| format!("load config: {e}"))?;

        let memory =
            crate::openhuman::memory::MemoryClient::from_workspace_dir(config.workspace_dir)
                .map_err(|e| format!("memory client: {e}"))?;

        let entries = memory
            .kv_list_namespace("subconscious")
            .await
            .map_err(|e| format!("list actions: {e}"))?;

        let mut actions: Vec<Value> = Vec::new();
        for entry in entries {
            let key = entry.get("key").and_then(|v| v.as_str()).unwrap_or("");
            if !key.starts_with("actions:") {
                continue;
            }
            let timestamp = key
                .strip_prefix("actions:")
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            let value = entry.get("value").and_then(|v| v.as_str()).unwrap_or("[]");
            let parsed_actions: Value =
                serde_json::from_str(value).unwrap_or(Value::String(value.to_string()));

            actions.push(serde_json::json!({
                "tick_at": timestamp,
                "actions": parsed_actions,
            }));
        }

        actions.sort_by(|a, b| {
            let ta = a.get("tick_at").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let tb = b.get("tick_at").and_then(|v| v.as_f64()).unwrap_or(0.0);
            tb.partial_cmp(&ta).unwrap_or(std::cmp::Ordering::Equal)
        });

        actions.truncate(limit);

        to_json(RpcOutcome::single_log(
            serde_json::json!({ "entries": actions, "count": actions.len() }),
            "subconscious actions listed",
        ))
    })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
