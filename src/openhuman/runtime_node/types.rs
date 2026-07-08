use serde::Serialize;

use crate::openhuman::skills::types::ToolResult;

/// Agent-callable tool metadata exposed through `javascript.list_tools`.
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeToolSummary {
    pub name: String,
    pub description: String,
    pub category: String,
    pub permission_level: String,
    pub scope: String,
    pub supports_markdown: bool,
    pub parameters: serde_json::Value,
}

/// Result of `javascript.execute_tool`.
#[derive(Debug, Clone, Serialize)]
pub struct ExecuteToolOutcome {
    pub tool_name: String,
    pub elapsed_ms: u64,
    pub result: ToolResult,
}
