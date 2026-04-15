//! Tool: ask_user_clarification — pause execution and ask the user a question.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Pauses the current execution to ask the user for clarification.
///
/// In the orchestrator flow, this surfaces the question to the user via the
/// event channel and waits for a response before continuing.
pub struct AskClarificationTool;

impl Default for AskClarificationTool {
    fn default() -> Self {
        Self::new()
    }
}

impl AskClarificationTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for AskClarificationTool {
    fn name(&self) -> &str {
        "ask_user_clarification"
    }

    fn description(&self) -> &str {
        "Ask the user a clarifying question when the task is ambiguous or requires \
         a decision. The question will be shown to the user and their response returned. \
         Use sparingly — only when the answer cannot be inferred from context."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The clarifying question to ask the user. \
                                   If omitted, a generic clarification prompt is used."
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of choices to present to the user."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let question = args
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("Could you clarify?");

        let options = args.get("options").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        });

        let mut output = format!("[CLARIFICATION NEEDED]\n{question}");
        if let Some(opts) = options {
            output.push_str(&format!("\n\nOptions: {opts}"));
        }

        // In a full implementation, this would:
        // 1. Emit an event to the frontend/CLI.
        // 2. Block on a response channel.
        // 3. Return the user's answer.
        // For now, return the question as output so the orchestrator can surface it.
        tracing::info!("[ask_clarification] question: {question}");

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn name_is_correct() {
        assert_eq!(AskClarificationTool::new().name(), "ask_user_clarification");
    }

    #[test]
    fn description_is_non_empty() {
        assert!(!AskClarificationTool::new().description().is_empty());
    }

    #[test]
    fn schema_is_object_type() {
        let schema = AskClarificationTool::new().parameters_schema();
        assert_eq!(schema["type"], "object");
    }

    #[test]
    fn permission_level_is_none() {
        assert_eq!(
            AskClarificationTool::new().permission_level(),
            PermissionLevel::None
        );
    }

    #[test]
    fn default_and_new_are_equivalent() {
        let a = AskClarificationTool::new();
        let b = AskClarificationTool::default();
        assert_eq!(a.name(), b.name());
    }

    #[tokio::test]
    async fn execute_with_question_includes_question_in_output() {
        let tool = AskClarificationTool::new();
        let result = tool
            .execute(json!({ "question": "Which branch should I target?" }))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("Which branch should I target?"));
    }

    #[tokio::test]
    async fn execute_with_options_lists_choices() {
        let tool = AskClarificationTool::new();
        let result = tool
            .execute(json!({
                "question": "Which env?",
                "options": ["staging", "production"]
            }))
            .await
            .unwrap();
        assert!(!result.is_error);
        let out = result.output();
        assert!(out.contains("staging"));
        assert!(out.contains("production"));
    }

    #[tokio::test]
    async fn execute_without_question_uses_fallback() {
        let tool = AskClarificationTool::new();
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("CLARIFICATION NEEDED"));
    }
}
