use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct WebhookListLogsParams {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct WebhookRegisterEchoParams {
    tunnel_uuid: String,
    tunnel_name: Option<String>,
    backend_tunnel_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WebhookUnregisterEchoParams {
    tunnel_uuid: String,
}

#[derive(Debug, Deserialize)]
struct WebhookRegisterAgentParams {
    tunnel_uuid: String,
    agent_id: Option<String>,
    tunnel_name: Option<String>,
    backend_tunnel_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WebhookTriggerAgentParams {
    /// Trigger source slug: `"webhook"`, `"cron"`, or `"external"`.
    source: Option<String>,
    /// Stable identifier for the caller (tunnel UUID, job ID, etc.).
    caller_id: String,
    /// Human-readable reason / label for the trigger.
    reason: Option<String>,
    /// Trigger payload forwarded to the triage pipeline.
    payload: Option<Value>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list_registrations"),
        schemas("list_logs"),
        schemas("clear_logs"),
        schemas("register_echo"),
        schemas("unregister_echo"),
        schemas("register_agent"),
        schemas("trigger_agent"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list_registrations"),
            handler: handle_list_registrations,
        },
        RegisteredController {
            schema: schemas("list_logs"),
            handler: handle_list_logs,
        },
        RegisteredController {
            schema: schemas("clear_logs"),
            handler: handle_clear_logs,
        },
        RegisteredController {
            schema: schemas("register_echo"),
            handler: handle_register_echo,
        },
        RegisteredController {
            schema: schemas("unregister_echo"),
            handler: handle_unregister_echo,
        },
        RegisteredController {
            schema: schemas("register_agent"),
            handler: handle_register_agent,
        },
        RegisteredController {
            schema: schemas("trigger_agent"),
            handler: handle_trigger_agent,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list_registrations" => ControllerSchema {
            namespace: "webhooks",
            function: "list_registrations",
            description:
                "List all webhook tunnel registrations currently owned by the app runtime.",
            inputs: vec![],
            outputs: vec![json_output("result", "Webhook registration list.")],
        },
        "list_logs" => ControllerSchema {
            namespace: "webhooks",
            function: "list_logs",
            description: "List captured webhook request and response debug logs.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum number of log entries to return.",
                required: false,
            }],
            outputs: vec![json_output("result", "Webhook debug log list.")],
        },
        "clear_logs" => ControllerSchema {
            namespace: "webhooks",
            function: "clear_logs",
            description: "Clear captured webhook debug logs.",
            inputs: vec![],
            outputs: vec![json_output("result", "Webhook log clear result.")],
        },
        "register_echo" => ControllerSchema {
            namespace: "webhooks",
            function: "register_echo",
            description: "Register a built-in echo webhook target for a tunnel UUID.",
            inputs: vec![
                FieldSchema {
                    name: "tunnel_uuid",
                    ty: TypeSchema::String,
                    comment: "Tunnel UUID from the backend.",
                    required: true,
                },
                FieldSchema {
                    name: "tunnel_name",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional human-readable tunnel name.",
                    required: false,
                },
                FieldSchema {
                    name: "backend_tunnel_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional backend tunnel id.",
                    required: false,
                },
            ],
            outputs: vec![json_output("result", "Updated webhook registrations.")],
        },
        "unregister_echo" => ControllerSchema {
            namespace: "webhooks",
            function: "unregister_echo",
            description: "Unregister a built-in echo webhook target for a tunnel UUID.",
            inputs: vec![FieldSchema {
                name: "tunnel_uuid",
                ty: TypeSchema::String,
                comment: "Tunnel UUID from the backend.",
                required: true,
            }],
            outputs: vec![json_output("result", "Updated webhook registrations.")],
        },
        "register_agent" => ControllerSchema {
            namespace: "webhooks",
            function: "register_agent",
            description:
                "Register an agent-backed webhook tunnel. Incoming requests on this tunnel \
                 are routed to the triage pipeline instead of direct skill dispatch.",
            inputs: vec![
                FieldSchema {
                    name: "tunnel_uuid",
                    ty: TypeSchema::String,
                    comment: "Tunnel UUID from the backend.",
                    required: true,
                },
                FieldSchema {
                    name: "agent_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional agent definition id to pin for this tunnel.",
                    required: false,
                },
                FieldSchema {
                    name: "tunnel_name",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional human-readable tunnel name.",
                    required: false,
                },
                FieldSchema {
                    name: "backend_tunnel_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional backend tunnel id.",
                    required: false,
                },
            ],
            outputs: vec![json_output("result", "Updated webhook registrations.")],
        },
        "trigger_agent" => ControllerSchema {
            namespace: "webhooks",
            function: "trigger_agent",
            description: "Trigger the triage/agent pipeline directly via RPC without requiring an \
                 incoming webhook request. Useful for testing and manual escalation.",
            inputs: vec![
                FieldSchema {
                    name: "caller_id",
                    ty: TypeSchema::String,
                    comment: "Stable identifier for the caller (tunnel UUID, job ID, etc.).",
                    required: true,
                },
                FieldSchema {
                    name: "source",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Trigger source slug: 'webhook', 'cron', or 'external' (default).",
                    required: false,
                },
                FieldSchema {
                    name: "reason",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Human-readable reason or label for the trigger.",
                    required: false,
                },
                FieldSchema {
                    name: "payload",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional trigger payload forwarded to the triage pipeline.",
                    required: false,
                },
            ],
            outputs: vec![json_output("result", "Triage decision result.")],
        },
        _ => ControllerSchema {
            namespace: "webhooks",
            function: "unknown",
            description: "Unknown webhooks controller function.",
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

fn handle_list_registrations(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(crate::skills::webhooks::ops::list_registrations().await?) })
}

fn handle_list_logs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WebhookListLogsParams>(params)?;
        to_json(crate::skills::webhooks::ops::list_logs(payload.limit).await?)
    })
}

fn handle_clear_logs(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(crate::skills::webhooks::ops::clear_logs().await?) })
}

fn handle_register_echo(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WebhookRegisterEchoParams>(params)?;
        to_json(
            crate::skills::webhooks::ops::register_echo(
                &payload.tunnel_uuid,
                payload.tunnel_name,
                payload.backend_tunnel_id,
            )
            .await?,
        )
    })
}

fn handle_unregister_echo(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WebhookUnregisterEchoParams>(params)?;
        to_json(crate::skills::webhooks::ops::unregister_echo(&payload.tunnel_uuid).await?)
    })
}

fn handle_register_agent(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WebhookRegisterAgentParams>(params)?;
        to_json(
            crate::skills::webhooks::ops::register_agent(
                &payload.tunnel_uuid,
                payload.agent_id,
                payload.tunnel_name,
                payload.backend_tunnel_id,
            )
            .await?,
        )
    })
}

fn handle_trigger_agent(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WebhookTriggerAgentParams>(params)?;
        let source = payload.source.as_deref().unwrap_or("external");
        let reason = payload.reason.as_deref().unwrap_or("rpc_trigger");
        let trigger_payload = payload.payload.unwrap_or_else(|| serde_json::json!({}));
        to_json(
            crate::skills::webhooks::ops::trigger_agent(
                source,
                &payload.caller_id,
                reason,
                trigger_payload,
            )
            .await?,
        )
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
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
