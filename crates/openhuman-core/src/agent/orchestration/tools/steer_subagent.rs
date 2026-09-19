//! Tool: `steer_subagent` — inject a message into a running async sub-agent.
//!
//! Pairs with `spawn_async_subagent`: that tool returns a `task_id` for a child
//! running in the background. `steer_subagent` pushes a message into that child's
//! steering queue, which the steering forwarder in the child's turn
//! (`run_turn_via_tinyagents_shared`) drains mid-flight — so the parent can
//! redirect or feed data to a running sub-agent
//! without waiting for it to finish or restarting it. Mirrors Codex `send_input`.

use crate::agent::harness::fork_context::ParentExecutionContext;
use crate::agent::orchestration::running_subagents::{self, SteerError};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::run_queue::QueueLane;
use tinyagents_harness::tool::{ToolDispatch, ToolExecutionContext};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult};

pub struct SteerSubagentTool;

pub(crate) struct SteerSubagentDispatch {
    tool: Arc<dyn Tool>,
}
impl SteerSubagentDispatch {
    pub(crate) fn new(tool: Arc<dyn Tool>) -> Self {
        Self { tool }
    }
}
#[async_trait]
impl ToolDispatch<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for SteerSubagentDispatch
{
    fn tool(&self) -> Arc<dyn Tool> {
        self.tool.clone()
    }
    async fn execute(
        &self,
        _state: &(),
        arguments: serde_json::Value,
        _options: ToolCallOptions,
        parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let context = ToolExecutionContext::from_run_context(parent);
        SteerSubagentTool::new()
            .execute_with_parent_context(arguments, parent.data.parent.clone(), Some(&context))
            .await
    }
}

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
        "Send a message into a running async sub-agent, redirecting it or feeding it data mid-run without restarting. It is picked up at the next step. `mode: steer` (default) is an instruction it must address; `mode: collect` is silent context. Returns immediately."
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
        self.execute_with_parent_context(args, None, None).await
    }
}

impl SteerSubagentTool {
    async fn execute_with_parent_context(
        &self,
        args: serde_json::Value,
        parent: Option<ParentExecutionContext>,
        _tool_context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
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
        let (lane, mode) = match args.get("mode").and_then(|v| v.as_str()).unwrap_or("steer") {
            "collect" => (QueueLane::Collect, "collect"),
            _ => (QueueLane::Steer, "steer"),
        };

        if task_id.is_empty() && subagent_session_id.is_empty() {
            return Ok(ToolResult::error(
                "steer_subagent: `subagent_session_id` or `task_id` is required",
            ));
        }
        if message.is_empty() {
            return Ok(ToolResult::error("steer_subagent: `message` is required"));
        }

        let parent = match parent {
            Some(parent) => parent,
            None => {
                return Ok(ToolResult::error(
                    "steer_subagent called outside of an agent turn",
                ));
            }
        };
        let parent_session = parent.session_id;

        let resolved_task_id = if task_id.is_empty() {
            match running_subagents::task_id_for_session_in_workspace(
                &subagent_session_id,
                &parent_session,
                &parent.workspace_dir,
            ) {
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

        match running_subagents::steer(&resolved_task_id, &parent_session, message, lane).await {
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
            Err(SteerError::UnsupportedLane) => Ok(ToolResult::error(
                "steer_subagent: only steer and collect messages can be delivered to a running sub-agent",
            )),
        }
    }
}

#[cfg(test)]
#[path = "steer_subagent_tests.rs"]
mod tests;
