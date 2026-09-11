use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("report"), schemas("models")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("report"),
            handler: handle_report,
        },
        RegisteredController {
            schema: schemas("models"),
            handler: handle_models,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "report" => ControllerSchema {
            namespace: "doctor",
            function: "report",
            description: "Run diagnostics for workspace and runtime configuration.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "report",
                ty: TypeSchema::Ref("DoctorReport"),
                comment: "Aggregated diagnostics report.",
                required: true,
            }],
        },
        "models" => ControllerSchema {
            namespace: "doctor",
            function: "models",
            description: "Probe provider model availability and auth status.",
            inputs: vec![FieldSchema {
                name: "use_cache",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "Reuse cached provider metadata when available.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "report",
                ty: TypeSchema::Ref("ModelProbeReport"),
                comment: "Model probe summary grouped by provider.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "doctor",
            function: "unknown",
            description: "Unknown doctor controller function.",
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

fn handle_report(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::platform::doctor::rpc::doctor_report(&config).await?)
    })
}

fn handle_models(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let use_cache = read_optional::<bool>(&params, "use_cache")?.unwrap_or(true);
        to_json(crate::openhuman::platform::doctor::rpc::doctor_models(&config, use_cache).await?)
    })
}

fn read_optional<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<T>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| format!("invalid '{key}': {e}")),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
