use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewMcpWriteRecord {
    pub timestamp_ms: i64,
    pub client_info: String,
    pub tool_name: String,
    pub args_summary: Value,
    pub resulting_chunk_id: Option<String>,
    pub success: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpWriteRecord {
    pub id: i64,
    pub timestamp_ms: i64,
    pub client_info: String,
    pub tool_name: String,
    pub args_summary: Value,
    pub resulting_chunk_id: Option<String>,
    pub success: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct McpWriteListQuery {
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub offset: Option<u64>,
    #[serde(default)]
    pub since_ms: Option<u64>,
    #[serde(default)]
    pub client_filter: Option<String>,
    #[serde(default)]
    pub tool_filter: Option<String>,
    #[serde(default)]
    pub success_only: Option<bool>,
}
