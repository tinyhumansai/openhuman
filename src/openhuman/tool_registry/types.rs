use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolRegistryEntry {
    pub tool_id: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub version: String,
    pub transport: ToolRegistryTransport,
    pub route: Value,
    pub input_schema: Value,
    pub output_schema: Value,
    pub allowed_agents: Vec<String>,
    pub tags: Vec<String>,
    pub enabled: bool,
    pub health: ToolRegistryHealth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRegistryTransport {
    JsonRpc,
    McpStdio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRegistryHealth {
    Available,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolRegistryList {
    pub tools: Vec<ToolRegistryEntry>,
}
