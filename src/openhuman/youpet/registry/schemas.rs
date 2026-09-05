use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::ops::{
    registry_get_agent_version, registry_get_connector_binding_version,
    registry_get_connector_type_version, registry_get_tool_definition_version,
    registry_get_tool_enablement_version, registry_list_agents, registry_list_connector_bindings,
    registry_list_connector_types, registry_list_tool_definitions, registry_list_tool_enablements,
};
use super::types::{
    RegistryGetAgentVersionRpcParams, RegistryGetConnectorBindingVersionRpcParams,
    RegistryGetConnectorTypeVersionRpcParams, RegistryGetToolDefinitionVersionRpcParams,
    RegistryGetToolEnablementVersionRpcParams, RegistryListAgentsRpcParams,
    RegistryListConnectorBindingsRpcParams, RegistryListConnectorTypesRpcParams,
    RegistryListToolDefinitionsRpcParams,
};

pub fn all_internal_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: registry_schemas("registry_list_agents"),
            handler: handle_registry_list_agents,
        },
        RegisteredController {
            schema: registry_schemas("registry_get_agent_version"),
            handler: handle_registry_get_agent_version,
        },
        RegisteredController {
            schema: registry_schemas("registry_list_tool_definitions"),
            handler: handle_registry_list_tool_definitions,
        },
        RegisteredController {
            schema: registry_schemas("registry_get_tool_definition_version"),
            handler: handle_registry_get_tool_definition_version,
        },
        RegisteredController {
            schema: registry_schemas("registry_list_tool_enablements"),
            handler: handle_registry_list_tool_enablements,
        },
        RegisteredController {
            schema: registry_schemas("registry_get_tool_enablement_version"),
            handler: handle_registry_get_tool_enablement_version,
        },
        RegisteredController {
            schema: registry_schemas("registry_list_connector_types"),
            handler: handle_registry_list_connector_types,
        },
        RegisteredController {
            schema: registry_schemas("registry_get_connector_type_version"),
            handler: handle_registry_get_connector_type_version,
        },
        RegisteredController {
            schema: registry_schemas("registry_list_connector_bindings"),
            handler: handle_registry_list_connector_bindings,
        },
        RegisteredController {
            schema: registry_schemas("registry_get_connector_binding_version"),
            handler: handle_registry_get_connector_binding_version,
        },
    ]
}

