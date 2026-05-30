//! Controller-registry schemas for `openhuman.agent_registry_*`.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

use super::rpc;

const NAMESPACE: &str = "agent_registry";

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("available_tools"),
        schemas("get"),
        schemas("create_custom"),
        schemas("upsert_custom"),
        schemas("update"),
        schemas("set_enabled"),
        schemas("remove"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("available_tools"),
            handler: handle_available_tools,
        },
        RegisteredController {
            schema: schemas("get"),
            handler: handle_get,
        },
        RegisteredController {
            schema: schemas("create_custom"),
            handler: handle_create_custom,
        },
        RegisteredController {
            schema: schemas("upsert_custom"),
            handler: handle_upsert_custom,
        },
        RegisteredController {
            schema: schemas("update"),
            handler: handle_update,
        },
        RegisteredController {
            schema: schemas("set_enabled"),
            handler: handle_set_enabled,
        },
        RegisteredController {
            schema: schemas("remove"),
            handler: handle_remove,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: NAMESPACE,
            function: "list",
            description: "List default and custom agents available in the high-level registry.",
            inputs: vec![FieldSchema {
                name: "include_disabled",
                ty: TypeSchema::Bool,
                comment: "When true, include disabled agents in the response.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "agents",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("AgentRegistryEntry"))),
                comment: "Registry entries in default-first order.",
                required: true,
            }],
        },
        "available_tools" => ControllerSchema {
            namespace: NAMESPACE,
            function: "available_tools",
            description: "List every assignable agent tool (the full built-in tool catalog), with descriptions, for the agent editor's tool picker.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "tools",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("AgentToolInfo"))),
                comment: "Available tools sorted by name; each name is a valid tool_allowlist entry.",
                required: true,
            }],
        },
        "get" => ControllerSchema {
            namespace: NAMESPACE,
            function: "get",
            description: "Get one agent registry entry by id.",
            inputs: vec![required_string("id", "Agent id.")],
            outputs: vec![FieldSchema {
                name: "agent",
                ty: TypeSchema::Option(Box::new(TypeSchema::Ref("AgentRegistryEntry"))),
                comment: "Agent registry entry if found.",
                required: false,
            }],
        },
        "upsert_custom" => ControllerSchema {
            namespace: NAMESPACE,
            function: "upsert_custom",
            description: "Create or replace a custom user-authored agent with its tool policy.",
            inputs: vec![FieldSchema {
                name: "agent",
                ty: TypeSchema::Ref("AgentRegistryEntry"),
                comment: "Custom agent entry. Source is forced to custom.",
                required: true,
            }],
            outputs: vec![agent_output()],
        },
        "create_custom" => ControllerSchema {
            namespace: NAMESPACE,
            function: "create_custom",
            description: "Create or replace a custom user-authored agent from flat RPC params.",
            inputs: vec![
                required_string("id", "Custom agent id."),
                required_string("name", "Display name."),
                required_string("description", "When this agent should be used."),
                optional_bool("enabled", "Enable or disable this agent. Defaults to true."),
                optional_string("model", "Model id or route hint."),
                optional_string("system_prompt", "Custom instructions."),
                optional_string_array("tool_allowlist", "Allowed tool names; '*' means all."),
                optional_string_array("tool_denylist", "Denied tool names."),
                optional_string_array("subagents", "Delegated agent ids."),
                optional_string_array("tags", "UI grouping/search tags."),
                FieldSchema {
                    name: "metadata",
                    ty: TypeSchema::Json,
                    comment: "Free-form metadata.",
                    required: false,
                },
            ],
            outputs: vec![agent_output()],
        },
        "update" => ControllerSchema {
            namespace: NAMESPACE,
            function: "update",
            description: "Patch either a default-agent override or a custom agent.",
            inputs: vec![
                required_string("id", "Agent id."),
                optional_string("name", "New display name."),
                optional_string("description", "New description."),
                optional_bool("enabled", "Enable or disable this agent."),
                optional_string("model", "Model id or route hint."),
                optional_string("system_prompt", "Custom instructions."),
                optional_string_array("tool_allowlist", "Allowed tool names; '*' means all."),
                optional_string_array("tool_denylist", "Denied tool names."),
                optional_string_array("subagents", "Delegated agent ids."),
                optional_string_array("tags", "UI grouping/search tags."),
                FieldSchema {
                    name: "metadata",
                    ty: TypeSchema::Json,
                    comment: "Free-form metadata.",
                    required: false,
                },
            ],
            outputs: vec![agent_output()],
        },
        "set_enabled" => ControllerSchema {
            namespace: NAMESPACE,
            function: "set_enabled",
            description: "Enable or disable a default or custom agent.",
            inputs: vec![
                required_string("id", "Agent id."),
                FieldSchema {
                    name: "enabled",
                    ty: TypeSchema::Bool,
                    comment: "Desired enabled state.",
                    required: true,
                },
            ],
            outputs: vec![agent_output()],
        },
        "remove" => ControllerSchema {
            namespace: NAMESPACE,
            function: "remove",
            description: "Remove a custom agent or reset a default-agent override.",
            inputs: vec![required_string("id", "Agent id.")],
            outputs: vec![FieldSchema {
                name: "removed",
                ty: TypeSchema::Bool,
                comment: "True when a configured entry was removed.",
                required: true,
            }],
        },
        other => panic!("unknown agent_registry schema function: {other}"),
    }
}

