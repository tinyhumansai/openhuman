//! RPC payloads and handlers for the agent registry domain.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;

use super::ops;
use super::types::{
    AgentRegistryEntry, AgentRegistryPatch, AgentRegistrySource, AgentSubagentPolicy, AgentToolInfo,
};

#[derive(Debug, Deserialize)]
pub struct ListRequest {
    #[serde(default)]
    pub include_disabled: bool,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub agents: Vec<AgentRegistryEntry>,
}

pub async fn list_rpc(
    ctx: &DispatchCtx,
    req: ListRequest,
) -> Result<Outcome<ListResponse>, String> {
    Ok(Outcome::new(
        ListResponse {
            agents: ops::list_agents(ctx, req.include_disabled).await?,
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
    ctx: &DispatchCtx,
    _req: AvailableToolsRequest,
) -> Result<Outcome<AvailableToolsResponse>, String> {
    Ok(Outcome::new(
        AvailableToolsResponse {
            tools: ops::available_tools(ctx).await?,
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

pub async fn get_rpc(ctx: &DispatchCtx, req: GetRequest) -> Result<Outcome<GetResponse>, String> {
    Ok(Outcome::new(
        GetResponse {
            agent: ops::get_agent(ctx, &req.id).await?,
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
    pub tool_rules: Option<tinytools::ToolRules>,
    #[serde(default)]
    pub subagents: AgentSubagentPolicy,
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
            // A new entry never stores a no-op rule set. (Clearing is an
            // `update`: `AgentRegistryPatch` passes an empty set through to
            // `apply_patch`, which clears the field.)
            tool_rules: self.tool_rules.filter(|rules| !rules.is_permissive()),
            subagents: self.subagents,
            tags: self.tags,
            metadata: self.metadata.unwrap_or(Value::Null),
        }
    }
}

pub async fn create_custom_rpc(
    ctx: &DispatchCtx,
    req: CreateCustomRequest,
) -> Result<Outcome<AgentResponse>, String> {
    Ok(Outcome::new(
        AgentResponse {
            agent: ops::upsert_custom_agent(ctx, req.into_entry()).await?,
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
    ctx: &DispatchCtx,
    req: UpsertCustomRequest,
) -> Result<Outcome<AgentResponse>, String> {
    Ok(Outcome::new(
        AgentResponse {
            agent: ops::upsert_custom_agent(ctx, req.agent).await?,
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

pub async fn update_rpc(
    ctx: &DispatchCtx,
    req: UpdateRequest,
) -> Result<Outcome<AgentResponse>, String> {
    Ok(Outcome::new(
        AgentResponse {
            agent: ops::update_agent(ctx, &req.id, req.patch).await?,
        },
        vec![],
    ))
}

#[derive(Debug, Deserialize)]
pub struct SetEnabledRequest {
    pub id: String,
    pub enabled: bool,
}

pub async fn set_enabled_rpc(
    ctx: &DispatchCtx,
    req: SetEnabledRequest,
) -> Result<Outcome<AgentResponse>, String> {
    Ok(Outcome::new(
        AgentResponse {
            agent: ops::set_agent_enabled(ctx, &req.id, req.enabled).await?,
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

pub async fn remove_rpc(
    ctx: &DispatchCtx,
    req: RemoveRequest,
) -> Result<Outcome<RemoveResponse>, String> {
    Ok(Outcome::new(
        RemoveResponse {
            removed: ops::remove_agent(ctx, &req.id).await?,
        },
        vec![],
    ))
}
