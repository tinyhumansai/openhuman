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
mod tests {
    use super::*;
    use serde_json::json;

    // ── Catalog integrity ─────────────────────────────────────────

    const EXPECTED_FUNCTIONS: &[&str] = &[
        "list_registrations",
        "list_logs",
        "clear_logs",
        "register_echo",
        "unregister_echo",
        "register_agent",
        "trigger_agent",
        "list_tunnels",
        "create_tunnel",
        "get_tunnel",
        "update_tunnel",
        "delete_tunnel",
        "get_bandwidth",
    ];

    #[test]
    fn all_controller_schemas_matches_expected_function_set() {
        let schemas_list = all_controller_schemas();
        assert_eq!(schemas_list.len(), EXPECTED_FUNCTIONS.len());
        let names: Vec<&str> = schemas_list.iter().map(|s| s.function).collect();
        for expected in EXPECTED_FUNCTIONS {
            assert!(
                names.contains(expected),
                "catalog missing `{expected}`; got {names:?}"
            );
        }
    }

    #[test]
    fn all_controller_schemas_entries_are_all_under_webhooks_namespace() {
        for s in all_controller_schemas() {
            assert_eq!(
                s.namespace, "webhooks",
                "schema `{}` has wrong namespace",
                s.function
            );
            assert!(
                !s.description.trim().is_empty(),
                "schema `{}` must have a description",
                s.function
            );
        }
    }

    #[test]
    fn all_registered_controllers_parallels_the_schema_list() {
        let schemas_list = all_controller_schemas();
        let handlers = all_registered_controllers();
        assert_eq!(schemas_list.len(), handlers.len());

        // Every registered controller's schema must resolve back to the
        // same ControllerSchema produced by `schemas()` — proves the two
        // lists are kept in lock-step and no handler is mis-wired.
        for rc in &handlers {
            let resolved = schemas(rc.schema.function);
            assert_eq!(resolved.function, rc.schema.function);
            assert_eq!(resolved.namespace, rc.schema.namespace);
        }
    }

    #[test]
    fn all_registered_controller_function_names_are_unique() {
        let handlers = all_registered_controllers();
        let mut names: Vec<&str> = handlers.iter().map(|rc| rc.schema.function).collect();
        names.sort_unstable();
        let unique_count = {
            let mut clone = names.clone();
            clone.dedup();
            clone.len()
        };
        assert_eq!(
            unique_count,
            names.len(),
            "duplicate function names: {names:?}"
        );
    }

    // ── schemas(function) per-arm coverage ───────────────────────

