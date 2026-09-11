use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_announcements_controller_schemas() -> Vec<ControllerSchema> {
    vec![announcements_schemas("announcements_get_latest")]
}

pub fn all_announcements_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: announcements_schemas("announcements_get_latest"),
        handler: handle_announcements_get_latest,
    }]
}

pub fn announcements_schemas(function: &str) -> ControllerSchema {
    match function {
        "announcements_get_latest" => ControllerSchema {
            namespace: "announcements",
            function: "get_latest",
            description: "Fetch the latest active announcement for the signed-in user (or null).",
            inputs: vec![],
            outputs: vec![json_output(
                "announcement",
                "Latest active announcement from backend /announcements/latest, or null when none.",
            )],
        },
        _ => ControllerSchema {
            namespace: "announcements",
            function: "unknown",
            description: "Unknown announcements controller.",
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

fn handle_announcements_get_latest(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::hosted::announcements::get_latest_announcement(&config).await?)
    })
}

fn to_json(outcome: RpcOutcome<Value>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
