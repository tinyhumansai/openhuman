//! JSON-RPC controller surface for AgentBox.
//!
//! Read-only today: exposes `openhuman.agentbox_status` so the desktop control
//! panel can show whether the marketplace adapter is active and how GMI MaaS is
//! wired — without ever surfacing the API key.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

use super::status::agentbox_status;

pub fn all_agentbox_controller_schemas() -> Vec<ControllerSchema> {
    vec![agentbox_schemas("agentbox_status")]
}

pub fn all_agentbox_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: agentbox_schemas("agentbox_status"),
        handler: handle_agentbox_status,
    }]
}

pub fn agentbox_schemas(function: &str) -> ControllerSchema {
    match function {
        "agentbox_status" => ControllerSchema {
            namespace: "agentbox",
            function: "status",
            description: "Report AgentBox marketplace adapter status (mode flag + GMI provider \
                          wiring). Never includes the API key.",
            inputs: vec![],
            outputs: vec![
                bool_field("mode_enabled", "Whether OPENHUMAN_AGENTBOX_MODE=1."),
                bool_field(
                    "provider_configured",
                    "Whether all GMI_MAAS_* env vars are present and non-blank.",
                ),
                FieldSchema {
                    name: "provider",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Ref("AgentBoxProviderInfo"))),
                    comment: "Non-secret GMI provider wiring (slug, base_url, model).",
                    required: false,
                },
            ],
        },
        other => panic!("unknown agentbox schema function: {other}"),
    }
}

fn handle_agentbox_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        tracing::debug!("[agentbox] status requested");
        to_json(agentbox_status())
    })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn bool_field(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Bool,
        comment,
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_name_is_stable() {
        let s = agentbox_schemas("agentbox_status");
        assert_eq!(s.namespace, "agentbox");
        assert_eq!(s.function, "status");
    }

    #[test]
    fn controller_lists_match_lengths() {
        assert_eq!(
            all_agentbox_controller_schemas().len(),
            all_agentbox_registered_controllers().len()
        );
    }
}
