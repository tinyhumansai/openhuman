//! RPC payloads and handlers for the agent registry domain.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rpc::RpcOutcome;

use super::ops;
use super::types::{AgentRegistryEntry, AgentRegistryPatch, AgentRegistrySource, AgentToolInfo};

#[derive(Debug, Deserialize)]
pub struct ListRequest {
    #[serde(default)]
    pub include_disabled: bool,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub agents: Vec<AgentRegistryEntry>,
}

pub async fn list_rpc(req: ListRequest) -> Result<RpcOutcome<ListResponse>, String> {
    Ok(RpcOutcome::new(
        ListResponse {
            agents: ops::list_agents(req.include_disabled).await?,
        },
        vec![],
    ))
}

#[derive(Debug, Default, Deserialize)]
pub struct AvailableToolsRequest {}

#[derive(Debug, Serialize)]
pub struct AvailableToolsResponse {
    pub tools: Vec<AgentToolInfo>,
}

pub async fn available_tools_rpc(
    _req: AvailableToolsRequest,
) -> Result<RpcOutcome<AvailableToolsResponse>, String> {
    Ok(RpcOutcome::new(
        AvailableToolsResponse {
            tools: ops::available_tools().await?,
        },
        vec![],
    ))
}

#[derive(Debug, Deserialize)]
pub struct GetRequest {
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct GetResponse {
    pub agent: Option<AgentRegistryEntry>,
}

pub async fn get_rpc(req: GetRequest) -> Result<RpcOutcome<GetResponse>, String> {
    Ok(RpcOutcome::new(
        GetResponse {
            agent: ops::get_agent(&req.id).await?,
        },
        vec![],
    ))
}

#[derive(Debug, Deserialize)]
pub struct CreateCustomRequest {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    #[serde(default)]
    pub tool_denylist: Vec<String>,
    #[serde(default)]
    pub subagents: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

impl CreateCustomRequest {
    fn into_entry(self) -> AgentRegistryEntry {
        AgentRegistryEntry {
            id: self.id,
            name: self.name,
            description: self.description,
            source: AgentRegistrySource::Custom,
            enabled: self.enabled.unwrap_or(true),
            model: self.model,
            system_prompt: self.system_prompt,
            tool_allowlist: self.tool_allowlist,
            tool_denylist: self.tool_denylist,
            subagents: self.subagents,
            tags: self.tags,
            metadata: self.metadata.unwrap_or(Value::Null),
        }
    }
}

pub async fn create_custom_rpc(
    req: CreateCustomRequest,
) -> Result<RpcOutcome<AgentResponse>, String> {
    Ok(RpcOutcome::new(
        AgentResponse {
            agent: ops::upsert_custom_agent(req.into_entry()).await?,
        },
        vec![],
    ))
}

#[derive(Debug, Deserialize)]
pub struct UpsertCustomRequest {
    pub agent: AgentRegistryEntry,
}

#[derive(Debug, Serialize)]
pub struct AgentResponse {
    pub agent: AgentRegistryEntry,
}

pub async fn upsert_custom_rpc(
    req: UpsertCustomRequest,
) -> Result<RpcOutcome<AgentResponse>, String> {
    Ok(RpcOutcome::new(
        AgentResponse {
            agent: ops::upsert_custom_agent(req.agent).await?,
        },
        vec![],
    ))
}

#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub id: String,
    #[serde(flatten)]
    pub patch: AgentRegistryPatch,
}

pub async fn update_rpc(req: UpdateRequest) -> Result<RpcOutcome<AgentResponse>, String> {
    Ok(RpcOutcome::new(
        AgentResponse {
            agent: ops::update_agent(&req.id, req.patch).await?,
        },
        vec![],
    ))
}

#[derive(Debug, Deserialize)]
pub struct SetEnabledRequest {
    pub id: String,
    pub enabled: bool,
}

pub async fn set_enabled_rpc(req: SetEnabledRequest) -> Result<RpcOutcome<AgentResponse>, String> {
    Ok(RpcOutcome::new(
        AgentResponse {
            agent: ops::set_agent_enabled(&req.id, req.enabled).await?,
        },
        vec![],
    ))
}

#[derive(Debug, Deserialize)]
pub struct RemoveRequest {
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct RemoveResponse {
    pub removed: bool,
}

pub async fn remove_rpc(req: RemoveRequest) -> Result<RpcOutcome<RemoveResponse>, String> {
    Ok(RpcOutcome::new(
        RemoveResponse {
            removed: ops::remove_agent(&req.id).await?,
        },
        vec![],
    ))
}
