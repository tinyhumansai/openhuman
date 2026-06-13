//! Tool: `wait_subagent` — block until a running async sub-agent finishes.
//!
//! Pairs with `spawn_async_subagent` / `steer_subagent`: once the parent has
//! fanned out background work, `wait_subagent` collects a child's final result
//! inline (with a timeout), instead of relying solely on lifecycle events.
//! Mirrors Codex `wait`.

use std::time::Duration;

use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent_orchestration::running_subagents::{
    self, SubagentStatus, WaitError, WaitOutcome,
};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;

pub struct WaitSubagentTool;

impl WaitSubagentTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WaitSubagentTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WaitSubagentTool {
    fn name(&self) -> &str {
        "wait_subagent"
    }

    fn description(&self) -> &str {
        "Block until an async sub-agent (started with spawn_async_subagent) \
         finishes, then return its final result. Optionally bound the wait with \
         `timeout_secs` (default 120, max 600); on timeout it reports the \
         sub-agent is still running and you can call wait_subagent again."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task_id returned by spawn_async_subagent."
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_TIMEOUT_SECS,
                    "description": "Max seconds to block before returning a 'still running' result. Default 120."
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
        if task_id.is_empty() {
            return Ok(ToolResult::error("wait_subagent: `task_id` is required"));
        }

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS);

        let parent_session = match current_parent() {
            Some(parent) => parent.session_id,
            None => {
                return Ok(ToolResult::error(
                    "wait_subagent called outside of an agent turn",
                ));
            }
        };

        log::info!(
            "[wait_subagent] task_id={} timeout_secs={}",
            task_id,
            timeout_secs
        );

        match running_subagents::wait(
            &task_id,
            &parent_session,
            Duration::from_secs(timeout_secs),
        )
        .await
        {
            Ok(WaitOutcome::Terminal(SubagentStatus::Completed { output, iterations })) => {
                Ok(ToolResult::success(format!(
                    "Sub-agent `{task_id}` completed in {iterations} iteration(s):\n\n{output}"
                )))
            }
            Ok(WaitOutcome::Terminal(SubagentStatus::AwaitingUser { question })) => {
                Ok(ToolResult::success(format!(
                    "Sub-agent `{task_id}` paused for clarification and did not finish: {question}\n\n\
                     It cannot proceed unattended. Resume it with continue_subagent once you have an answer."
                )))
            }
            Ok(WaitOutcome::Terminal(SubagentStatus::Failed { error })) => Ok(ToolResult::error(
                format!("Sub-agent `{task_id}` failed: {error}"),
            )),
            // `Running` is never terminal; treat defensively as a timeout-style result.
            Ok(WaitOutcome::Terminal(SubagentStatus::Running))
            | Ok(WaitOutcome::TimedOut(_)) => Ok(ToolResult::success(format!(
                "Sub-agent `{task_id}` is still running after {timeout_secs}s. \
                 Continue with other work and call wait_subagent again later, or steer_subagent to redirect it."
            ))),
            Err(WaitError::Unknown) => Ok(ToolResult::error(format!(
                "wait_subagent: no sub-agent with task_id `{task_id}`. It may have already finished and \
                 been collected, or the task_id is wrong."
            ))),
            Err(WaitError::NotOwned) => Ok(ToolResult::error(format!(
                "wait_subagent: sub-agent `{task_id}` was not started by this agent."
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_requires_task_id() {
        let schema = WaitSubagentTool::new().parameters_schema();
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("required list");
        assert!(required.iter().any(|v| v.as_str() == Some("task_id")));
    }

    #[tokio::test]
    async fn missing_task_id_is_rejected() {
        let res = WaitSubagentTool::new().execute(json!({})).await.unwrap();
        assert!(res.is_error);
        assert!(res.output().contains("task_id"));
    }

    #[tokio::test]
    async fn outside_agent_turn_is_rejected() {
        let res = WaitSubagentTool::new()
            .execute(json!({ "task_id": "sub-1" }))
            .await
            .unwrap();
        assert!(res.is_error);
        assert!(res.output().contains("outside of an agent turn"));
    }
}
