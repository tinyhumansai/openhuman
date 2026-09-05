use crate::openhuman::config::Config;
use crate::openhuman::youpet::YouPetTransport;
use crate::rpc::RpcOutcome;

use super::types::{
    AgentRegistryAgent, AgentRegistryAgentSummary, ConnectorRegistryBinding,
    ConnectorRegistryBindingSummary, ConnectorRegistryType, ConnectorRegistryTypeSummary,
    RegistryCursorListResponse, RegistryGetAgentVersionRpcParams,
    RegistryGetConnectorBindingVersionRpcParams, RegistryGetConnectorTypeVersionRpcParams,
    RegistryGetToolDefinitionVersionRpcParams, RegistryGetToolEnablementVersionRpcParams,
    RegistryListAgentsRpcParams, RegistryListConnectorBindingsRpcParams,
    RegistryListConnectorTypesRpcParams, RegistryListToolDefinitionsRpcParams,
    RegistryUnpagedListResponse, ToolRegistryToolDefinition, ToolRegistryToolDefinitionSummary,
    ToolRegistryToolEnablement,
};

const REGISTRY_LIST_AGENTS_PATH: &str = "/api/v1/kernel/agents";
const REGISTRY_AGENT_VERSION_PATH: &str = "/api/v1/kernel/agents/{agent_key}/versions/{version}";
const REGISTRY_LIST_TOOL_DEFINITIONS_PATH: &str = "/api/v1/kernel/tool-definitions";
const REGISTRY_TOOL_DEFINITION_VERSION_PATH: &str =
    "/api/v1/kernel/tool-definitions/{tool_key}/versions/{version}";
const REGISTRY_LIST_TOOL_ENABLEMENTS_PATH: &str = "/api/v1/kernel/tool-enablement";
const REGISTRY_TOOL_ENABLEMENT_VERSION_PATH: &str =
    "/api/v1/kernel/tool-enablement/{tool_key}/versions/{version}";
const REGISTRY_LIST_CONNECTOR_TYPES_PATH: &str = "/api/v1/kernel/connector-types";
const REGISTRY_CONNECTOR_TYPE_VERSION_PATH: &str =
    "/api/v1/kernel/connector-types/{connector_key}/versions/{version}";
const REGISTRY_LIST_CONNECTOR_BINDINGS_PATH: &str = "/api/v1/kernel/connector-bindings";
const REGISTRY_CONNECTOR_BINDING_VERSION_PATH: &str =
    "/api/v1/kernel/connector-bindings/{binding_key}/versions/{version}";

pub async fn registry_list_agents(
    config: &Config,
    params: RegistryListAgentsRpcParams,
) -> Result<RpcOutcome<RegistryCursorListResponse<AgentRegistryAgentSummary>>, String> {
    params.validate()?;
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let mut request = transport.get(REGISTRY_LIST_AGENTS_PATH)?;
    request = request.query(&[("limit", params.limit_or_default())]);
    if let Some(cursor) = params
        .cursor
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request = request.query(&[("cursor", cursor)]);
    }
    let response = transport.send(request).await?;
    Ok(RpcOutcome::single_log(
        response,
        "[youpet] listed Core registry agents",
    ))
}

pub async fn registry_get_agent_version(
    config: &Config,
    params: RegistryGetAgentVersionRpcParams,
) -> Result<RpcOutcome<AgentRegistryAgent>, String> {
    params.validate()?;
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let path = REGISTRY_AGENT_VERSION_PATH
        .replace("{agent_key}", &urlencoding::encode(params.agent_key.trim()))
        .replace("{version}", &params.version.to_string());
    let response = transport.send(transport.get(&path)?).await?;
    Ok(RpcOutcome::single_log(
        response,
        "[youpet] loaded Core registry agent version",
    ))
}

pub async fn registry_list_tool_definitions(
    config: &Config,
    params: RegistryListToolDefinitionsRpcParams,
) -> Result<RpcOutcome<RegistryCursorListResponse<ToolRegistryToolDefinitionSummary>>, String> {
    params.validate()?;
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let mut request = transport.get(REGISTRY_LIST_TOOL_DEFINITIONS_PATH)?;
    request = request.query(&[("limit", params.limit_or_default())]);
    if let Some(cursor) = params
        .cursor
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request = request.query(&[("cursor", cursor)]);
    }
    let response = transport.send(request).await?;
    Ok(RpcOutcome::single_log(
        response,
        "[youpet] listed Core registry tool definitions",
    ))
}

