//! Tool: ask_user_clarification — pause execution and ask the user a question.

use super::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Pauses the current execution to ask the user for clarification.
///
/// In the orchestrator flow, this surfaces the question to the user via the
/// event channel and waits for a response before continuing.
pub struct AskClarificationTool;

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
