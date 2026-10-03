//! Tool: `spawn_worker_thread` — spawn a dedicated worker thread for a complex delegated task.
//!
//! Unlike `spawn_subagent`, which collapses sub-agent work into a single
//! tool result in the current thread, `spawn_worker_thread` creates a new
//! persisted thread with label `worker`. The sub-agent's full transcript
//! is recorded into that thread, and the parent receives a compact
//! reference (worker thread id) instead of the full output.
//!
//! Worker threads carry a hard cap on depth: a worker thread cannot spawn
//! another worker thread.

use crate::agent::harness::definition::AgentDefinitionRegistry;
use crate::agent::subagent_host::{run_subagent_with_parent, SubagentRunOptions};
use crate::threads::store as conversations;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::tool::{ToolDispatch, ToolExecutionContext};
use tinytools::ToolRunContext;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult};

/// Spawns a sub-agent in a dedicated worker thread.
pub struct SpawnWorkerThreadTool;

pub(crate) struct SpawnWorkerThreadDispatch {
    tool: Arc<dyn Tool>,
}

impl SpawnWorkerThreadDispatch {
    pub(crate) fn new(tool: Arc<dyn Tool>) -> Self {
        Self { tool }
    }
}

#[async_trait]
impl ToolDispatch<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for SpawnWorkerThreadDispatch
{
    fn tool(&self) -> Arc<dyn Tool> {
        self.tool.clone()
    }

    async fn execute(
        &self,
        _state: &(),
        _call_id: tinyagents_harness::CallId,
        arguments: serde_json::Value,
        _options: ToolCallOptions,
        parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let context = ToolExecutionContext::from_run_context(parent, _call_id.clone());
        SpawnWorkerThreadTool::new()
            .execute_with_live_parent_context(
                arguments,
                Some(&context),
                parent.data.child(),
                Some(parent),
            )
            .await
    }
}

impl Default for SpawnWorkerThreadTool {
    fn default() -> Self {
        Self::new()
    }
}

impl SpawnWorkerThreadTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SpawnWorkerThreadTool {
    fn name(&self) -> &str {
        "spawn_worker_thread"
    }

