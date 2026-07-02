//! Controller schemas for user-consented tiny.place session pairing.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schema_for("orchestration_pairing_list"),
        schema_for("orchestration_pairing_link_session"),
        schema_for("orchestration_pairing_accept_request"),
        schema_for("orchestration_pairing_decline_request"),
        schema_for("orchestration_pairing_block_request"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_for("orchestration_pairing_list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schema_for("orchestration_pairing_link_session"),
            handler: handle_link_session,
        },
        RegisteredController {
            schema: schema_for("orchestration_pairing_accept_request"),
            handler: handle_accept_request,
        },
        RegisteredController {
            schema: schema_for("orchestration_pairing_decline_request"),
            handler: handle_decline_request,
        },
        RegisteredController {
            schema: schema_for("orchestration_pairing_block_request"),
            handler: handle_block_request,
        },
    ]
}

fn schema_for(function: &str) -> ControllerSchema {
    match function {
        "orchestration_pairing_list" => ControllerSchema {
            namespace: "orchestration_pairing",
            function: "list",
            description: "List local orchestration pairing records plus the signed tiny.place contact state.",
            inputs: vec![],
            outputs: vec![json_output("result", "PairingSnapshot.")],
        },
        "orchestration_pairing_link_session" => ControllerSchema {
            namespace: "orchestration_pairing",
            function: "link_session",
            description: "User-consented link request for a wrapped session identity. Refuses blocked peers and sends no free-text contact payload.",
            inputs: vec![
                required_str("agentId", "Session agent identity to link."),
                optional_str("label", "Optional local display label for the session identity."),
            ],
            outputs: vec![json_output("result", "PairingActionResult.")],
        },
        "orchestration_pairing_accept_request" => ControllerSchema {
            namespace: "orchestration_pairing",
            function: "accept_request",
            description: "Accept an incoming contact request and persist a local approved-request pairing record.",
            inputs: vec![required_str("agentId", "Requesting session agent identity.")],
            outputs: vec![json_output("result", "PairingActionResult.")],
        },
        "orchestration_pairing_decline_request" => ControllerSchema {
            namespace: "orchestration_pairing",
            function: "decline_request",
            description: "Decline an incoming contact request and remove any local pairing record.",
            inputs: vec![required_str("agentId", "Requesting session agent identity.")],
            outputs: vec![json_output("result", "PairingActionResult.")],
        },
        "orchestration_pairing_block_request" => ControllerSchema {
            namespace: "orchestration_pairing",
            function: "block_request",
            description: "Block an incoming contact request and persist a local blocked record so it is not re-requested automatically.",
            inputs: vec![required_str("agentId", "Requesting session agent identity.")],
            outputs: vec![json_output("result", "PairingActionResult.")],
        },
        other => unreachable!("unknown orchestration_pairing schema: {other}"),
    }
}

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("list").await?;
        to_json(super::pairing::list(&config).await?)
    })
}

fn handle_link_session(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("link_session").await?;
        let agent_id = required_param(&params, "agentId")?;
        let label = params
            .get("label")
            .and_then(Value::as_str)
            .map(str::to_string);
        to_json(super::pairing::link_session(&config, agent_id, label).await?)
    })
}

fn handle_accept_request(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("accept_request").await?;
        let agent_id = required_param(&params, "agentId")?;
        to_json(super::pairing::accept_request(&config, agent_id).await?)
    })
}

fn handle_decline_request(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("decline_request").await?;
        let agent_id = required_param(&params, "agentId")?;
        to_json(super::pairing::decline_request(&config, agent_id).await?)
    })
}

fn handle_block_request(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("block_request").await?;
        let agent_id = required_param(&params, "agentId")?;
        to_json(super::pairing::block_request(&config, agent_id).await?)
    })
}

async fn load_config(action: &str) -> Result<crate::openhuman::config::Config, String> {
    log::debug!(target: "orchestration_pairing_rpc", "[orchestration_pairing_rpc] {action}.config_load");
    config_rpc::load_config_with_timeout()
        .await
        .inspect_err(|err| {
            log::warn!(target: "orchestration_pairing_rpc", "[orchestration_pairing_rpc] {action}.config_failed err={err}");
        })
}

fn required_param<'a>(params: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{key} is required"))
}

fn required_str(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_str(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value)
        .map_err(|err| format!("serialize orchestration pairing response: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_use_orchestration_pairing_namespace() {
        let schemas = all_controller_schemas();
        assert_eq!(schemas.len(), 5);
        assert!(schemas
            .iter()
            .all(|schema| schema.namespace == "orchestration_pairing"));
        assert_eq!(
            schema_for("orchestration_pairing_link_session").function,
            "link_session"
        );
    }

    #[test]
    fn required_param_rejects_blank_agent_id() {
        let mut params = Map::new();
        params.insert("agentId".to_string(), Value::String("   ".to_string()));
        let err = required_param(&params, "agentId").unwrap_err();
        assert!(err.contains("agentId"));
    }
}
