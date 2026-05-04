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

#[derive(Debug, Deserialize)]
struct WebhookCreateTunnelParams {
    name: String,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WebhookTunnelIdParams {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebhookUpdateTunnelParams {
    id: String,
    name: Option<String>,
    description: Option<String>,
    is_active: Option<bool>,
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
        schemas("list_tunnels"),
        schemas("create_tunnel"),
        schemas("get_tunnel"),
        schemas("update_tunnel"),
        schemas("delete_tunnel"),
        schemas("get_bandwidth"),
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
        RegisteredController {
            schema: schemas("list_tunnels"),
            handler: handle_list_tunnels,
        },
        RegisteredController {
            schema: schemas("create_tunnel"),
            handler: handle_create_tunnel,
        },
        RegisteredController {
            schema: schemas("get_tunnel"),
            handler: handle_get_tunnel,
        },
        RegisteredController {
            schema: schemas("update_tunnel"),
            handler: handle_update_tunnel,
        },
        RegisteredController {
            schema: schemas("delete_tunnel"),
            handler: handle_delete_tunnel,
        },
        RegisteredController {
            schema: schemas("get_bandwidth"),
            handler: handle_get_bandwidth,
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
                 are routed to the triage pipeline instead of the skill runtime.",
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
        "list_tunnels" => ControllerSchema {
            namespace: "webhooks",
            function: "list_tunnels",
            description: "List backend-managed webhook tunnels for the current user.",
            inputs: vec![],
            outputs: vec![json_output("result", "Webhook tunnel list.")],
        },
        "create_tunnel" => ControllerSchema {
            namespace: "webhooks",
            function: "create_tunnel",
            description: "Create a backend-managed webhook tunnel.",
            inputs: vec![
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::String,
                    comment: "Tunnel name.",
                    required: true,
                },
                FieldSchema {
                    name: "description",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional tunnel description.",
                    required: false,
                },
            ],
            outputs: vec![json_output("result", "Created webhook tunnel.")],
        },
        "delete_tunnel" => ControllerSchema {
            namespace: "webhooks",
            function: "delete_tunnel",
            description: "Delete a backend-managed webhook tunnel.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Backend tunnel id.",
                required: true,
            }],
            outputs: vec![json_output("result", "Delete webhook tunnel result.")],
        },
        "get_tunnel" => ControllerSchema {
            namespace: "webhooks",
            function: "get_tunnel",
            description: "Fetch a backend-managed webhook tunnel by id.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Backend tunnel id.",
                required: true,
            }],
            outputs: vec![json_output("result", "Webhook tunnel payload.")],
        },
        "update_tunnel" => ControllerSchema {
            namespace: "webhooks",
            function: "update_tunnel",
            description: "Update a backend-managed webhook tunnel.",
            inputs: vec![
                FieldSchema {
                    name: "id",
                    ty: TypeSchema::String,
                    comment: "Backend tunnel id.",
                    required: true,
                },
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional tunnel name.",
                    required: false,
                },
                FieldSchema {
                    name: "description",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional tunnel description.",
                    required: false,
                },
                FieldSchema {
                    name: "isActive",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Optional active flag.",
                    required: false,
                },
            ],
            outputs: vec![json_output("result", "Updated webhook tunnel payload.")],
        },
        "get_bandwidth" => ControllerSchema {
            namespace: "webhooks",
            function: "get_bandwidth",
            description: "Fetch the remaining webhook bandwidth budget.",
            inputs: vec![],
            outputs: vec![json_output("result", "Webhook bandwidth payload.")],
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
    Box::pin(async { to_json(crate::openhuman::webhooks::ops::list_registrations().await?) })
}

fn handle_list_logs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WebhookListLogsParams>(params)?;
        to_json(crate::openhuman::webhooks::ops::list_logs(payload.limit).await?)
    })
}

fn handle_clear_logs(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(crate::openhuman::webhooks::ops::clear_logs().await?) })
}

fn handle_register_echo(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WebhookRegisterEchoParams>(params)?;
        to_json(
            crate::openhuman::webhooks::ops::register_echo(
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
        to_json(crate::openhuman::webhooks::ops::unregister_echo(&payload.tunnel_uuid).await?)
    })
}

fn handle_register_agent(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WebhookRegisterAgentParams>(params)?;
        to_json(
            crate::openhuman::webhooks::ops::register_agent(
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
            crate::openhuman::webhooks::ops::trigger_agent(
                source,
                &payload.caller_id,
                reason,
                trigger_payload,
            )
            .await?,
        )
    })
}

fn handle_list_tunnels(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = crate::openhuman::config::rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::webhooks::ops::list_tunnels(&config).await?)
    })
}

fn handle_create_tunnel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = crate::openhuman::config::rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<WebhookCreateTunnelParams>(params)?;
        to_json(
            crate::openhuman::webhooks::ops::create_tunnel(
                &config,
                payload.name.trim(),
                payload.description,
            )
            .await?,
        )
    })
}

fn handle_delete_tunnel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = crate::openhuman::config::rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<WebhookTunnelIdParams>(params)?;
        to_json(crate::openhuman::webhooks::ops::delete_tunnel(&config, payload.id.trim()).await?)
    })
}

fn handle_get_tunnel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = crate::openhuman::config::rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<WebhookTunnelIdParams>(params)?;
        to_json(crate::openhuman::webhooks::ops::get_tunnel(&config, payload.id.trim()).await?)
    })
}

fn handle_update_tunnel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = crate::openhuman::config::rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<WebhookUpdateTunnelParams>(params)?;
        let mut body = serde_json::Map::new();
        if let Some(name) = payload.name {
            body.insert("name".to_string(), Value::String(name));
        }
        if let Some(desc) = payload.description {
            body.insert("description".to_string(), Value::String(desc));
        }
        if let Some(active) = payload.is_active {
            body.insert("isActive".to_string(), Value::Bool(active));
        }
        let body = Value::Object(body);
        to_json(
            crate::openhuman::webhooks::ops::update_tunnel(&config, payload.id.trim(), body)
                .await?,
        )
    })
}

fn handle_get_bandwidth(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = crate::openhuman::config::rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::webhooks::ops::get_bandwidth(&config).await?)
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
