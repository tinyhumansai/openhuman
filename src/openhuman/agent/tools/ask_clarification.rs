//! Tool: ask_user_clarification — pause execution and ask the user a question.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Pauses the current execution to ask the user for clarification.
///
/// The pause is NOT implemented here — this tool only returns the question as
/// its output. The turn stops because `ask_user_clarification` is registered as
/// an *early-exit tool* on the harness seam
/// (`tinyagents::run_turn_via_tinyagents_shared`'s `early_exit_tools`): on a
/// successful call the hook records the output as the pause question and steers
/// the loop to `Pause`, so the caller ends the turn with that question as its
/// text instead of feeding this result back to the model.
///
/// Every caller that exposes this tool MUST name it in `early_exit_tools`.
/// A caller that does not gets a tool that answers its own question: the model
/// reads this output as a successful result and carries on without ever asking
/// (the chat and channel paths did exactly that until this was wired up).
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
        // Blank counts as absent. The schema has no `minLength`, so
        // `{"question":""}` is a legal call, and `as_str()` answers `Some("")` —
        // which slipped past the missing-argument fallback below. That was
        // survivable only while the `[CLARIFICATION NEEDED]` prefix kept the
        // output non-empty; now that this output *is* the message the user
        // reads, an empty one would park the turn on a blank prompt: a blank
        // assistant reply on the channel path, and an `AwaitingUser` sub-agent
        // card with nothing in it. Trimmed for the same reason — a whitespace
        // question is a blank one (#6213 review).
        let question = args
            .get("question")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .unwrap_or("Could you clarify?");

        let options = args.get("options").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        });

        // Plain question text, no marker: this output IS the message the user
        // reads. The early-exit hook captures it verbatim as the pause question,
        // which the chat path returns as the turn's reply and the sub-agent path
        // carries on `SubagentRunStatus::AwaitingUser`. Nothing anywhere parsed
        // the old `[CLARIFICATION NEEDED]` prefix — it only ever leaked into the
        // user's face.
        let mut output = question.to_string();
        if let Some(opts) = options {
            output.push_str(&format!("\n\nOptions: {opts}"));
        }

        tracing::info!("[ask_clarification] question: {question}");

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
#[path = "ask_clarification_tests.rs"]
mod tests;
