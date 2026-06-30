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
            "required": [],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Transient task_id returned by reusable async delegation."
                },
                "subagent_session_id": {
                    "type": "string",
                    "description": "Durable subagent_session_id returned by reusable async delegation. Preferred for cross-turn waits."
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
        let subagent_session_id = args
            .get("subagent_session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if task_id.is_empty() && subagent_session_id.is_empty() {
            return Ok(ToolResult::error(
                "wait_subagent: `subagent_session_id` or `task_id` is required",
            ));
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

        let resolved_task_id = if task_id.is_empty() {
            match running_subagents::task_id_for_session(&subagent_session_id, &parent_session) {
                Ok(id) => id,
                Err(WaitError::Unknown) => {
                    return Ok(ToolResult::error(format!(
                        "wait_subagent: no running sub-agent with subagent_session_id `{subagent_session_id}`."
                    )));
                }
                Err(WaitError::NotOwned) => {
                    return Ok(ToolResult::error(format!(
                        "wait_subagent: sub-agent session `{subagent_session_id}` was not started by this agent."
                    )));
                }
            }
        } else {
            task_id.clone()
        };

        log::info!(
            "[wait_subagent] task_id={} subagent_session_id={} timeout_secs={}",
            resolved_task_id,
            if subagent_session_id.is_empty() {
                "none"
            } else {
                &subagent_session_id
            },
            timeout_secs
        );

        let resume_ref =
            running_subagents::resume_ref_for_task(&resolved_task_id, &parent_session).ok();

        match running_subagents::wait(
            &resolved_task_id,
            &parent_session,
            Duration::from_secs(timeout_secs),
        )
        .await
        {
            Ok(WaitOutcome::Terminal(SubagentStatus::Completed { output, iterations })) => {
                log::debug!(
                    "[wait_subagent] outcome=completed task_id={} iterations={}",
                    resolved_task_id,
                    iterations
                );
                let status = wait_status_payload(
                    resume_ref.as_ref(),
                    &resolved_task_id,
                    "completed",
                    Some(iterations),
                    None,
                    "synthesize the sub-agent output into the parent response",
                );
                Ok(ToolResult::success(format!(
                    "Sub-agent `{}` completed in {iterations} iteration(s).\n\n[subagent_wait_result]\n{}\n[/subagent_wait_result]\n\n{output}",
                    status["agent_id"].as_str().unwrap_or("subagent"),
                    serde_json::to_string(&status).unwrap_or_else(|_| "{}".to_string())
                )))
            }
            Ok(WaitOutcome::Terminal(SubagentStatus::AwaitingUser { question })) => {
                log::debug!(
                    "[wait_subagent] outcome=awaiting_user task_id={} question_chars={}",
                    resolved_task_id,
                    question.chars().count()
                );
                let status = wait_status_payload(
                    resume_ref.as_ref(),
                    &resolved_task_id,
                    "awaiting_user",
                    None,
                    Some(&question),
                    "ask the user for the missing information, then call continue_subagent",
                );
                let mut message = format!(
                    "Sub-agent `{}` paused for clarification and did not finish: {question}\n\n\
                     It cannot proceed unattended. Resume it with continue_subagent once you have an answer.\n\n[subagent_wait_result]\n{}\n[/subagent_wait_result]",
                    status["agent_id"].as_str().unwrap_or("subagent"),
                    serde_json::to_string(&status).unwrap_or_else(|_| "{}".to_string())
                );
                if let Some(reference) = resume_ref {
                    message.push_str("\n\n[subagent_resume_ref]\n");
                    message.push_str(
                        &serde_json::to_string(&serde_json::json!({
                            "task_id": reference.task_id,
                            "agent_id": reference.agent_id,
                            "subagent_session_id": reference.subagent_session_id,
                            "tool": "continue_subagent"
                        }))
                        .unwrap_or_else(|_| "{}".to_string()),
                    );
                    message.push_str("\n[/subagent_resume_ref]");
                } else {
                    log::debug!(
                        "[wait_subagent] resume_ref_unavailable task_id={}",
                        resolved_task_id
                    );
                }
                Ok(ToolResult::success(message))
            }
            Ok(WaitOutcome::Terminal(SubagentStatus::Failed { error })) => {
                log::debug!(
                    "[wait_subagent] outcome=failed task_id={} error={}",
                    resolved_task_id,
                    error
                );
                let status = wait_status_payload(
                    resume_ref.as_ref(),
                    &resolved_task_id,
                    "failed",
                    None,
                    Some(&error),
                    "report the failure or retry with a corrected instruction",
                );
                Ok(ToolResult::error(format!(
                    "Sub-agent `{}` failed: {error}\n\n[subagent_wait_result]\n{}\n[/subagent_wait_result]",
                    status["agent_id"].as_str().unwrap_or("subagent"),
                    serde_json::to_string(&status).unwrap_or_else(|_| "{}".to_string())
                )))
            }
            // `Running` is never terminal; treat defensively as a timeout-style result.
            Ok(WaitOutcome::Terminal(SubagentStatus::Running)) => {
                log::debug!(
                    "[wait_subagent] outcome=running task_id={} timeout_secs={}",
                    resolved_task_id,
                    timeout_secs
                );
                Ok(ToolResult::success(format_running_wait_message(
                    resume_ref.as_ref(),
                    &resolved_task_id,
                    timeout_secs,
                )))
            }
            Ok(WaitOutcome::TimedOut(_)) => {
                log::debug!(
                    "[wait_subagent] outcome=timed_out task_id={} timeout_secs={}",
                    resolved_task_id,
                    timeout_secs
                );
                Ok(ToolResult::success(format_running_wait_message(
                    resume_ref.as_ref(),
                    &resolved_task_id,
                    timeout_secs,
                )))
            }
            Err(WaitError::Unknown) => {
                log::debug!(
                    "[wait_subagent] outcome=unknown task_id={}",
                    resolved_task_id
                );
                Ok(ToolResult::error(format!(
                    "wait_subagent: no sub-agent was found for that reference. It may have already finished and \
                     been collected, or the task_id is wrong."
                )))
            }
            Err(WaitError::NotOwned) => {
                log::debug!(
                    "[wait_subagent] outcome=not_owned task_id={}",
                    resolved_task_id
                );
                Ok(ToolResult::error(format!(
                    "wait_subagent: that sub-agent was not started by this agent."
                )))
            }
        }
    }
}

