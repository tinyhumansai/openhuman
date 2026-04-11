use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};

pub struct ArchetypeDelegationTool {
    pub tool_name: String,
    pub agent_id: String,
    pub tool_description: String,
}

#[async_trait]
impl Tool for ArchetypeDelegationTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Clear instruction for what to do. Include all relevant context — the sub-agent has no memory of your conversation."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        if prompt.is_empty() {
            return Ok(ToolResult::error(format!(
                "{}: `prompt` is required",
                self.tool_name
            )));
        }

        super::dispatch_subagent(&self.agent_id, &self.tool_name, &prompt, None).await
    }
}
