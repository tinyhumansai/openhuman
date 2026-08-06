//! Tool: `continue_subagent` — resume a paused sub-agent with a follow-up
//! message (typically the user's answer to a clarification question).
//!
//! When a sub-agent calls `ask_user_clarification`, its harness loop exits
//! early, checkpoints the full conversation history, and returns an
//! `AwaitingUser` envelope to the parent. The orchestrator surfaces the
//! question to the user, and when the user answers, calls this tool to
//! resume the same sub-agent from its checkpoint with the user's response
//! appended to the conversation history.

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentCheckpointData, SubagentRunOptions, SubagentRunStatus,
};
use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use tinyagents::harness::tool::ToolExecutionContext;

pub struct ContinueSubagentTool;

impl Default for ContinueSubagentTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ContinueSubagentTool {
    pub fn new() -> Self {
        Self
    }

    /// Resume a durable sub-agent session (no pause checkpoint on disk).
    ///
    /// Accepts either the transient `task_id` (`sub-…`) or the durable
    /// `subagent_session_id` in the tool's `task_id` argument — the ambient
    /// `[active_subagents]` roster advertises both. The resume routes through
    /// `spawn_async_subagent` pinned to the session's `task_key`, so the
    /// reusable-session machinery reattaches: a `running` worker is steered,
    /// an `idle` one is relaunched seeded with its persisted `latest_history`
    /// plus the new message, and the finished result is delivered back into
    /// the parent chat as a new turn.
    async fn resume_from_durable_store(
        &self,
        parent: &crate::openhuman::agent::harness::fork_context::ParentExecutionContext,
        task_id: &str,
        agent_id: &str,
        message: &str,
        tool_context: Option<&ToolExecutionContext>,
    ) -> anyhow::Result<ToolResult> {
        use crate::openhuman::agent::orchestration::subagent_sessions::{
            self, SubagentSessionStore,
        };

        let store = SubagentSessionStore::new(parent.workspace_dir.clone());
        let sessions = match subagent_sessions::list_for_parent(&store, &parent.session_id, None) {
            Ok(sessions) => sessions,
            Err(err) => {
                return Ok(ToolResult::error(format!(
                    "continue_subagent: no checkpoint found for task_id '{task_id}' and \
                         the durable session store could not be read: {err}"
                )));
            }
        };
        let session = sessions.iter().find(|s| {
            s.subagent_session_id == task_id || s.current_task_id.as_deref() == Some(task_id)
        });
        let Some(session) = session else {
            return Ok(ToolResult::error(format!(
                "continue_subagent: no checkpoint and no durable session found for \
                 task_id '{task_id}'. Check the [active_subagents] roster (or call \
                 list_subagents) and pass that subagent_session_id — or delegate a \
                 fresh task if the worker was closed."
            )));
        };
        if session.agent_id != agent_id {
            return Ok(ToolResult::error(format!(
                "continue_subagent: agent_id mismatch — durable session '{}' belongs to \
                 '{}', caller passed '{agent_id}'",
                session.subagent_session_id, session.agent_id
            )));
        }
        if !session.status.reusable() {
            return Ok(ToolResult::error(format!(
                "continue_subagent: durable session '{}' is {:?} and cannot be resumed. \
                 Delegate a fresh task instead.",
                session.subagent_session_id, session.status
            )));
        }

        tracing::info!(
            task_id = %task_id,
            subagent_session_id = %session.subagent_session_id,
            agent_id = %agent_id,
            status = ?session.status,
            "[continue_subagent] resuming durable session via reusable async path"
        );

        let mut async_args = json!({
            "agent_id": session.agent_id.clone(),
            "prompt": message,
            "task_key": session.task_key.clone(),
            "task_title": session.task_title.clone(),
        });
        if let (Some(obj), Some(toolkit)) = (async_args.as_object_mut(), &session.toolkit) {
            obj.insert(
                "toolkit".to_string(),
                serde_json::Value::String(toolkit.clone()),
            );
        }
        if let (Some(obj), Some(model)) = (async_args.as_object_mut(), &session.model) {
            obj.insert(
                "model".to_string(),
                serde_json::Value::String(model.clone()),
            );
        }
        super::spawn_async_subagent::SpawnAsyncSubagentTool::new()
            .execute_with_context(async_args, ToolCallOptions::default(), tool_context)
            .await
    }
}

