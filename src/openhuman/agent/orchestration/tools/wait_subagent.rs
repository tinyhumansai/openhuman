//! Tool: `wait_subagent` — block until a running async sub-agent finishes.
//!
//! Pairs with `spawn_async_subagent` / `steer_subagent`: once the parent has
//! fanned out background work, `wait_subagent` collects a child's final result
//! inline (with a timeout), instead of relying solely on lifecycle events.
//! Mirrors Codex `wait`.

use std::time::Duration;

use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::orchestration::running_subagents::{
    self, SubagentStatus, WaitError, WaitOutcome,
};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolTimeout};
use async_trait::async_trait;
use serde_json::json;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Ceiling on a single wait. Matches Codex's `wait_agent` maximum of one hour:
/// a wait that is long is fine, because reaching it is a *successful* "still
/// running" result the model can act on, not a failure.
const MAX_TIMEOUT_SECS: u64 = 3600;

/// The wait this call should perform, resolved identically for the harness
/// deadline ([`WaitSubagentTool::timeout_policy`]) and the wait itself.
///
/// These two MUST agree. This tool is designed to be polled: on expiry it
/// returns [`ToolResult::success`] saying the sub-agent is still running and
/// inviting another call. That graceful path is only reachable if the tool
/// out-lives its own wait — under the default [`ToolTimeout::Inherit`] the
/// generic 120s per-tool-call deadline killed the *tool* first, turning a
/// designed success into an `error` result. The repeated-failure breaker then
/// classifies "timed out" as transient, retries the identical call 8 times and
/// halts the turn — which is how a working coding sub-agent lost its work.
fn requested_timeout_secs(args: &serde_json::Value) -> u64 {
    args.get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS)
}

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
        "Block until an async sub-agent finishes and return its result. `timeout_secs` defaults to 120, max 3600; timing out is a normal outcome you can retry, so prefer one long wait over tight polling."
    }

    /// Let the wait own its own deadline instead of inheriting the generic
    /// per-tool-call one.
    ///
    /// Without this the harness kills the tool at the global `Inherit` timeout
    /// (120s by default) *before* the wait can return its "still running"
    /// success, so a legitimate long wait is reported as a tool failure. The
    /// harness adds its own small grace on top of `Secs`, so the tool always
    /// gets to finish and report first. Same reasoning as
    /// `spawn_parallel_agents`, which opts out for the same class of bug.
    fn timeout_policy(&self, args: &serde_json::Value) -> ToolTimeout {
        ToolTimeout::Secs(requested_timeout_secs(args))
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

        let timeout_secs = requested_timeout_secs(&args);

        let parent = match current_parent() {
            Some(parent) => parent,
            None => {
                return Ok(ToolResult::error(
                    "wait_subagent called outside of an agent turn",
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

        let resume_ref = running_subagents::resume_ref_for_task_in_workspace(
            &resolved_task_id,
            &parent_session,
            &parent.workspace_dir,
        )
        .ok();

        match running_subagents::wait_in_workspace(
            &resolved_task_id,
            &parent_session,
            &parent.workspace_dir,
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
                // The parent is collecting this result inline and will present
                // it in this turn, so suppress the detached completion's separate
                // background-delivery turn — otherwise the same result is
                // re-answered as a duplicate. Only the Completed arm marks:
                // AwaitingUser/Failed never record a completion, and a
                // still-Running/TimedOut sub-agent has no terminal result yet, so
                // a genuinely-later completion must still surface.
                crate::openhuman::agent::orchestration::background_completions::mark_collected(
                    &resolved_task_id,
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
                Ok(ToolResult::error("wait_subagent: no sub-agent was found for that reference. It may have already finished and \
                     been collected, or the task_id is wrong.".to_string()))
            }
            Err(WaitError::NotOwned) => {
                log::debug!(
                    "[wait_subagent] outcome=not_owned task_id={}",
                    resolved_task_id
                );
                Ok(ToolResult::error(
                    "wait_subagent: that sub-agent was not started by this agent.".to_string(),
                ))
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
#[path = "wait_subagent_tests.rs"]
mod tests;
