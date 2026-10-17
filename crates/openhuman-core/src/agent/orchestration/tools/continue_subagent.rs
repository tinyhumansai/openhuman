//! Tool: `continue_subagent` — resume a paused sub-agent with a follow-up
//! message (typically the user's answer to a clarification question).
//!
//! When a sub-agent calls `ask_user_clarification`, its harness loop exits
//! early, checkpoints the full conversation history, and returns an
//! `AwaitingUser` envelope to the parent. The orchestrator surfaces the
//! question to the user, and when the user answers, calls this tool to
//! resume the same sub-agent from its checkpoint with the user's response
//! appended to the conversation history.

use crate::agent::harness::host_definition::AgentDefinitionRegistry;
use crate::agent::orchestration::tools::dispatch::{incomplete_stop, stopped_subagent_result};
use crate::agent::progress::AgentProgress;
use crate::agent::subagent_host::{
    continue_subagent, continue_subagent_with_parent, load_subagent_checkpoint, SubagentRunOptions,
    SubagentRunStatus,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::tool::{ToolDispatch, ToolExecutionContext};
use tinytools::ToolRunContext;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult};

pub struct ContinueSubagentTool;

pub(crate) struct ContinueSubagentDispatch {
    tool: Arc<dyn Tool>,
}

impl ContinueSubagentDispatch {
    pub(crate) fn new(tool: Arc<dyn Tool>) -> Self {
        Self { tool }
    }
}

#[async_trait]
impl ToolDispatch<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for ContinueSubagentDispatch
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
        ContinueSubagentTool::new()
            .execute_with_live_parent_context(
                arguments,
                Some(&context),
                parent.data.child(),
                Some(parent),
            )
            .await
    }
}

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
        parent: &crate::agent::harness::fork_context::ParentExecutionContext,
        task_id: &str,
        agent_id: &str,
        message: &str,
        tool_context: Option<&dyn ToolRunContext>,
        run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
        live_parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        use crate::agent::orchestration::subagent_sessions::{self, SubagentSessionStore};

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
        // The session id already identifies a worker within this parent's
        // store. Accept it in `agent_id` too when a caller copies the roster's
        // session id into both fields, but keep rejecting unrelated agents.
        if session.agent_id != agent_id && session.subagent_session_id != agent_id {
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
            agent_id = %session.agent_id,
            status = ?session.status,
            "[continue_subagent] resuming durable session via reusable async path"
        );

        let mut async_args = json!({
            "agent_id": session.agent_id.clone(),
            "prompt": message,
            "task_key": session.task_key.clone(),
            "task_title": session.task_title.clone(),
        });
        if let (Some(obj), Some(model)) = (async_args.as_object_mut(), &session.model) {
            obj.insert(
                "model".to_string(),
                serde_json::Value::String(model.clone()),
            );
        }
        let detached_data = live_parent.data.detached_child();
        let detached_cancellation = detached_data.cancellation.clone();
        let detached_parent = live_parent
            .child(
                tinyagents_harness::context::RunConfig::new(format!(
                    "async-subagent-{}",
                    uuid::Uuid::new_v4()
                )),
                detached_data,
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
            .with_cancellation(detached_cancellation);
        super::spawn_async_subagent::SpawnAsyncSubagentTool::new()
            .execute_with_live_parent_context(
                async_args,
                tool_context,
                run_context,
                detached_parent,
            )
            .await
    }
}

#[async_trait]
impl Tool for ContinueSubagentTool {
    fn name(&self) -> &str {
        "continue_subagent"
    }

    fn description(&self) -> &str {
        "Resume an existing sub-agent with a follow-up, keeping its context: pass the `task_id` \
         from a `[SUBAGENT_AWAITING_USER]` envelope or a `subagent_session_id` from the roster. \
         Always prefer this to re-delegating."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["task_id", "agent_id", "message"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "task_id from the envelope, or the worker's subagent_session_id from the roster."
                },
                "agent_id": {
                    "type": "string",
                    "description": "The worker's agent_id (from the envelope or roster). For a durable session, its subagent_session_id is also accepted here when used as task_id."
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
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.execute_with_parent_context(
            args,
            tool_context,
            crate::agent::tinyagents::host::OpenHumanRunContext::new().with_core(
                crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(tool_context)
                    .into_core(),
            ),
        )
        .await
    }
}

impl ContinueSubagentTool {
    pub(crate) async fn execute_with_parent_context(
        &self,
        args: serde_json::Value,
        tool_context: Option<&dyn ToolRunContext>,
        run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    ) -> anyhow::Result<ToolResult> {
        self.execute_with_live_parent_context(args, tool_context, run_context, None)
            .await
    }