#[async_trait]
impl Tool for ContinueSubagentTool {
    fn name(&self) -> &str {
        "continue_subagent"
    }

    fn description(&self) -> &str {
        "Resume an existing sub-agent with a follow-up message, keeping its \
         full prior context. Two cases: (1) a sub-agent paused on \
         ask_user_clarification — pass the task_id from the \
         [SUBAGENT_AWAITING_USER] envelope and the user's answer; (2) a \
         durable worker from the [active_subagents] roster (e.g. an `idle` \
         workflow_builder whose proposal the user is now reacting to) — pass \
         its subagent_session_id (or task id) and the follow-up. Always \
         prefer this over re-delegating the same task from scratch: a fresh \
         delegation loses everything the worker already did."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["task_id", "agent_id", "message"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task_id from the [SUBAGENT_AWAITING_USER] envelope, or the subagent_session_id (preferred) / task id of a durable worker from the [active_subagents] roster."
                },
                "agent_id": {
                    "type": "string",
                    "description": "The worker's agent_id (from the envelope or the roster line)."
                },
                "message": {
                    "type": "string",
                    "description": "The user's answer or follow-up message to send to the paused sub-agent."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        tool_context: Option<&ToolExecutionContext>,
    ) -> anyhow::Result<ToolResult> {
        let task_id = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let agent_id = args
            .get("agent_id")
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

        if task_id.is_empty() {
            return Ok(ToolResult::error(
                "continue_subagent: `task_id` is required",
            ));
        }
        if agent_id.is_empty() {
            return Ok(ToolResult::error(
                "continue_subagent: `agent_id` is required",
            ));
        }
        if message.is_empty() {
            return Ok(ToolResult::error(
                "continue_subagent: `message` is required (the user's answer)",
            ));
        }

        let parent = match current_parent() {
            Some(p) => p,
            None => {
                return Ok(ToolResult::error(
                    "continue_subagent: no parent execution context available",
                ));
            }
        };

        // Load checkpoint
        let checkpoint_dir = parent.workspace_dir.join(".openhuman/subagent_checkpoints");
        let checkpoint_path = checkpoint_dir.join(format!("{task_id}.json"));

        let checkpoint_json = match std::fs::read_to_string(&checkpoint_path) {
            Ok(json) => json,
            Err(e) => {
                tracing::info!(
                    task_id = %task_id,
                    path = %checkpoint_path.display(),
                    error = %e,
                    "[continue_subagent] no pause checkpoint — trying durable session store"
                );
                // Durable-session fallback: the sub-agent did not pause on a
                // clarification (no checkpoint), but it may be a durable
                // worker from this or an earlier turn — resumable with its
                // full prior history via the reusable async path. This is
                // what lets the orchestrator continue a finished
                // workflow_builder ("looks good, save it") instead of
                // re-delegating a fresh, stateless one.
                return self
                    .resume_from_durable_store(&parent, &task_id, &agent_id, &message, tool_context)
                    .await;
            }
        };

        let checkpoint: SubagentCheckpointData = match serde_json::from_str(&checkpoint_json) {
            Ok(cp) => cp,
            Err(e) => {
                tracing::error!(
                    task_id = %task_id,
                    error = %e,
                    "[continue_subagent] failed to deserialize checkpoint"
                );
                return Ok(ToolResult::error(format!(
                    "continue_subagent: corrupted checkpoint for task_id '{task_id}': {e}"
                )));
            }
        };

        if checkpoint.agent_id != agent_id {
            return Ok(ToolResult::error(format!(
                "continue_subagent: agent_id mismatch — checkpoint has '{}', \
                 caller passed '{agent_id}'",
                checkpoint.agent_id
            )));
        }

        // Look up the agent definition
        let registry = match AgentDefinitionRegistry::global() {
            Some(reg) => reg,
            None => {
                return Ok(ToolResult::error(
                    "continue_subagent: AgentDefinitionRegistry not initialised",
                ));
            }
        };
        let definition = match registry.get(&agent_id) {
            Some(def) => def,
            None => {
                return Ok(ToolResult::error(format!(
                    "continue_subagent: unknown agent_id '{agent_id}'"
                )));
            }
        };

        // Reconstruct history and append the user's answer
        let mut history = checkpoint.history;
        history.push(ChatMessage::user(format!(
            "[User's answer to your clarification question]\n{message}"
        )));

        tracing::info!(
            task_id = %task_id,
            agent_id = %agent_id,
            history_len = history.len(),
            message_chars = message.chars().count(),
            "[continue_subagent] resuming sub-agent with user's answer"
        );

        let parent_session = parent.session_id.clone();
        let progress_sink = parent.on_progress.clone();

        // Publish resumed event (reuse SubagentSpawned with a note)
        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_spawned(
            parent_session.clone(),
            agent_id.clone(),
            "typed".to_string(),
            task_id.clone(),
            message.chars().count(),
        );

        if let Some(ref tx) = progress_sink {
            let _ = tx
                .send(AgentProgress::SubagentSpawned {
                    agent_id: agent_id.clone(),
                    task_id: task_id.clone(),
                    mode: "typed".to_string(),
                    dedicated_thread: false,
                    prompt_chars: message.chars().count(),
                    prompt: message.to_string(),
                    worker_thread_id: checkpoint.worker_thread_id.clone(),
                    display_name: definition.display_name.clone(),
                })
                .await;
        }

        // Build options with initial_history for replay
        let workspace_descriptor = tool_context.and_then(|ctx| ctx.workspace.clone());
        let worktree_action_dir = workspace_descriptor
            .as_ref()
            .map(|descriptor| descriptor.root.clone());
        if let Some(descriptor) = workspace_descriptor.as_ref() {
            tracing::debug!(
                task_id = %task_id,
                agent_id = %agent_id,
                workspace_root = %descriptor.root.display(),
                policy_id = %descriptor.policy_id,
                "[continue_subagent] using ToolExecutionContext workspace root"
            );
        }
        let options = SubagentRunOptions {
            skill_filter_override: checkpoint.skill_filter_override,
            toolkit_override: checkpoint.toolkit_override,
            context: None,
            model_override: checkpoint.model_override,
            task_id: Some(task_id.clone()),
            worker_thread_id: checkpoint.worker_thread_id.clone(),
            initial_history: Some(history),
            checkpoint_dir: Some(checkpoint_dir.clone()),
            worktree_action_dir,
            workspace_descriptor,
            run_queue: None,
        };

        // Run the sub-agent from its checkpoint
        match run_subagent(definition, "", options).await {
            Ok(outcome) => {
                match &outcome.status {
                    SubagentRunStatus::AwaitingUser {
                        question,
                        options: _,
                    } => {
                        // Another round of clarification
                        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_awaiting_user(
                            parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            question.clone(),
                        );
                        if let Some(ref tx) = progress_sink {
                            let _ = tx
                                .send(AgentProgress::SubagentAwaitingUser {
                                    agent_id: outcome.agent_id.clone(),
                                    task_id: outcome.task_id.clone(),
                                    question: question.clone(),
                                    worker_thread_id: checkpoint.worker_thread_id.clone(),
                                })
                                .await;
                        }
                        let wt_display = checkpoint.worker_thread_id.as_deref().unwrap_or("(none)");
                        let envelope = format!(
                            "[SUBAGENT_AWAITING_USER]\n\
                             task_id: {}\n\
                             agent_id: {}\n\
                             worker_thread_id: {}\n\
                             question: {}\n\
                             [/SUBAGENT_AWAITING_USER]\n\n\
                             The sub-agent needs further clarification. \
                             Surface the above question to the user. When the user responds, \
                             call continue_subagent again with the same task_id, agent_id, \
                             and the user's new answer.",
                            outcome.task_id, outcome.agent_id, wt_display, question,
                        );
                        Ok(ToolResult::success(envelope))
                    }
                    SubagentRunStatus::Completed => {
                        // Clean up checkpoint file on successful completion
                        if let Err(e) = std::fs::remove_file(&checkpoint_path) {
                            tracing::debug!(
                                task_id = %task_id,
                                error = %e,
                                "[continue_subagent] failed to remove checkpoint (best-effort)"
                            );
                        } else {
                            tracing::info!(
                                task_id = %task_id,
                                "[continue_subagent] checkpoint cleaned up after completion"
                            );
                        }

                        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                            parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.elapsed.as_millis() as u64,
                            outcome.output.chars().count(),
                            outcome.iterations,
                        );
                        if let Some(ref tx) = progress_sink {
                            let _ = tx
                                .send(AgentProgress::SubagentCompleted {
                                    agent_id: outcome.agent_id.clone(),
                                    task_id: outcome.task_id.clone(),
                                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                                    iterations: outcome.iterations as u32,
                                    output_chars: outcome.output.chars().count(),
                                    output: outcome.output.clone(),
                                    worktree_path: None,
                                    changed_files: Vec::new(),
                                    dirty_status: None,
                                })
                                .await;
                        }
                        Ok(ToolResult::success(outcome.output))
                    }
                    SubagentRunStatus::Incomplete { reason } => {
                        // The continued sub-agent stopped short again (stuck halt
                        // / iteration cap). Hand back the partial progress framed
                        // as incomplete rather than a clean success (#4096).
                        // The run is no longer awaiting input, so the checkpoint
                        // written for the prior AwaitingUser pause is stale —
                        // clean it up best-effort, mirroring the Completed arm.
                        if let Err(e) = std::fs::remove_file(&checkpoint_path) {
                            tracing::debug!(
                                task_id = %task_id,
                                error = %e,
                                "[continue_subagent] failed to remove checkpoint after incomplete (best-effort)"
                            );
                        }
                        tracing::info!(
                            agent_id = %outcome.agent_id,
                            task_id = %outcome.task_id,
                            reason = %reason,
                            "[continue_subagent] sub-agent stopped incomplete after continue"
                        );
                        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                            parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.elapsed.as_millis() as u64,
                            outcome.output.chars().count(),
                            outcome.iterations,
                        );
                        if let Some(ref tx) = progress_sink {
                            let _ = tx
                                .send(AgentProgress::SubagentCompleted {
                                    agent_id: outcome.agent_id.clone(),
                                    task_id: outcome.task_id.clone(),
                                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                                    iterations: outcome.iterations as u32,
                                    output_chars: outcome.output.chars().count(),
                                    output: outcome.output.clone(),
                                    worktree_path: None,
                                    changed_files: Vec::new(),
                                    dirty_status: None,
                                })
                                .await;
                        }
                        Ok(ToolResult::success(format!(
                            "[SUBAGENT_INCOMPLETE]\n\
                             task_id: {}\n\
                             agent_id: {}\n\
                             reason: the sub-agent {reason}\n\
                             progress:\n{}\n\
                             [/SUBAGENT_INCOMPLETE]\n\n\
                             The sub-agent did NOT finish. Above is its partial progress. Do NOT \
                             report this as done; relay the partial result and the blocker to the \
                             user, or take a different approach.",
                            outcome.task_id, outcome.agent_id, outcome.output,
                        )))
                    }
                }
            }
            Err(err) => {
                let message = err.to_string();
                tracing::error!(
                    task_id = %task_id,
                    agent_id = %agent_id,
                    "[continue_subagent] sub-agent execution failed"
                );
                crate::openhuman::agent::orchestration::subagent_events::publish_subagent_failed(
                    parent_session,
                    task_id.clone(),
                    agent_id.clone(),
                    message.clone(),
                );
                if let Some(ref tx) = progress_sink {
                    let _ = tx
                        .send(AgentProgress::SubagentFailed {
                            agent_id: agent_id.clone(),
                            task_id: task_id.clone(),
                            error: message.clone(),
                        })
                        .await;
                }
                Ok(ToolResult::error(format!(
                    "continue_subagent failed: {message}"
                )))
            }
        }
    }
}