    fn description(&self) -> &str {
        "Spawn a dedicated worker thread for a complex delegated task. \
         Use this when the task is long or involves many steps that would \
         clutter the current conversation. The sub-agent runs in a fresh \
         thread labeled 'worker', and you receive the thread ID and a \
         summary. Worker threads cannot spawn other worker threads."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let agent_ids: Vec<String> = AgentDefinitionRegistry::global()
            .map(|reg| reg.list().iter().map(|d| d.id.clone()).collect())
            .unwrap_or_default();

        let agent_id_schema = if agent_ids.is_empty() {
            json!({
                "type": "string",
                "description": "Sub-agent id (e.g. code_executor, critic, planner)."
            })
        } else {
            json!({
                "type": "string",
                "enum": agent_ids,
                "description": "Sub-agent id from the registry."
            })
        };

        json!({
            "type": "object",
            "required": ["agent_id", "prompt", "task_title"],
            "properties": {
                "agent_id": agent_id_schema,
                "prompt": {
                    "type": "string",
                    "description": "Clear, specific instruction for the sub-agent. The sub-agent has no memory of the parent's conversation, so include all context the sub-agent needs to act."
                },
                "task_title": {
                    "type": "string",
                    "description": "A short, descriptive title for the worker thread (e.g. 'Researching Rust async patterns')."
                },
                "context": {
                    "type": "string",
                    "description": "Optional context blob from prior task results. Rendered as a `[Context]` block before the prompt."
                },
                "model": {
                    "type": "string",
                    "description": "Optional exact model id for this spawn only. Keeps the parent provider/routing, but pins the worker child agent to this model instead of the agent definition's default."
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
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        if let Some(live_parent) = super::ambient_parent_run_context("direct-spawn-worker") {
            let run_context = live_parent.data.child();
            return self
                .execute_with_live_parent_context(
                    args,
                    tool_context,
                    run_context,
                    Some(&live_parent),
                )
                .await;
        }
        self.execute_with_parent_context(
            args,
            tool_context,
            crate::agent::tinyagents::host::OpenHumanRunContext::new(),
        )
        .await
    }
}

impl SpawnWorkerThreadTool {
    pub(crate) async fn execute_with_parent_context(
        &self,
        args: serde_json::Value,
        tool_context: Option<&dyn ToolRunContext>,
        run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    ) -> anyhow::Result<ToolResult> {
        self.execute_with_live_parent_context(args, tool_context, run_context, None)
            .await
    }

    /// Typed-harness entrypoint that retains the live parent for this inline
    /// child. The public `Tool` fallback has no live harness parent.
    pub(crate) async fn execute_with_live_parent_context(
        &self,
        args: serde_json::Value,
        tool_context: Option<&dyn ToolRunContext>,
        run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
        live_parent: Option<&RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>>,
    ) -> anyhow::Result<ToolResult> {
        let Some(live_parent) = live_parent else {
            return Ok(ToolResult::error(
                "spawn_worker_thread requires a live harness run context.",
            ));
        };
        let started = std::time::Instant::now();

        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let task_title = args
            .get("task_title")
            .and_then(|v| v.as_str())
            .unwrap_or("Worker Task")
            .to_string();
        let context = args
            .get("context")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let model_override = args
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if agent_id.is_empty() || prompt.is_empty() {
            tracing::warn!(
                agent_id = %agent_id,
                prompt_empty = prompt.is_empty(),
                "[spawn_worker_thread] rejected: agent_id and prompt are required"
            );
            return Ok(ToolResult::error("agent_id and prompt are required"));
        }

        let parent = run_context
            .parent
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no parent context"))?;
        let parent_session = parent.session_id.clone();

        // ── Depth Guard ────────────────────────────────────────────────
        // Check if the current thread is already a worker thread.
        let current_thread_id = tool_context
            .and_then(ToolRunContext::thread_id)
            .unwrap_or("unknown")
            .to_string();

        tracing::info!(
            agent_id = %agent_id,
            task_title = %task_title,
            current_thread_id = %current_thread_id,
            has_context = context.is_some(),
            "[spawn_worker_thread] invoked"
        );

        let threads = conversations::list_threads(parent.workspace_dir.clone())
            .map_err(|e| anyhow::anyhow!(e))?;
        if let Some(current_thread) = threads.iter().find(|t| t.id == current_thread_id) {
            let is_delegated_label = current_thread
                .labels
                .iter()
                .any(|label| label == "tasks" || label == "worker" || label == "agent-task");
            if is_delegated_label || current_thread.parent_thread_id.is_some() {
                tracing::warn!(
                    agent_id = %agent_id,
                    current_thread_id = %current_thread_id,
                    is_delegated_label,
                    has_parent_thread_id = current_thread.parent_thread_id.is_some(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "[spawn_worker_thread] depth guard blocked spawn from worker thread"
                );
                return Ok(ToolResult::error(
                    "Worker threads cannot spawn other worker threads. Depth is capped at 1. Use spawn_subagent for inline delegation instead.",
                ));
            }
        }

        let registry = AgentDefinitionRegistry::global()
            .ok_or_else(|| anyhow::anyhow!("AgentDefinitionRegistry not initialised"))?;

        let definition = registry
            .get(&agent_id)
            .ok_or_else(|| anyhow::anyhow!("agent_id '{}' not found", agent_id))?;

        if !parent.allowed_subagent_ids.contains(&definition.id) {
            tracing::warn!(
                parent_agent = %parent.agent_definition_id,
                requested_agent = %definition.id,
                allowed = ?parent.allowed_subagent_ids,
                "[spawn_worker_thread] blocked subagent outside parent allowlist"
            );
            return Ok(ToolResult::error(format!(
                "spawn_worker_thread: agent '{}' is not in parent agent '{}' subagents.allowlist",
                definition.id, parent.agent_definition_id
            )));
        }

        tracing::debug!(
            parent_agent = %parent.agent_definition_id,
            requested_agent = %definition.id,
            "[spawn_worker_thread] subagent allowlist check passed"
        );

        // ── Create Worker Thread ───────────────────────────────────────
        // Shared with `spawn_subagent` so both delegation paths persist an
        // identical, reopenable sub-thread seeded with the prompt.
        let worker_thread_id = super::worker_thread::create_worker_thread(
            parent.workspace_dir.clone(),
            &current_thread_id,
            &agent_id,
            &task_title,
            &prompt,
        )
        .map_err(|e| anyhow::anyhow!(e))?;

        // We don't have an easy way to append a system message to the parent
        // thread here without triggering a re-render of the history the model
        // sees. Instead, we return the info in the tool result.

        // ── Run Subagent ──────────────────────────────────────────────
        let workspace_descriptor = tool_context.and_then(|ctx| ctx.workspace().cloned());
        let worktree_action_dir = workspace_descriptor
            .as_ref()
            .map(|descriptor| descriptor.root.clone());
        if let Some(descriptor) = workspace_descriptor.as_ref() {
            tracing::debug!(
                agent_id = %agent_id,
                worker_thread_id = %worker_thread_id,
                workspace_root = %descriptor.root.display(),
                policy_id = %descriptor.policy_id,
                "[spawn_worker_thread] using ToolExecutionContext workspace root"
            );
        }
        let progress_sink = run_context.progress.clone();
        let options = SubagentRunOptions {
            skill_filter_override: None,
            context,
            model_override,
            task_id: None,
            thread_id: tool_context
                .and_then(ToolRunContext::thread_id)
                .map(str::to_owned),
            run_context,
            worker_thread_id: Some(worker_thread_id.clone()),
            initial_history: None,
            checkpoint_dir: None,
            worktree_action_dir,
            workspace_descriptor,
            run_queue: None,
        };

        tracing::debug!(
            agent_id = %agent_id,
            worker_thread_id = %worker_thread_id,
            "[spawn_worker_thread] dispatching run_subagent"
        );

        let run =
            run_subagent_with_parent(live_parent, definition.clone(), prompt.clone(), options)
                .await;
        match run {
            Ok(outcome) => {
                let owns_effects = outcome.should_emit_lifecycle_effects();
                match outcome.status {
                    crate::agent::subagent_host::SubagentRunStatus::Completed => {
                        if owns_effects {
                            crate::agent::orchestration::subagent_events::publish_subagent_completed(
                                parent_session,
                                outcome.task_id.clone(),
                                outcome.agent_id.clone(),
                                outcome.elapsed.as_millis() as u64,
                                outcome.output.chars().count(),
                                outcome.iterations,
                            );
                        }
                        tracing::info!(
                            agent_id = %agent_id,
                            worker_thread_id = %worker_thread_id,
                            task_id = %outcome.task_id,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "[spawn_worker_thread] completed successfully"
                        );
                        Ok(ToolResult::success(format!(
                            "Spawned worker thread `{worker_thread_id}` for the task: {task_title}. \
                             The sub-agent has completed its work. You can find the full transcript \
                             in the worker thread.\n\n\
                             [worker_thread_ref]\n{}\n[/worker_thread_ref]",
                            json!({
                                "thread_id": worker_thread_id,
                                "label": "worker",
                                "agent_id": agent_id,
                                "task_id": outcome.task_id,
                                "status": "completed"
                            })
                        )))
                    }
                    crate::agent::subagent_host::SubagentRunStatus::AwaitingUser {
                        question,
                        checkpoint,
                        ..
                    } => {
                        if owns_effects {
                            crate::agent::orchestration::subagent_events::publish_subagent_awaiting_user(
                                parent_session,
                                outcome.task_id.clone(),
                                outcome.agent_id.clone(),
                                question.clone(),
                            );
                            if let Some(progress) = progress_sink {
                                let _ = progress
                                    .send(crate::agent::progress::AgentProgress::SubagentAwaitingUser {
                                        agent_id: outcome.agent_id.clone(),
                                        task_id: outcome.task_id.clone(),
                                        question: question.clone(),
                                        worker_thread_id: Some(worker_thread_id.clone()),
                                        checkpoint_path: checkpoint
                                            .as_ref()
                                            .map(|path| path.to_string_lossy().to_string()),
                                    })
                                    .await;
                            }
                        }
                        Ok(ToolResult::success(format!(
                            "Worker thread `{worker_thread_id}` is awaiting user input: {question}\n\n\
                             [worker_thread_ref]\n{}\n[/worker_thread_ref]",
                            json!({
                                "thread_id": worker_thread_id,
                                "label": "worker",
                                "agent_id": agent_id,
                                "task_id": outcome.task_id,
                                "status": "awaiting_user",
                                "question": question,
                            })
                        )))
                    }
                    crate::agent::subagent_host::SubagentRunStatus::Incomplete { reason } => {
                        if owns_effects {
                            crate::agent::orchestration::subagent_events::publish_subagent_completed(
                                parent_session,
                                outcome.task_id.clone(),
                                outcome.agent_id.clone(),
                                outcome.elapsed.as_millis() as u64,
                                outcome.output.chars().count(),
                                outcome.iterations,
                            );
                        }
                        Ok(ToolResult::error(format!(
                            "Worker thread `{worker_thread_id}` stopped before finishing: {reason}"
                        )))
                    }
                    crate::agent::subagent_host::SubagentRunStatus::Cancelled => {
                        // Cancellation is not a completed lifecycle. Surface it
                        // as failed/cancelled and deliberately do not publish a
                        // completed event or progress record.
                        if owns_effects {
                            crate::agent::orchestration::subagent_events::publish_subagent_failed(
                                parent_session,
                                outcome.task_id.clone(),
                                outcome.agent_id.clone(),
                                "worker sub-agent was cancelled".into(),
                            );
                            if let Some(progress) = progress_sink {
                                let _ = progress
                                    .send(crate::agent::progress::AgentProgress::SubagentFailed {
                                        agent_id: outcome.agent_id.clone(),
                                        task_id: outcome.task_id.clone(),
                                        error: "worker sub-agent was cancelled".into(),
                                    })
                                    .await;
                            }
                        }
                        Ok(ToolResult::error(format!(
                            "Worker thread `{worker_thread_id}` was cancelled"
                        )))
                    }
                }
            }
            Err(err) => {
                tracing::error!(
                    agent_id = %agent_id,
                    worker_thread_id = %worker_thread_id,
                    error = %err,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "[spawn_worker_thread] execution failed"
                );
                Ok(ToolResult::error(format!(
                    "Worker thread execution failed: {err}"
                )))
            }
        }
    }
}

#[cfg(test)]
#[path = "spawn_worker_thread_tests.rs"]
mod tests;