    pub(crate) async fn execute_with_live_parent_context(
        &self,
        args: serde_json::Value,
        tool_context: Option<&dyn ToolRunContext>,
        run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
        live_parent: Option<
            &tinyagents_harness::context::RunContext<
                crate::agent::tinyagents::host::OpenHumanRunContext,
            >,
        >,
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
        // `task_id` is model-authored and is about to be joined into a
        // filesystem path twice: once to read the pause checkpoint below, and
        // again — via `SubagentRunOptions::task_id` — to write one if the
        // resumed child pauses a second time. `../../../../tmp/pwn` would walk
        // both clean out of the checkpoint directory, so it is rejected at the
        // boundary rather than only at the sink.
        if !crate::agent::subagent_host::is_safe_task_id(&task_id) {
            return Ok(ToolResult::error(format!(
                "continue_subagent: `task_id` must be a plain identifier \
                 (letters, digits, `-`, `_`); got '{task_id}'"
            )));
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

        let parent = match run_context.parent.clone() {
            Some(p) => p,
            None => {
                return Ok(ToolResult::error(
                    "continue_subagent: no parent execution context available",
                ));
            }
        };

        // Load checkpoint
        let checkpoint_dir = parent.workspace_dir.join(".openhuman/subagent_checkpoints");
        let checkpoint = match load_subagent_checkpoint(&checkpoint_dir, &task_id) {
            Ok(checkpoint) => checkpoint,
            Err(error) => {
                // A missing file is the *expected* case, not a failure: a
                // `subsess-…` id from the `[active_subagents]` roster is a
                // durable-session id, which never has a pause checkpoint (those
                // are keyed by task id and only written when a child exits on
                // `ask_user_clarification`). Logging it at INFO with
                // `error=No such file or directory` made the designed happy
                // path read as a broken one — which is what #5928 was filed on.
                // Any other IO error here really is one, so it stays loud.
                tracing::debug!(task_id = %task_id, error = %error, "[continue_subagent] no scoped pause checkpoint; resolving durable session");
                // Durable-session fallback: the sub-agent did not pause on a
                // clarification (no checkpoint), but it may be a durable
                // worker from this or an earlier turn — resumable with its
                // full prior history via the reusable async path. This is
                // what lets the orchestrator continue a finished
                // workflow_builder ("looks good, save it") instead of
                // re-delegating a fresh, stateless one.
                return self
                    .resume_from_durable_store(
                        &parent,
                        &task_id,
                        &agent_id,
                        &message,
                        tool_context,
                        run_context,
                        live_parent.ok_or_else(|| {
                            anyhow::anyhow!("continue_subagent requires a live harness run context")
                        })?,
                    )
                    .await;
            }
        };
        let original_key = checkpoint.task_key.clone().ok_or_else(|| {
            anyhow::anyhow!("continue_subagent: checkpoint omitted original task key")
        })?;

        if checkpoint.agent_id != agent_id {
            return Ok(ToolResult::error(format!(
                "continue_subagent: agent_id mismatch — checkpoint has '{}', \
                 caller passed '{agent_id}'",
                checkpoint.agent_id
            )));
        }

        // Look up the agent definition
        let registry = match AgentDefinitionRegistry::current() {
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

        tracing::info!(
            task_id = %task_id,
            agent_id = %agent_id,
            message_chars = message.chars().count(),
            "[continue_subagent] resuming sub-agent with a durable user answer"
        );

        let parent_session = parent.session_id.clone();
        let progress_sink = parent.on_progress.clone();

        // Publish resumed event (reuse SubagentSpawned with a note)
        crate::agent::orchestration::subagent_events::publish_subagent_spawned(
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
                    parent_call_id: crate::tools::host_extensions::tool_call_id(tool_context),
                })
                .await;
        }

        // The neutral lifecycle reloads the durable history and the host
        // planner appends `message` exactly once. Do not preload a second
        // history here: it would be overwritten by resume loading or duplicate
        // the user answer on an already-resumed pause.
        let workspace_descriptor = tool_context
            .and_then(|ctx| ctx.workspace().cloned())
            .or_else(|| run_context.workspace.clone());
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
            context: None,
            model_override: checkpoint.model_override,
            task_id: Some(task_id.clone()),
            thread_id: tool_context
                .and_then(ToolRunContext::thread_id)
                .or(run_context.thread_id.as_deref())
                .map(str::to_owned),
            run_context,
            worker_thread_id: checkpoint.worker_thread_id.clone(),
            initial_history: None,
            checkpoint_dir: Some(checkpoint_dir.clone()),
            worktree_action_dir,
            workspace_descriptor,
            run_queue: None,
        };

