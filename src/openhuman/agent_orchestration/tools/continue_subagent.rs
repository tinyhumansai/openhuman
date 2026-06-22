//! Tool: `continue_subagent` — resume a paused sub-agent with a follow-up
//! message (typically the user's answer to a clarification question).
//!
//! When a sub-agent calls `ask_user_clarification`, its harness loop exits
//! early, checkpoints the full conversation history, and returns an
//! `AwaitingUser` envelope to the parent. The orchestrator surfaces the
//! question to the user, and when the user answers, calls this tool to
//! resume the same sub-agent from its checkpoint with the user's response
//! appended to the conversation history.

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentCheckpointData, SubagentRunOptions, SubagentRunStatus,
};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::ChatMessage;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

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
}

#[async_trait]
impl Tool for ContinueSubagentTool {
    fn name(&self) -> &str {
        "continue_subagent"
    }

    fn description(&self) -> &str {
        "Resume a paused sub-agent that requested user input via \
         ask_user_clarification. Pass the task_id from the \
         [SUBAGENT_AWAITING_USER] envelope and the user's answer \
         as the message. The sub-agent continues from its checkpoint \
         with full prior context."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["task_id", "agent_id", "message"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task_id from the [SUBAGENT_AWAITING_USER] envelope."
                },
                "agent_id": {
                    "type": "string",
                    "description": "The agent_id from the [SUBAGENT_AWAITING_USER] envelope."
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
                tracing::warn!(
                    task_id = %task_id,
                    path = %checkpoint_path.display(),
                    error = %e,
                    "[continue_subagent] checkpoint not found"
                );
                return Ok(ToolResult::error(format!(
                    "continue_subagent: no checkpoint found for task_id '{task_id}'. \
                     The sub-agent may not have paused, or the checkpoint expired."
                )));
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
        publish_global(DomainEvent::SubagentSpawned {
            parent_session: parent_session.clone(),
            agent_id: agent_id.clone(),
            mode: "typed".to_string(),
            task_id: task_id.clone(),
            prompt_chars: message.chars().count(),
        });

        if let Some(ref tx) = progress_sink {
            let _ = tx
                .send(AgentProgress::SubagentSpawned {
                    agent_id: agent_id.clone(),
                    task_id: task_id.clone(),
                    mode: "typed".to_string(),
                    dedicated_thread: false,
                    prompt_chars: message.chars().count(),
                    worker_thread_id: checkpoint.worker_thread_id.clone(),
                    display_name: definition.display_name.clone(),
                })
                .await;
        }

        // Build options with initial_history for replay
        let options = SubagentRunOptions {
            skill_filter_override: checkpoint.skill_filter_override,
            toolkit_override: checkpoint.toolkit_override,
            context: None,
            model_override: checkpoint.model_override,
            task_id: Some(task_id.clone()),
            worker_thread_id: checkpoint.worker_thread_id.clone(),
            initial_history: Some(history),
            checkpoint_dir: Some(checkpoint_dir.clone()),
            worktree_action_dir: None,
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
                        publish_global(DomainEvent::SubagentAwaitingUser {
                            parent_session,
                            task_id: outcome.task_id.clone(),
                            agent_id: outcome.agent_id.clone(),
                            question: question.clone(),
                        });
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

                        publish_global(DomainEvent::SubagentCompleted {
                            parent_session,
                            task_id: outcome.task_id.clone(),
                            agent_id: outcome.agent_id.clone(),
                            elapsed_ms: outcome.elapsed.as_millis() as u64,
                            output_chars: outcome.output.chars().count(),
                            iterations: outcome.iterations,
                        });
                        if let Some(ref tx) = progress_sink {
                            let _ = tx
                                .send(AgentProgress::SubagentCompleted {
                                    agent_id: outcome.agent_id.clone(),
                                    task_id: outcome.task_id.clone(),
                                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                                    iterations: outcome.iterations as u32,
                                    output_chars: outcome.output.chars().count(),
                                    worktree_path: None,
                                    changed_files: Vec::new(),
                                    dirty_status: None,
                                })
                                .await;
                        }
                        Ok(ToolResult::success(outcome.output))
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
                publish_global(DomainEvent::SubagentFailed {
                    parent_session,
                    task_id: task_id.clone(),
                    agent_id: agent_id.clone(),
                    error: message.clone(),
                });
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
