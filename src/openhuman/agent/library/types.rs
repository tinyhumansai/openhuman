use serde::{Deserialize, Serialize};

use crate::openhuman::agent::harness::definition::ToolScope;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDefinitionSource {
    Builtin,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDefinitionModel {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinitionDisplay {
    pub id: String,
    pub display_name: String,
    pub when_to_use: String,
    pub tier: String,
    pub model: AgentDefinitionModel,
    pub tools: ToolScope,
    pub direct_tool_count: usize,
    pub direct_tool_names: Vec<String>,
    pub uses_wildcard_tools: bool,
    pub subagent_ids: Vec<String>,
    pub includes_profile: bool,
    pub includes_memory_md: bool,
    pub includes_memory_context: bool,
    pub can_run_as_user_facing_worker: bool,
    pub write_capable: bool,
    pub source: AgentDefinitionSource,
}
