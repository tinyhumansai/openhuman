use serde::Serialize;

use tinytools::ToolResult;

/// Result of [`super::native_ops::execute_tool_with_security`].
#[derive(Debug, Clone, Serialize)]
pub struct ExecuteToolOutcome {
    pub tool_name: String,
    pub elapsed_ms: u64,
    pub result: ToolResult,
}
