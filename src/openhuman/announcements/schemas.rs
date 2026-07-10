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
        to_json(crate::openhuman::announcements::get_latest_announcement(&config).await?)
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
mod tests {
    use super::*;

    #[test]
    fn get_latest_schema_has_expected_namespace_and_no_inputs() {
        let schema = announcements_schemas("announcements_get_latest");
        assert_eq!(schema.namespace, "announcements");
        assert_eq!(schema.function, "get_latest");
        assert!(schema.inputs.is_empty());
        assert_eq!(schema.outputs.len(), 1);
        assert_eq!(schema.outputs[0].name, "announcement");
    }

    #[test]
    fn unknown_function_falls_back() {
        let schema = announcements_schemas("nope");
        assert_eq!(schema.function, "unknown");
    }

    #[test]
    fn registers_exactly_one_controller() {
        assert_eq!(all_announcements_registered_controllers().len(), 1);
        assert_eq!(all_announcements_controller_schemas().len(), 1);
    }
}