fn handle_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::ListRequest>(Value::Object(params))?;
        to_json(rpc::list_rpc(req).await?)
    })
}

fn handle_available_tools(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::AvailableToolsRequest>(Value::Object(params))?;
        to_json(rpc::available_tools_rpc(req).await?)
    })
}

fn handle_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::GetRequest>(Value::Object(params))?;
        to_json(rpc::get_rpc(req).await?)
    })
}

fn handle_create_custom(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::CreateCustomRequest>(Value::Object(params))?;
        to_json(rpc::create_custom_rpc(req).await?)
    })
}

fn handle_upsert_custom(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::UpsertCustomRequest>(Value::Object(params))?;
        to_json(rpc::upsert_custom_rpc(req).await?)
    })
}

fn handle_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::UpdateRequest>(Value::Object(params))?;
        to_json(rpc::update_rpc(req).await?)
    })
}

fn handle_set_enabled(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::SetEnabledRequest>(Value::Object(params))?;
        to_json(rpc::set_enabled_rpc(req).await?)
    })
}

fn handle_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::RemoveRequest>(Value::Object(params))?;
        to_json(rpc::remove_rpc(req).await?)
    })
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

fn optional_string_array(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Array(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn agent_output() -> FieldSchema {
    FieldSchema {
        name: "agent",
        ty: TypeSchema::Ref("AgentRegistryEntry"),
        comment: "Updated agent registry entry.",
        required: true,
    }
}

fn parse_value<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_controller_schemas_and_registered_controllers_stay_in_sync() {
        let schemas = all_controller_schemas();
        let controllers = all_registered_controllers();
        assert_eq!(schemas.len(), controllers.len());
        assert!(schemas.iter().all(|schema| schema.namespace == NAMESPACE));
    }

    #[test]
    #[should_panic(expected = "unknown agent_registry schema function")]
    fn schemas_panics_on_unknown_function() {
        schemas("missing");
    }

    #[test]
    fn available_tools_schema_is_registered_with_tools_output() {
        let schema = schemas("available_tools");
        assert_eq!(schema.namespace, NAMESPACE);
        assert_eq!(schema.function, "available_tools");
        assert!(schema.inputs.is_empty());
        let tools = schema
            .outputs
            .iter()
            .find(|field| field.name == "tools")
            .expect("available_tools should output a `tools` field");
        assert!(tools.required);
        assert!(all_controller_schemas()
            .iter()
            .any(|s| s.function == "available_tools"));
    }
}
