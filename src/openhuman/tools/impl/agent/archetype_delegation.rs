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
                },
                "model": {
                    "type": "string",
                    "description": "Optional exact model id for this delegation only. Keeps the parent provider/routing, but pins the child agent to this model instead of the agent definition's default."
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

        let model_override = args
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        super::dispatch_subagent(
            &self.agent_id,
            &self.tool_name,
            &prompt,
            None,
            model_override,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;

    fn sample_tool() -> ArchetypeDelegationTool {
        ArchetypeDelegationTool {
            tool_name: "delegate_researcher".to_string(),
            agent_id: "researcher".to_string(),
            tool_description: "Use for web and docs research.".to_string(),
        }
    }

    #[test]
    fn metadata_methods_expose_name_description_and_system_category() {
        let tool = sample_tool();
        assert_eq!(tool.name(), "delegate_researcher");
        assert_eq!(tool.description(), "Use for web and docs research.");
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
        assert_eq!(tool.category(), ToolCategory::System);
    }

    #[test]
    fn parameters_schema_requires_prompt_only() {
        let tool = sample_tool();
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!(["prompt"]));
        assert_eq!(schema["properties"]["prompt"]["type"], "string");
    }

    #[tokio::test]
    async fn execute_rejects_missing_or_blank_prompt() {
        let tool = sample_tool();

        let missing = tool.execute(json!({})).await.unwrap();
        assert!(missing.is_error);
        assert!(missing.output().contains("`prompt` is required"));

        let blank = tool.execute(json!({ "prompt": "   " })).await.unwrap();
        assert!(blank.is_error);
        assert!(blank.output().contains("`prompt` is required"));
    }

    #[tokio::test]
    async fn execute_accepts_non_empty_prompt_and_reaches_dispatch_path() {
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let tool = sample_tool();
        let result = tool
            .execute(json!({ "prompt": "find the answer" }))
            .await
            .unwrap();

        let out = result.output();
        assert!(
            !out.contains("`prompt` is required"),
            "non-empty prompt should bypass local validation, got: {out}"
        );
    }
}
