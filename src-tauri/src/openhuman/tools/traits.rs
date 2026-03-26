use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Result of a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

/// Description of a tool for the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Core tool trait — implement for any capability
#[async_trait]
pub trait Tool: Send + Sync {
    /// Tool name (used in LLM function calling)
    fn name(&self) -> &str;

    /// Human-readable description
    fn description(&self) -> &str;

    /// JSON schema for parameters
    fn parameters_schema(&self) -> serde_json::Value;

    /// Execute the tool with given arguments
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult>;

    /// Get the full spec for LLM registration
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyTool;

    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "dummy_tool"
        }

        fn description(&self) -> &str {
            "A deterministic test tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                }
            })
        }

        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult {
                success: true,
                output: args
                    .get("value")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                error: None,
            })
        }
    }

    #[test]
    fn spec_uses_tool_metadata_and_schema() {
        let tool = DummyTool;
        let spec = tool.spec();

        assert_eq!(spec.name, "dummy_tool");
        assert_eq!(spec.description, "A deterministic test tool");
        assert_eq!(spec.parameters["type"], "object");
        assert_eq!(spec.parameters["properties"]["value"]["type"], "string");
    }

    #[tokio::test]
    async fn execute_returns_expected_output() {
        let tool = DummyTool;
        let result = tool
            .execute(serde_json::json!({ "value": "hello-tool" }))
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.output, "hello-tool");
        assert!(result.error.is_none());
    }

    #[test]
    fn tool_result_serialization_roundtrip() {
        let result = ToolResult {
            success: false,
            output: String::new(),
            error: Some("boom".into()),
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ToolResult = serde_json::from_str(&json).unwrap();

        assert!(!parsed.success);
        assert_eq!(parsed.error.as_deref(), Some("boom"));
    }
}