pub fn registry_schemas(function: &str) -> ControllerSchema {
    match function {
        "registry_list_agents" => ControllerSchema {
            namespace: "youpet",
            function: "registry_list_agents",
            description: "List active Core Agent Registry summaries through the Rust bridge.",
            inputs: list_inputs(),
            outputs: vec![json_output("page", "Cursor-backed Agent Registry page.")],
        },
        "registry_get_agent_version" => ControllerSchema {
            namespace: "youpet",
            function: "registry_get_agent_version",
            description: "Load one exact Core Agent Registry version.",
            inputs: exact_inputs("agentKey", "Agent logical key."),
            outputs: vec![json_output("agent", "Exact Agent Registry record.")],
        },
        "registry_list_tool_definitions" => ControllerSchema {
            namespace: "youpet",
            function: "registry_list_tool_definitions",
            description: "List active Core Tool Definition summaries through the Rust bridge.",
            inputs: list_inputs(),
            outputs: vec![json_output(
                "page",
                "Cursor-backed Tool Definition Registry page.",
            )],
        },
        "registry_get_tool_definition_version" => ControllerSchema {
            namespace: "youpet",
            function: "registry_get_tool_definition_version",
            description: "Load one exact Core Tool Definition version.",
            inputs: exact_inputs("toolKey", "Tool logical key."),
            outputs: vec![json_output(
                "toolDefinition",
                "Exact Tool Definition record.",
            )],
        },
        "registry_list_tool_enablements" => ControllerSchema {
            namespace: "youpet",
            function: "registry_list_tool_enablements",
            description: "List current Core Tool Enablement summaries through the Rust bridge.",
            inputs: vec![],
            outputs: vec![json_output(
                "items",
                "Unpaged Tool Enablement collection for the sole active Kernel Tenant.",
            )],
        },
        "registry_get_tool_enablement_version" => ControllerSchema {
            namespace: "youpet",
            function: "registry_get_tool_enablement_version",
            description: "Load one exact Core Tool Enablement version.",
            inputs: exact_inputs("toolKey", "Tool logical key."),
            outputs: vec![json_output(
                "toolEnablement",
                "Exact Tool Enablement record.",
            )],
        },
        "registry_list_connector_types" => ControllerSchema {
            namespace: "youpet",
            function: "registry_list_connector_types",
            description: "List Core Connector Type history through the Rust bridge.",
            inputs: list_inputs(),
            outputs: vec![json_output("page", "Cursor-backed Connector Type page.")],
        },
        "registry_get_connector_type_version" => ControllerSchema {
            namespace: "youpet",
            function: "registry_get_connector_type_version",
            description: "Load one exact Core Connector Type version.",
            inputs: exact_inputs("connectorKey", "Connector logical key."),
            outputs: vec![json_output("connectorType", "Exact Connector Type record.")],
        },
        "registry_list_connector_bindings" => ControllerSchema {
            namespace: "youpet",
            function: "registry_list_connector_bindings",
            description: "List Core Connector Binding history through the Rust bridge.",
            inputs: list_inputs(),
            outputs: vec![json_output("page", "Cursor-backed Connector Binding page.")],
        },
        "registry_get_connector_binding_version" => ControllerSchema {
            namespace: "youpet",
            function: "registry_get_connector_binding_version",
            description: "Load one exact Core Connector Binding version.",
            inputs: exact_inputs("bindingKey", "Binding logical key."),
            outputs: vec![json_output(
                "connectorBinding",
                "Exact Connector Binding record.",
            )],
        },
        _ => ControllerSchema {
            namespace: "youpet",
            function: "unknown",
            description: "Unknown YouPet registry controller function.",
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

fn handle_registry_list_agents(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<RegistryListAgentsRpcParams>(params)?;
        to_json(registry_list_agents(&config, payload).await?)
    })
}

fn handle_registry_get_agent_version(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<RegistryGetAgentVersionRpcParams>(params)?;
        to_json(registry_get_agent_version(&config, payload).await?)
    })
}

fn handle_registry_list_tool_definitions(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<RegistryListToolDefinitionsRpcParams>(params)?;
        to_json(registry_list_tool_definitions(&config, payload).await?)
    })
}

fn handle_registry_get_tool_definition_version(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<RegistryGetToolDefinitionVersionRpcParams>(params)?;
        to_json(registry_get_tool_definition_version(&config, payload).await?)
    })
}

fn handle_registry_list_tool_enablements(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(registry_list_tool_enablements(&config).await?)
    })
}

fn handle_registry_get_tool_enablement_version(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<RegistryGetToolEnablementVersionRpcParams>(params)?;
        to_json(registry_get_tool_enablement_version(&config, payload).await?)
    })
}

fn handle_registry_list_connector_types(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<RegistryListConnectorTypesRpcParams>(params)?;
        to_json(registry_list_connector_types(&config, payload).await?)
    })
}

fn handle_registry_get_connector_type_version(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<RegistryGetConnectorTypeVersionRpcParams>(params)?;
        to_json(registry_get_connector_type_version(&config, payload).await?)
    })
}

fn handle_registry_list_connector_bindings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<RegistryListConnectorBindingsRpcParams>(params)?;
        to_json(registry_list_connector_bindings(&config, payload).await?)
    })
}

fn handle_registry_get_connector_binding_version(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<RegistryGetConnectorBindingVersionRpcParams>(params)?;
        to_json(registry_get_connector_binding_version(&config, payload).await?)
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

fn list_inputs() -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "limit",
            ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
            comment: "Optional page size (1-200).",
            required: false,
        },
        FieldSchema {
            name: "cursor",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Optional opaque cursor returned by the matching Registry collection.",
            required: false,
        },
    ]
}

fn exact_inputs(key_name: &'static str, key_comment: &'static str) -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: key_name,
            ty: TypeSchema::String,
            comment: key_comment,
            required: true,
        },
        FieldSchema {
            name: "version",
            ty: TypeSchema::I64,
            comment: "Exact Registry version (>= 1).",
            required: true,
        },
    ]
}
