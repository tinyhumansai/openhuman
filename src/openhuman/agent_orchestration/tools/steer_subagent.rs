//! Tool: `steer_subagent` — inject a message into a running async sub-agent.
//!
//! Pairs with `spawn_async_subagent`: that tool returns a `task_id` for a child
//! running in the background. `steer_subagent` pushes a message into that child's
//! steering queue, which its inner `run_turn_engine` drains at the next iteration
//! boundary — so the parent can redirect or feed data to a running sub-agent
//! without waiting for it to finish or restarting it. Mirrors Codex `send_input`.

use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::run_queue::QueueMode;
use crate::openhuman::agent_orchestration::running_subagents::{self, SteerError};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct SteerSubagentTool;

impl SteerSubagentTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SteerSubagentTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SteerSubagentTool {
    fn name(&self) -> &str {
        "steer_subagent"
    }

    fn description(&self) -> &str {
        "Send a message into a running async sub-agent (one you started with \
         spawn_async_subagent), redirecting or feeding it data mid-run without \
         restarting it. The sub-agent picks the message up at its next step. Use \
         `mode: steer` (default) for a new instruction it must address, or \
         `mode: collect` for silent extra context. Returns immediately; use \
         wait_subagent to collect the final result."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["message"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Transient task_id returned by reusable async delegation."
                },
                "subagent_session_id": {
                    "type": "string",
                    "description": "Durable subagent_session_id returned by reusable async delegation. Preferred over task_id for cross-turn messaging."
                },
                "message": {
                    "type": "string",
                    "description": "Instruction or data to inject into the running sub-agent."
                },
                "mode": {
                    "type": "string",
                    "enum": ["steer", "collect"],
                    "default": "steer",
                    "description": "steer = a new instruction the sub-agent must address; collect = silent additional context."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let task_id = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let subagent_session_id = args
            .get("subagent_session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let mode = match args.get("mode").and_then(|v| v.as_str()).unwrap_or("steer") {
            "collect" => QueueMode::Collect,
            _ => QueueMode::Steer,
        };

        if task_id.is_empty() && subagent_session_id.is_empty() {
            return Ok(ToolResult::error(
                "steer_subagent: `subagent_session_id` or `task_id` is required",
            ));
        }
        if message.is_empty() {
            return Ok(ToolResult::error("steer_subagent: `message` is required"));
        }

        let parent_session = match current_parent() {
            Some(parent) => parent.session_id,
            None => {
                return Ok(ToolResult::error(
                    "steer_subagent called outside of an agent turn",
                ));
            }
        };

        let resolved_task_id = if task_id.is_empty() {
            match running_subagents::task_id_for_session(&subagent_session_id, &parent_session) {
                Ok(id) => id,
                Err(running_subagents::WaitError::Unknown) => {
                    return Ok(ToolResult::error(format!(
                        "steer_subagent: no running sub-agent with subagent_session_id `{subagent_session_id}`."
                    )));
                }
                Err(running_subagents::WaitError::NotOwned) => {
                    return Ok(ToolResult::error(format!(
                        "steer_subagent: sub-agent session `{subagent_session_id}` was not started by this agent."
                    )));
                }
            }
        } else {
            task_id.clone()
        };

        log::info!(
            "[steer_subagent] task_id={} subagent_session_id={} mode={} chars={}",
            resolved_task_id,
            if subagent_session_id.is_empty() {
                "none"
            } else {
                &subagent_session_id
            },
            mode,
            message.chars().count()
        );

        match running_subagents::steer(&resolved_task_id, &parent_session, message, mode).await {
            Ok(()) => Ok(ToolResult::success(format!(
                "Steered sub-agent `{resolved_task_id}` ({mode}). It will pick this up at its next step. \
                 Use wait_subagent with its subagent_session_id or task_id to collect its result."
            ))),
            Err(SteerError::Unknown) => Ok(ToolResult::error(format!(
                "steer_subagent: no running sub-agent with task_id `{resolved_task_id}`. It may have already \
                 finished — use wait_subagent to collect its result, or check the task_id."
            ))),
            Err(SteerError::NotOwned) => Ok(ToolResult::error(format!(
                "steer_subagent: sub-agent `{resolved_task_id}` was not started by this agent and cannot be steered."
            ))),
            Err(SteerError::AlreadyDone) => Ok(ToolResult::error(format!(
                "steer_subagent: sub-agent `{resolved_task_id}` has already finished. Use wait_subagent to collect its result."
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_requires_task_id_and_message() {
        let schema = SteerSubagentTool::new().parameters_schema();
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("required list");
        assert!(required.iter().any(|v| v.as_str() == Some("message")));
    }

    #[tokio::test]
    async fn missing_task_id_is_rejected() {
        let tool = SteerSubagentTool::new();
        let res = tool.execute(json!({ "message": "go" })).await.unwrap();
        assert!(res.is_error);
        assert!(res.output().contains("subagent_session_id"));
    }

    #[tokio::test]
    async fn missing_message_is_rejected() {
        let tool = SteerSubagentTool::new();
        let res = tool.execute(json!({ "task_id": "sub-1" })).await.unwrap();
        assert!(res.is_error);
        assert!(res.output().contains("message"));
    }

    #[tokio::test]
    async fn outside_agent_turn_is_rejected() {
        // No PARENT_CONTEXT task-local installed in a bare test.
        let tool = SteerSubagentTool::new();
        let res = tool
            .execute(json!({ "task_id": "sub-1", "message": "go" }))
            .await
            .unwrap();
        assert!(res.is_error);
        assert!(res.output().contains("outside of an agent turn"));
    }
}