/// Render a timeout/running wait response with a structured status payload.
fn format_running_wait_message(
    reference: Option<&running_subagents::SubagentResumeRef>,
    task_id: &str,
    timeout_secs: u64,
) -> String {
    let status = wait_status_payload(
        reference,
        task_id,
        "running",
        None,
        None,
        "continue other work, call wait_subagent again, or call steer_subagent to send more input",
    );
    format!(
        "Sub-agent `{}` is still running after {timeout_secs}s.\n\n[subagent_wait_result]\n{}\n[/subagent_wait_result]\n\nContinue with other work and call wait_subagent again later, or steer_subagent to redirect it.",
        status["agent_id"].as_str().unwrap_or("subagent"),
        serde_json::to_string(&status).unwrap_or_else(|_| "{}".to_string())
    )
}

/// Build the machine-readable wait status block returned to the orchestrator.
fn wait_status_payload(
    reference: Option<&running_subagents::SubagentResumeRef>,
    task_id: &str,
    status: &str,
    iterations: Option<usize>,
    detail: Option<&str>,
    next_action: &str,
) -> serde_json::Value {
    let agent_id = reference.map(|r| r.agent_id.as_str()).unwrap_or("unknown");
    let subagent_session_id = reference.and_then(|r| r.subagent_session_id.as_deref());
    json!({
        "task_id": task_id,
        "taskId": task_id,
        "subagent_session_id": subagent_session_id,
        "subagentSessionId": subagent_session_id,
        "agent_id": agent_id,
        "agentId": agent_id,
        "status": status,
        "iterations": iterations,
        "detail": detail,
        "next_action": next_action,
        "nextAction": next_action,
        "instructions": {
            "send_message": {
                "tool": "steer_subagent",
                "arguments": {
                    "subagent_session_id": subagent_session_id,
                    "task_id": task_id,
                    "message": "<message>",
                    "mode": "steer"
                }
            },
            "wait": {
                "tool": "wait_subagent",
                "arguments": {
                    "subagent_session_id": subagent_session_id,
                    "task_id": task_id,
                    "timeout_secs": DEFAULT_TIMEOUT_SECS
                }
            },
            "timeout_tick": {
                "tool": "wait_subagent",
                "arguments": {
                    "subagent_session_id": subagent_session_id,
                    "task_id": task_id,
                    "timeout_secs": 1
                }
            }
        }
    })
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
        assert!(required.is_empty());
    }

    #[tokio::test]
    async fn missing_task_id_is_rejected() {
        let res = WaitSubagentTool::new().execute(json!({})).await.unwrap();
        assert!(res.is_error);
        assert!(res.output().contains("subagent_session_id"));
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

    #[test]
    fn running_wait_message_includes_agent_id_and_tick_instruction() {
        let reference = running_subagents::SubagentResumeRef {
            task_id: "sub-1".into(),
            agent_id: "researcher".into(),
            subagent_session_id: Some("subsess-1".into()),
        };
        let message = format_running_wait_message(Some(&reference), "sub-1", 1);

        assert!(message.contains("Sub-agent `researcher` is still running"));
        assert!(message.contains("[subagent_wait_result]"));
        assert!(message.contains("\"agentId\":\"researcher\""));
        assert!(message.contains("\"timeout_tick\""));
        assert!(message.contains("\"timeout_secs\":1"));
    }
}