    fn required_input_names(s: &ControllerSchema) -> Vec<&'static str> {
        s.inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect()
    }

    #[test]
    fn list_registrations_has_no_inputs_and_json_output() {
        let s = schemas("list_registrations");
        assert!(s.inputs.is_empty());
        assert_eq!(s.outputs.len(), 1);
        assert_eq!(s.outputs[0].name, "result");
        assert!(matches!(s.outputs[0].ty, TypeSchema::Json));
    }

    #[test]
    fn list_logs_limit_is_optional_u64() {
        let s = schemas("list_logs");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "limit");
        assert!(!s.inputs[0].required);
        match &s.inputs[0].ty {
            TypeSchema::Option(inner) => assert!(matches!(**inner, TypeSchema::U64)),
            other => panic!("limit must be Option<U64>, got {other:?}"),
        }
    }

    #[test]
    fn clear_logs_has_no_inputs() {
        assert!(schemas("clear_logs").inputs.is_empty());
    }

    #[test]
    fn register_echo_requires_tunnel_uuid_only() {
        let s = schemas("register_echo");
        assert_eq!(required_input_names(&s), vec!["tunnel_uuid"]);
        // The two optional fields must exist and be Option<String>.
        for optional in ["tunnel_name", "backend_tunnel_id"] {
            let f = s
                .inputs
                .iter()
                .find(|f| f.name == optional)
                .unwrap_or_else(|| panic!("missing optional `{optional}`"));
            assert!(!f.required);
            assert!(
                matches!(&f.ty, TypeSchema::Option(inner) if matches!(**inner, TypeSchema::String))
            );
        }
    }

    #[test]
    fn unregister_echo_requires_tunnel_uuid_only() {
        let s = schemas("unregister_echo");
        assert_eq!(required_input_names(&s), vec!["tunnel_uuid"]);
    }

    #[test]
    fn register_agent_requires_tunnel_uuid_and_has_optional_fields() {
        let s = schemas("register_agent");
        assert_eq!(required_input_names(&s), vec!["tunnel_uuid"]);
        for optional in ["agent_id", "tunnel_name", "backend_tunnel_id"] {
            assert!(
                s.inputs.iter().any(|f| f.name == optional && !f.required),
                "`register_agent` must accept optional `{optional}`"
            );
        }
    }

    #[test]
    fn trigger_agent_requires_caller_id_only() {
        let s = schemas("trigger_agent");
        assert_eq!(required_input_names(&s), vec!["caller_id"]);
        for optional in ["source", "reason", "payload"] {
            assert!(
                s.inputs.iter().any(|f| f.name == optional && !f.required),
                "`trigger_agent` must accept optional `{optional}`"
            );
        }
    }

    #[test]
    fn list_tunnels_has_no_inputs() {
        assert!(schemas("list_tunnels").inputs.is_empty());
    }

    #[test]
    fn create_tunnel_requires_name_and_allows_optional_description() {
        let s = schemas("create_tunnel");
        assert_eq!(required_input_names(&s), vec!["name"]);
        assert!(s
            .inputs
            .iter()
            .any(|f| f.name == "description" && !f.required));
    }

    #[test]
    fn get_and_delete_tunnel_require_id_only() {
        for fn_name in ["get_tunnel", "delete_tunnel"] {
            let s = schemas(fn_name);
            assert_eq!(
                required_input_names(&s),
                vec!["id"],
                "`{fn_name}` must require only `id`"
            );
        }
    }

    #[test]
    fn update_tunnel_requires_id_and_allows_optional_name_description_is_active() {
        let s = schemas("update_tunnel");
        assert_eq!(required_input_names(&s), vec!["id"]);
        for optional in ["name", "description", "isActive"] {
            assert!(
                s.inputs.iter().any(|f| f.name == optional && !f.required),
                "`update_tunnel` must accept optional `{optional}`"
            );
        }
    }

    #[test]
    fn get_bandwidth_has_no_inputs() {
        assert!(schemas("get_bandwidth").inputs.is_empty());
    }

    #[test]
    fn unknown_function_returns_error_fallback_schema() {
        let s = schemas("no_such_fn");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.namespace, "webhooks");
        assert_eq!(s.outputs.len(), 1);
        assert_eq!(s.outputs[0].name, "error");
        assert!(matches!(s.outputs[0].ty, TypeSchema::String));
        assert!(s.outputs[0].required);
    }

    // ── deserialize_params ────────────────────────────────────────

    #[test]
    fn deserialize_params_returns_typed_struct_for_valid_input() {
        let mut params = Map::new();
        params.insert("tunnel_uuid".to_string(), Value::String("u-1".into()));
        params.insert("tunnel_name".to_string(), Value::String("n".into()));
        params.insert("backend_tunnel_id".to_string(), Value::Null);
        let parsed = deserialize_params::<WebhookRegisterEchoParams>(params).unwrap();
        assert_eq!(parsed.tunnel_uuid, "u-1");
        assert_eq!(parsed.tunnel_name.as_deref(), Some("n"));
        assert!(parsed.backend_tunnel_id.is_none());
    }

    #[test]
    fn deserialize_params_reports_invalid_params_errors() {
        // Missing required `tunnel_uuid` for WebhookUnregisterEchoParams.
        let err = deserialize_params::<WebhookUnregisterEchoParams>(Map::new()).unwrap_err();
        assert!(
            err.contains("invalid params"),
            "expected 'invalid params' prefix, got: {err}"
        );
    }

    #[test]
    fn deserialize_params_honours_camel_case_rename_for_update_tunnel() {
        // `WebhookUpdateTunnelParams` uses `#[serde(rename_all = "camelCase")]`,
        // so the JSON key is `isActive` even though the Rust field is
        // `is_active`. This test locks in that contract.
        let mut params = Map::new();
        params.insert("id".to_string(), Value::String("t-1".into()));
        params.insert("isActive".to_string(), Value::Bool(true));
        let parsed = deserialize_params::<WebhookUpdateTunnelParams>(params).unwrap();
        assert_eq!(parsed.id, "t-1");
        assert_eq!(parsed.is_active, Some(true));
    }

    // ── json_output / to_json ─────────────────────────────────────

    #[test]
    fn json_output_builds_required_json_field() {
        let f = json_output("result", "stuff");
        assert_eq!(f.name, "result");
        assert_eq!(f.comment, "stuff");
        assert!(f.required);
        assert!(matches!(f.ty, TypeSchema::Json));
    }

    #[test]
    fn to_json_renders_rpc_outcome_in_cli_compatible_shape() {
        // `to_json` is a thin wrapper over `RpcOutcome::into_cli_compatible_json`.
        // We exercise it here so coverage follows the real shape the
        // adapters produce, rather than asserting on implementation details.
        let outcome: RpcOutcome<serde_json::Value> = RpcOutcome::new(json!({"ok": true}), vec![]);
        let value = to_json(outcome).unwrap();
        assert!(value.is_object());
    }
}
