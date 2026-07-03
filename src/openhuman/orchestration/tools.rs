//! Orchestration front-end tools (stage 4).
//!
//! The two-pass front-end agent expresses its routing decision through two
//! early-exit tools (domain-owned per the repo tool-ownership rule):
//!
//! - [`ReplyToChannelTool`] (`reply_to_channel`) — pass 2: emit the finished
//!   `channel_response` that goes back over the tiny.place DM.
//! - [`DeferToOrchestratorTool`] (`defer_to_orchestrator`) — pass 1: hand
//!   macro-instructions down to the reasoning core.
//!
//! Both are pure "record the decision" tools: they echo their payload back as a
//! `ToolResult` and the harness [`EarlyExit`](crate::openhuman::tinyagents::EarlyExit)
//! hook captures the tool name + argument. They carry no external effect — the
//! actual DM send is the graph's `send_dm` node — so they stay `ReadOnly`.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::tools::{Tool, ToolResult};

/// `reply_to_channel` — the front end's pass-2 terminal decision.
pub struct ReplyToChannelTool;

/// `defer_to_orchestrator` — the front end's pass-1 hand-off decision.
pub struct DeferToOrchestratorTool;

/// Extract a required string field, returning an error `ToolResult` when absent.
fn required_str(args: &Value, field: &str) -> Result<String, ToolResult> {
    match args.get(field).and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => Ok(s.to_string()),
        _ => Err(ToolResult::error(format!("`{field}` is required"))),
    }
}

#[async_trait]
impl Tool for ReplyToChannelTool {
    fn name(&self) -> &str {
        "reply_to_channel"
    }

    fn description(&self) -> &str {
        "Send the finished reply back to the session over its tiny.place DM channel. \
         Call this once you have a complete answer for the counterpart."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The finished reply to send back to the session."
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        match required_str(&args, "text") {
            Ok(text) => Ok(ToolResult::success(text)),
            Err(e) => Ok(e),
        }
    }
}

#[async_trait]
impl Tool for DeferToOrchestratorTool {
    fn name(&self) -> &str {
        "defer_to_orchestrator"
    }

    fn description(&self) -> &str {
        "Hand this turn down to the reasoning core with macro-instructions. Call this \
         when the request needs real work (tools, sub-agents, multi-step reasoning) \
         rather than an immediate reply."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "instructions": {
                    "type": "string",
                    "description": "Concise macro-instructions describing what the reasoning core should do."
                }
            },
            "required": ["instructions"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        match required_str(&args, "instructions") {
            Ok(instructions) => Ok(ToolResult::success(instructions)),
            Err(e) => Ok(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reply_tool_echoes_text_and_rejects_empty() {
        let t = ReplyToChannelTool;
        assert_eq!(t.name(), "reply_to_channel");
        let ok = t.execute(json!({"text": "all done"})).await.unwrap();
        assert!(ok.text().contains("all done"));
        let bad = t.execute(json!({"text": "  "})).await.unwrap();
        assert!(bad.is_error);
    }

    #[tokio::test]
    async fn defer_tool_echoes_instructions_and_rejects_missing() {
        let t = DeferToOrchestratorTool;
        assert_eq!(t.name(), "defer_to_orchestrator");
        let ok = t
            .execute(json!({"instructions": "research X then summarize"}))
            .await
            .unwrap();
        assert!(ok.text().contains("research X"));
        let bad = t.execute(json!({})).await.unwrap();
        assert!(bad.is_error);
    }
}
