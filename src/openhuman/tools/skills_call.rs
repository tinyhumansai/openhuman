use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::openhuman::skills::require_engine;
use crate::openhuman::tools::{Tool, ToolResult};

pub struct SkillsCallTool;

impl SkillsCallTool {
    pub fn new() -> Self {
        Self
    }

    fn validate_args(args: Value) -> anyhow::Result<(String, String, Value)> {
        let obj = args
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("arguments must be a JSON object"))?;

        let skill_id = obj
            .get("skill_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("missing required field: skill_id"))?
            .to_string();

        let tool_name = obj
            .get("tool_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("missing required field: tool_name"))?
            .to_string();

        let arguments = obj
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));

        Ok((skill_id, tool_name, arguments))
    }

    fn render_skill_output(result: &crate::openhuman::skills::types::ToolResult) -> String {
        let mut parts = Vec::new();
        for block in &result.content {
            match block {
                crate::openhuman::skills::types::ToolContent::Text { text } => {
                    parts.push(text.clone());
                }
                crate::openhuman::skills::types::ToolContent::Json { data } => {
                    parts.push(data.to_string());
                }
            }
        }

        if parts.is_empty() {
            String::new()
        } else {
            parts.join("\n")
        }
    }
}

#[async_trait]
impl Tool for SkillsCallTool {
    fn name(&self) -> &str {
        "skills_call"
    }

    fn description(&self) -> &str {
        "Call a running QuickJS skill tool by skill_id and tool_name."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "skill_id": {
                    "type": "string",
                    "description": "The runtime skill id (for example: notion, google, github)."
                },
                "tool_name": {
                    "type": "string",
                    "description": "The tool name exported by that skill."
                },
                "arguments": {
                    "type": "object",
                    "description": "Arguments for the skill tool call. Defaults to {}."
                }
            },
            "required": ["skill_id", "tool_name"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let (skill_id, tool_name, arguments) = Self::validate_args(args)?;
        let engine = require_engine().map_err(anyhow::Error::msg)?;

        let result = engine
            .call_tool(&skill_id, &tool_name, arguments)
            .await
            .map_err(anyhow::Error::msg)?;

        Ok(ToolResult {
            success: !result.is_error,
            output: Self::render_skill_output(&result),
            error: result
                .is_error
                .then(|| "skill tool reported an error".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_declares_required_fields() {
        let schema = SkillsCallTool::new().parameters_schema();
        let required = schema["required"]
            .as_array()
            .expect("required must be an array");
        assert!(required.contains(&Value::String("skill_id".to_string())));
        assert!(required.contains(&Value::String("tool_name".to_string())));
    }

    #[test]
    fn validate_args_defaults_arguments_object() {
        let (skill_id, tool_name, arguments) = SkillsCallTool::validate_args(serde_json::json!({
            "skill_id": "notion",
            "tool_name": "search"
        }))
        .expect("args should be valid");

        assert_eq!(skill_id, "notion");
        assert_eq!(tool_name, "search");
        assert!(arguments.is_object());
    }

    #[test]
    fn render_skill_output_joins_content_blocks() {
        let rendered =
            SkillsCallTool::render_skill_output(&crate::openhuman::skills::types::ToolResult {
                content: vec![
                    crate::openhuman::skills::types::ToolContent::Text {
                        text: "first".to_string(),
                    },
                    crate::openhuman::skills::types::ToolContent::Json {
                        data: serde_json::json!({"ok": true}),
                    },
                ],
                is_error: false,
            });

        assert!(rendered.contains("first"));
        assert!(rendered.contains(r#"{"ok":true}"#));
    }

    #[tokio::test]
    async fn execute_fails_when_runtime_is_not_initialized() {
        let tool = SkillsCallTool::new();
        let result = tool
            .execute(serde_json::json!({
                "skill_id": "missing-runtime",
                "tool_name": "echo",
                "arguments": {}
            }))
            .await;

        assert!(result.is_err());
        let err = result.err().expect("error expected").to_string();
        assert!(
            err.contains("skill runtime not initialized"),
            "unexpected error: {err}"
        );
    }
}