        // Run the sub-agent from its checkpoint
        let continuation = if let Some(parent) = live_parent {
            continue_subagent_with_parent(
                parent,
                original_key,
                definition.clone(),
                message.clone(),
                options,
            )
            .await
        } else {
            continue_subagent(original_key, definition, &message, options).await
        };
        match continuation {
            Ok(outcome) => {
                let emit_lifecycle_effects = outcome.should_emit_lifecycle_effects();
                match &outcome.status {
                    SubagentRunStatus::AwaitingUser {
                        question,
                        options: _,
                        checkpoint: pause_checkpoint,
                    } => {
                        // Another round of clarification
                        if emit_lifecycle_effects {
                            crate::agent::orchestration::subagent_events::publish_subagent_awaiting_user(
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
                                        checkpoint_path: pause_checkpoint
                                            .as_ref()
                                            .map(|p| p.to_string_lossy().to_string()),
                                    })
                                    .await;
                            }
                        }
                        // Built by the shared helper, not hand-rolled here.
                        // This path used to `format!` its own copy with the
                        // question interpolated RAW — the exact hole
                        // `awaiting_user_envelope` exists to close: a
                        // sub-agent-authored question containing a newline and
                        // a literal `[/SUBAGENT_AWAITING_USER]` closed the
                        // block early and injected resume instructions the
                        // orchestrator then trusted. The helper JSON-encodes
                        // it. The hand-rolled copy also omitted the "do NOT
                        // re-spawn" instruction that the #4291 fix added.
                        let envelope = super::awaiting_user::awaiting_user_envelope(
                            &outcome.task_id,
                            &outcome.agent_id,
                            checkpoint.worker_thread_id.as_deref(),
                            question,
                            pause_checkpoint.is_some(),
                        );
                        Ok(ToolResult::success(envelope))
                    }
                    SubagentRunStatus::Completed => {
                        // The lifecycle persistence boundary atomically marks
                        // terminal state and retires the scoped pause.

                        if emit_lifecycle_effects {
                            crate::agent::orchestration::subagent_events::publish_subagent_completed(
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
                                        // Not audited for whether this child's spend reached the
                                        // parent turn's ledger, so it stays silent: omission adds
                                        // nothing, which is the status quo. See the field's docs.
                                        usage: None,
                                        worktree_path: None,
                                        changed_files: Vec::new(),
                                        dirty_status: None,
                                        stop: None,
                                    })
                                    .await;
                            }
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
                        tracing::info!(
                            agent_id = %outcome.agent_id,
                            task_id = %outcome.task_id,
                            reason = %reason,
                            "[continue_subagent] sub-agent stopped incomplete after continue"
                        );
                        if emit_lifecycle_effects {
                            crate::agent::orchestration::subagent_events::publish_subagent_completed(
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
                                        // Not audited for whether this child's spend reached the
                                        // parent turn's ledger, so it stays silent: omission adds
                                        // nothing, which is the status quo. See the field's docs.
                                        usage: None,
                                        worktree_path: None,
                                        changed_files: Vec::new(),
                                        dirty_status: None,
                                        stop: incomplete_stop(reason),
                                    })
                                    .await;
                            }
                        }
                        let envelope = format!(
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
                        );
                        Ok(stopped_subagent_result(
                            "continue_subagent",
                            &outcome.agent_id,
                            &outcome.task_id,
                            reason,
                            envelope,
                        ))
                    }
                    SubagentRunStatus::Cancelled => {
                        tracing::info!(
                            agent_id = %outcome.agent_id,
                            task_id = %outcome.task_id,
                            "[continue_subagent] sub-agent cancelled"
                        );
                        if emit_lifecycle_effects {
                            let message = "sub-agent was cancelled".to_string();
                            crate::agent::orchestration::subagent_events::publish_subagent_failed(
                                parent_session,
                                outcome.task_id.clone(),
                                outcome.agent_id.clone(),
                                message.clone(),
                            );
                            if let Some(ref tx) = progress_sink {
                                let _ = tx
                                    .send(AgentProgress::SubagentFailed {
                                        agent_id: outcome.agent_id.clone(),
                                        task_id: outcome.task_id.clone(),
                                        error: message,
                                    })
                                    .await;
                            }
                        }
                        Ok(ToolResult::error(
                            "continue_subagent: sub-agent was cancelled",
                        ))
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
                crate::agent::orchestration::subagent_events::publish_subagent_failed(
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
