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

use std::future::Future;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::tools::{Tool, ToolResult};

tokio::task_local! {
    /// Task-local capture of the front end's decision payload. The decision
    /// tools echo their argument as a `ToolResult`, but the split-brain graph
    /// needs the exact `text` / `instructions` the model passed — NOT the
    /// trailing narration the agent loop returns after the tool call (which is
    /// what `run_single` yields). Each decision tool records its payload here.
    static DECISION_CAPTURE: Arc<Mutex<Option<String>>>;
}

/// Scope a front-end decision capture around one front-end agent turn `fut`,
/// returning `(turn_output, captured_payload)`. `captured_payload` is the
/// argument the model passed to `reply_to_channel` / `defer_to_orchestrator`
/// (the authoritative channel response / macro-instructions), or `None` when the
/// turn ended without calling a decision tool (caller falls back to the raw text).
pub async fn with_decision_capture<F: Future>(fut: F) -> (F::Output, Option<String>) {
    let cell = Arc::new(Mutex::new(None));
    let out = DECISION_CAPTURE.scope(cell.clone(), Box::pin(fut)).await;
    let captured = cell.lock().ok().and_then(|mut slot| slot.take());
    (out, captured)
}

/// Record a front-end decision payload from a decision tool. Last write wins
/// (the turn's terminal decision). No-op outside a [`with_decision_capture`] scope.
fn record_decision(payload: &str) {
    let _ = DECISION_CAPTURE.try_with(|cell| {
        if let Ok(mut slot) = cell.lock() {
            *slot = Some(payload.to_string());
        }
    });
}

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
            Ok(text) => {
                record_decision(&text);
                Ok(ToolResult::success(text))
            }
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
            Ok(instructions) => {
                record_decision(&instructions);
                Ok(ToolResult::success(instructions))
            }
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

    #[tokio::test]
    async fn decision_capture_surfaces_tool_payload_not_turn_narration() {
        // The runtime must send the `reply_to_channel` argument (the real reply),
        // not the model's trailing "Done — sent to the session" narration that
        // `run_single` returns. Reproduces the reply-plumbing bug.
        let reply = ReplyToChannelTool;
        let (turn_text, captured) = with_decision_capture(async {
            let _ = reply
                .execute(json!({"text": "the actual email summary"}))
                .await
                .unwrap();
            "Done — the reply has been sent to the session".to_string()
        })
        .await;
        assert_eq!(turn_text, "Done — the reply has been sent to the session");
        assert_eq!(captured.as_deref(), Some("the actual email summary"));
    }

    #[tokio::test]
    async fn decision_capture_is_none_without_a_decision_tool() {
        let (turn_text, captured) =
            with_decision_capture(async { "just narration".to_string() }).await;
        assert_eq!(turn_text, "just narration");
        assert_eq!(captured, None);
    }
}