pub async fn registry_get_tool_definition_version(
    config: &Config,
    params: RegistryGetToolDefinitionVersionRpcParams,
) -> Result<RpcOutcome<ToolRegistryToolDefinition>, String> {
    params.validate()?;
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let path = REGISTRY_TOOL_DEFINITION_VERSION_PATH
        .replace("{tool_key}", &urlencoding::encode(params.tool_key.trim()))
        .replace("{version}", &params.version.to_string());
    let response = transport.send(transport.get(&path)?).await?;
    Ok(RpcOutcome::single_log(
        response,
        "[youpet] loaded Core registry tool definition version",
    ))
}

pub async fn registry_list_tool_enablements(
    config: &Config,
) -> Result<RpcOutcome<RegistryUnpagedListResponse<ToolRegistryToolEnablement>>, String> {
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let response = transport
        .send(transport.get(REGISTRY_LIST_TOOL_ENABLEMENTS_PATH)?)
        .await?;
    Ok(RpcOutcome::single_log(
        response,
        "[youpet] listed Core registry tool enablements",
    ))
}

pub async fn registry_get_tool_enablement_version(
    config: &Config,
    params: RegistryGetToolEnablementVersionRpcParams,
) -> Result<RpcOutcome<ToolRegistryToolEnablement>, String> {
    params.validate()?;
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let path = REGISTRY_TOOL_ENABLEMENT_VERSION_PATH
        .replace("{tool_key}", &urlencoding::encode(params.tool_key.trim()))
        .replace("{version}", &params.version.to_string());
    let response = transport.send(transport.get(&path)?).await?;
    Ok(RpcOutcome::single_log(
        response,
        "[youpet] loaded Core registry tool enablement version",
    ))
}

pub async fn registry_list_connector_types(
    config: &Config,
    params: RegistryListConnectorTypesRpcParams,
) -> Result<RpcOutcome<RegistryCursorListResponse<ConnectorRegistryTypeSummary>>, String> {
    params.validate()?;
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let mut request = transport.get(REGISTRY_LIST_CONNECTOR_TYPES_PATH)?;
    request = request.query(&[("limit", params.limit_or_default())]);
    if let Some(cursor) = params
        .cursor
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request = request.query(&[("cursor", cursor)]);
    }
    let response = transport.send(request).await?;
    Ok(RpcOutcome::single_log(
        response,
        "[youpet] listed Core registry connector types",
    ))
}

pub async fn registry_get_connector_type_version(
    config: &Config,
    params: RegistryGetConnectorTypeVersionRpcParams,
) -> Result<RpcOutcome<ConnectorRegistryType>, String> {
    params.validate()?;
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let path = REGISTRY_CONNECTOR_TYPE_VERSION_PATH
        .replace(
            "{connector_key}",
            &urlencoding::encode(params.connector_key.trim()),
        )
        .replace("{version}", &params.version.to_string());
    let response = transport.send(transport.get(&path)?).await?;
    Ok(RpcOutcome::single_log(
        response,
        "[youpet] loaded Core registry connector type version",
    ))
}

pub async fn registry_list_connector_bindings(
    config: &Config,
    params: RegistryListConnectorBindingsRpcParams,
) -> Result<RpcOutcome<RegistryCursorListResponse<ConnectorRegistryBindingSummary>>, String> {
    params.validate()?;
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let mut request = transport.get(REGISTRY_LIST_CONNECTOR_BINDINGS_PATH)?;
    request = request.query(&[("limit", params.limit_or_default())]);
    if let Some(cursor) = params
        .cursor
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request = request.query(&[("cursor", cursor)]);
    }
    let response = transport.send(request).await?;
    Ok(RpcOutcome::single_log(
        response,
        "[youpet] listed Core registry connector bindings",
    ))
}

pub async fn registry_get_connector_binding_version(
    config: &Config,
    params: RegistryGetConnectorBindingVersionRpcParams,
) -> Result<RpcOutcome<ConnectorRegistryBinding>, String> {
    params.validate()?;
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let path = REGISTRY_CONNECTOR_BINDING_VERSION_PATH
        .replace(
            "{binding_key}",
            &urlencoding::encode(params.binding_key.trim()),
        )
        .replace("{version}", &params.version.to_string());
    let response = transport.send(transport.get(&path)?).await?;
    Ok(RpcOutcome::single_log(
        response,
        "[youpet] loaded Core registry connector binding version",
    ))
}
