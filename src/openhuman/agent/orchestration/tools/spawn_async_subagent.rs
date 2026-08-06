//! Tool: `spawn_async_subagent` - fire-and-forget sub-agent delegation.
//!
//! Unlike `spawn_subagent`, this tool returns as soon as the child run is
//! accepted. Completion/failure is reported through normal sub-agent lifecycle
//! events and, when possible, persisted in the child worker thread.

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::{current_parent, with_parent_context};
use crate::openhuman::agent::harness::run_queue::RunQueue;
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunOptions, SubagentRunStatus,
};
use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::agent::orchestration::running_subagents::{self, SubagentStatus};
use crate::openhuman::agent::orchestration::subagent_sessions::{
    self, DurableSubagentStatus, SubagentSessionSelector, SubagentSessionStore,
    SubagentSessionUpsert,
};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::memory::conversations::{self as conversations, ConversationMessage};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use tinyagents::harness::tool::ToolExecutionContext;

pub struct SpawnAsyncSubagentTool;

impl SpawnAsyncSubagentTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SpawnAsyncSubagentTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SpawnAsyncSubagentTool {
    fn name(&self) -> &str {
        "spawn_async_subagent"
    }

    fn description(&self) -> &str {
        "Fire-and-forget a specialised sub-agent for low-attention background work. \
         Use sparingly, only when the user does not need the result in the current \
         response, such as best-effort memory archiving, cleanup, or background \
         investigation. Do not use for user-visible answers, code changes, external \
         service writes, financial actions, or anything that may need clarification. \
         Never use it when the sub-agent's result must gate your final answer (e.g. \
         review/critique/verify/approve X BEFORE finalizing): this returns immediately \
         and the turn finalizes before the result lands. For those, run a synchronous \
         awaited sub-agent instead — a blocking delegate_* specialist or \
         spawn_parallel_agents (which collects results before returning)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let agent_ids: Vec<String> = AgentDefinitionRegistry::global()
            .map(|reg| reg.list().iter().map(|d| d.id.clone()).collect())
            .unwrap_or_default();

        let agent_id_schema = if agent_ids.is_empty() {
            json!({
                "type": "string",
                "description": "Sub-agent id (e.g. archivist, researcher, tools_agent)."
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
            "required": ["agent_id", "prompt"],
            "properties": {
                "agent_id": agent_id_schema,
                "prompt": {
                    "type": "string",
                    "description": "Clear, self-contained background instruction. Include all context needed. The sub-agent must not ask the user for clarification."
                },
                "context": {
                    "type": "string",
                    "description": "Optional context blob from prior task results. Rendered as a `[Context]` block before the prompt."
                },
                "model": {
                    "type": "string",
                    "description": "Optional exact model id for this background spawn only."
                },
                "toolkit": {
                    "type": "string",
                    "description": "Composio toolkit slug to scope this spawn to. Required when agent_id is `integrations_agent`."
                },
                "task_title": {
                    "type": "string",
                    "description": "Optional short title for the persisted background worker thread."
                },
                "task_key": {
                    "type": "string",
                    "description": "Optional deterministic identity key for reusable delegation. Defaults to a normalized task_title/prompt."
                },
                "fresh": {
                    "type": "boolean",
                    "description": "When true, bypass reusable subagent matching and create a fresh durable worker."
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
        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let context = args
            .get("context")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let model_override = args
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let toolkit_override = args
            .get("toolkit")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let task_title = args
            .get("task_title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Background subagent")
            .to_string();
        let task_key_source = durable_task_key_source(&args, &prompt, context.as_deref());
        let task_key = subagent_sessions::normalize_task_key(&task_key_source);
        let force_fresh = args.get("fresh").and_then(|v| v.as_bool()).unwrap_or(false);

        if agent_id.is_empty() {
            return Ok(ToolResult::error(
                "spawn_async_subagent: `agent_id` is required",
            ));
        }
        if prompt.is_empty() {
            return Ok(ToolResult::error(
                "spawn_async_subagent: `prompt` is required",
            ));
        }

        let parent = match current_parent() {
            Some(parent) => parent,
            None => {
                return Ok(ToolResult::error(
                    "spawn_async_subagent called outside of an agent turn",
                ));
            }
        };

        let registry = match AgentDefinitionRegistry::global() {
            Some(registry) => registry,
            None => {
                return Ok(ToolResult::error(
                    "spawn_async_subagent: AgentDefinitionRegistry has not been initialised",
                ));
            }
        };
        let definition = match registry.get(&agent_id).cloned() {
            Some(definition) => definition,
            None => {
                let available: Vec<&str> = registry.list().iter().map(|d| d.id.as_str()).collect();
                return Ok(ToolResult::error(format!(
                    "spawn_async_subagent: unknown agent_id '{agent_id}'. Available: {}",
                    available.join(", ")
                )));
            }
        };

        if !parent.allowed_subagent_ids.contains(&definition.id) {
            log::warn!(
                "[spawn_async_subagent] blocked subagent outside allowlist parent={} requested={} allowed={:?}",
                parent.agent_definition_id,
                definition.id,
                parent.allowed_subagent_ids
            );
            return Ok(ToolResult::error(format!(
                "spawn_async_subagent: agent '{}' is not in parent agent '{}' subagents.allowlist",
                definition.id, parent.agent_definition_id
            )));
        }

        if definition.id == "integrations_agent" && toolkit_override.is_none() {
            return Ok(ToolResult::error(
                "spawn_async_subagent(integrations_agent): the `toolkit` argument is required",
            ));
        }

        let parent_session = parent.session_id.clone();
        let progress_sink = parent.on_progress.clone();
        let parent_thread_id =
            crate::openhuman::agent::tinyagents::thread_context::current_thread_id();

        // Async delivery is thread-addressed: the finished result is inserted
        // back into the parent chat thread as a follow-up turn
        // (`background_delivery`). Outside a chat turn (flow `agent` nodes,
        // CLI, cron) there is no `current_thread_id()` to deliver into, so
        // `background_delivery::deliver_batch` logs "dropping headless batch"
        // and the (possibly real, completed) work is silently discarded — the
        // caller sees "Accepted" and never learns the result never arrived.
        // Fail loudly instead: the caller has a synchronous alternative
        // (`spawn_subagent` with `blocking: true`, or a `delegate_*` tool).
        // Both of those self-heal to blocking dispatch in this situation
        // rather than reaching this guard — see the `has_delivery_thread`
        // checks in `spawn_subagent.rs` and `dispatch.rs::dispatch_subagent`.
        // Only a *direct* `spawn_async_subagent` call lands here.
        if parent_thread_id.is_none() {
            log::warn!(
                "[spawn_async_subagent] refusing fire-and-forget spawn with no delivery thread \
                 parent={} requested={} — directing caller to synchronous delegation (flow node / \
                 CLI / cron context, background result would be discarded)",
                parent.agent_definition_id,
                definition.id
            );
            return Ok(ToolResult::error(
                "spawn_async_subagent: no parent chat thread available to deliver the result \
                 into (this looks like a flow node, CLI, or cron run rather than an interactive \
                 chat turn). Fire-and-forget delegation has nowhere to land its result here and \
                 the sub-agent's work would be silently discarded. Use synchronous delegation \
                 instead: call `spawn_subagent` with `blocking: true`, or use a `delegate_*` \
                 tool — both run the sub-agent inline and hand you its output in this turn. \
                 For parallel work, model it as parallel flow nodes rather than background \
                 sub-agents.",
            ));
        }
        let store = SubagentSessionStore::new(parent.workspace_dir.clone());
        let workspace_descriptor = tool_context.and_then(|ctx| ctx.workspace.clone());
        let effective_action_root = workspace_descriptor
            .as_ref()
            .map(|workspace| {
                tracing::debug!(
                    workspace_root = %workspace.root.display(),
                    policy_id = %workspace.policy_id,
                    "[spawn_async_subagent] using ToolExecutionContext workspace root"
                );
                workspace.root.clone()
            })
            .or_else(|| {
                crate::openhuman::security::live_policy::current()
                    .map(|policy| policy.action_dir.clone())
            });
        let selector = SubagentSessionSelector {
            parent_session: parent_session.clone(),
            parent_thread_id: parent_thread_id.clone(),
            agent_id: definition.id.clone(),
            toolkit: toolkit_override.clone(),
            model: model_override.clone(),
            sandbox_mode: format!("{:?}", definition.sandbox_mode),
            action_root: subagent_sessions::action_root_key(effective_action_root.as_deref()),
            task_key: task_key.clone(),
        };

        let reusable = if force_fresh {
            match subagent_sessions::find_reusable(&store, &selector) {
                Ok(Some(session)) => {
                    let _ = running_subagents::cancel_by_session_in_workspace(
                        &session.subagent_session_id,
                        &parent_session,
                        &parent.workspace_dir,
                    );
                    if let Err(err) = subagent_sessions::close(&store, &session.subagent_session_id)
                    {
                        log::warn!(
                            "[subagent_reuse] fresh close failed parent_thread_id={} subagent_session_id={} agent_id={} task_key={} error={}",
                            parent_thread_id.as_deref().unwrap_or("none"),
                            session.subagent_session_id,
                            definition.id,
                            task_key,
                            err
                        );
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    log::warn!(
                        "[subagent_reuse] fresh lookup failed parent_thread_id={} agent_id={} task_key={} error={}",
                        parent_thread_id.as_deref().unwrap_or("none"),
                        definition.id,
                        task_key,
                        err
                    );
                }
            }
            None
        } else {
            match subagent_sessions::find_reusable(&store, &selector) {
                Ok(session) => session,
                Err(err) => {
                    log::warn!(
                        "[subagent_reuse] lookup failed parent_thread_id={} agent_id={} task_key={} error={}",
                        parent_thread_id.as_deref().unwrap_or("none"),
                        definition.id,
                        task_key,
                        err
                    );
                    None
                }
            }
        };
        let reuse_decision = subagent_sessions::reuse_decision(reusable.as_ref(), force_fresh);
        let follow_up_prompt = reusable_follow_up_message(&prompt, context.as_deref());

        if let Some(session) = reusable.as_ref() {
            if session.status == DurableSubagentStatus::Running {
                if let Some(ref running_task_id) = session.current_task_id {
                    match running_subagents::steer(
                        running_task_id,
                        &parent_session,
                        follow_up_prompt.clone(),
                        crate::openhuman::agent::harness::run_queue::QueueMode::Steer,
                    )
                    .await
                    {
                        Ok(()) => {
                            log::info!(
                                "[subagent_reuse] parent_thread_id={} subagent_session_id={} task_id={} agent_id={} reuse_decision={}",
                                parent_thread_id.as_deref().unwrap_or("none"),
                                session.subagent_session_id,
                                running_task_id,
                                definition.id,
                                reuse_decision.as_str()
                            );
                            let payload = async_subagent_ref_payload(
                                running_task_id,
                                &session.subagent_session_id,
                                &definition.id,
                                session.worker_thread_id.as_deref(),
                                true,
                                reuse_decision.as_str(),
                                "running",
                            );
                            return Ok(ToolResult::success(format!(
                                "Continued reusable async sub-agent `{}`. It is already running and will pick up the new instruction at its next step. \
                                 Use the structured reference below to send more input, wait, or perform a short timeout tick.\n\n[async_subagent_ref]\n{}\n[/async_subagent_ref]",
                                payload["agent_id"].as_str().unwrap_or("subagent"),
                                serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
                            )));
                        }
                        Err(err) => {
                            log::warn!(
                                "[subagent_reuse] running steer failed parent_thread_id={} subagent_session_id={} task_id={} agent_id={} error={:?}",
                                parent_thread_id.as_deref().unwrap_or("none"),
                                session.subagent_session_id,
                                running_task_id,
                                definition.id,
                                err
                            );
                        }
                    }
                }
            }
        }

        let task_id = format!("sub-{}", uuid::Uuid::new_v4());
        let worker_thread_id = reusable
            .as_ref()
            .and_then(|session| session.worker_thread_id.clone())
            .or_else(|| {
                parent_thread_id.as_ref().and_then(|parent_thread_id| {
                    super::worker_thread::create_worker_thread(
                        parent.workspace_dir.clone(),
                        parent_thread_id,
                        &definition.id,
                        &task_title,
                        &prompt,
                    )
                    .ok()
                })
            });
        if let (Some(session), Some(worker_thread_id)) =
            (reusable.as_ref(), worker_thread_id.as_ref())
        {
            if session.worker_thread_id.as_deref() == Some(worker_thread_id.as_str()) {
                if let Err(err) = super::worker_thread::append_worker_user_message(
                    parent.workspace_dir.clone(),
                    worker_thread_id,
                    &definition.id,
                    &task_id,
                    &follow_up_prompt,
                ) {
                    log::warn!(
                        "[subagent_reuse] worker follow-up append failed parent_thread_id={} subagent_session_id={} worker_thread_id={} task_id={} error={}",
                        parent_thread_id.as_deref().unwrap_or("none"),
                        session.subagent_session_id,
                        worker_thread_id,
                        task_id,
                        err
                    );
                }
            }
        }
        let durable_session = match subagent_sessions::upsert_running(
            &store,
            SubagentSessionUpsert {
                selector,
                display_name: Some(definition.display_name().to_string()),
                task_title: task_title.clone(),
                worker_thread_id: worker_thread_id.clone(),
                task_id: task_id.clone(),
            },
            reusable.as_ref(),
        ) {
            Ok(session) => session,
            Err(err) => {
                log::warn!(
                    "[subagent_reuse] upsert failed parent_thread_id={} task_id={} agent_id={} reuse_decision={} error={}",
                    parent_thread_id.as_deref().unwrap_or("none"),
                    task_id,
                    definition.id,
                    reuse_decision.as_str(),
                    err
                );
                return Ok(ToolResult::error(format!(
                    "spawn_async_subagent: failed to persist reusable sub-agent session: {err}"
                )));
            }
        };

        let initial_history = reusable
            .as_ref()
            .and_then(|session| session.latest_history.clone())
            .map(|mut history| {
                history.push(ChatMessage::user(follow_up_prompt.clone()));
                history
            });

        log::info!(
            "[subagent_reuse] parent_thread_id={} subagent_session_id={} task_id={} agent_id={} reuse_decision={} task_key={}",
            parent_thread_id.as_deref().unwrap_or("none"),
            durable_session.subagent_session_id,
            task_id,
            definition.id,
            reuse_decision.as_str(),
            task_key
        );

        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_spawned(
            parent_session.clone(),
            definition.id.clone(),
            "async".to_string(),
            task_id.clone(),
            prompt.chars().count(),
        );
        if let Some(ref tx) = progress_sink {
            let _ = tx
                .send(AgentProgress::SubagentSpawned {
                    agent_id: definition.id.clone(),
                    task_id: task_id.clone(),
                    mode: "async".to_string(),
                    dedicated_thread: worker_thread_id.is_some(),
                    prompt_chars: prompt.chars().count(),
                    prompt: prompt.clone(),
                    worker_thread_id: worker_thread_id.clone(),
                    display_name: Some(definition.display_name().to_string()),
                })
                .await;
        }

        // Steering channel + status channel so the parent can `steer_subagent`
        // this run mid-flight and `wait_subagent` for its result. The engine
        // drains `steer_queue` at iteration boundaries; `status_tx` publishes
        // the terminal state to any waiter.
        let steer_queue = RunQueue::new();
        let task_queue = steer_queue.clone();
        let (status_tx, status_rx) = running_subagents::status_channel();

        let background_parent = parent.clone();
        let background_workspace_dir = parent.workspace_dir.clone();
        let background_definition = definition.clone();
        let background_agent_id = definition.id.clone();
        let background_task_id = task_id.clone();
        let background_parent_session = parent_session.clone();
        let background_progress = progress_sink.clone();
        let background_worker_thread_id = worker_thread_id.clone();
        let background_store = store.clone();
        let background_subagent_session_id = durable_session.subagent_session_id.clone();
        let background_workspace_descriptor = workspace_descriptor.clone();
        let background_worktree_action_dir = background_workspace_descriptor
            .as_ref()
            .map(|descriptor| descriptor.root.clone());
        let background_thread_affinity_id = background_worker_thread_id
            .clone()
            .unwrap_or_else(|| background_subagent_session_id.clone());
        let background_initial_history = initial_history;
        // Capture the parent chat thread NOW (the spawning turn's thread) so the
        // finished result can be delivered back into it as a system turn.
        let background_parent_thread_id = parent_thread_id.clone();
        // Kept on this side (the closure moves its own clone) so the registry
        // entry knows which parent thread owns this sub-agent — that's how
        // `cancel_for_thread` aborts it when the thread is deleted.
        let register_parent_thread_id = background_parent_thread_id.clone();
        // Lifecycle-critical wiring: log the parent-thread binding so the
        // thread-close cancellation path (`cancel_for_thread`) is grep-friendly.
        log::debug!(
            "[spawn_async_subagent] register task_id={} parent_session={} parent_thread_id={}",
            task_id,
            parent_session,
            register_parent_thread_id.as_deref().unwrap_or("none")
        );
        let background_prompt = add_background_contract(&prompt);

        let join = tokio::spawn(async move {
            let options = SubagentRunOptions {
                skill_filter_override: None,
                toolkit_override,
                context,
                model_override,
                task_id: Some(background_task_id.clone()),
                worker_thread_id: background_worker_thread_id.clone(),
                initial_history: background_initial_history,
                checkpoint_dir: None,
                worktree_action_dir: background_worktree_action_dir,
                workspace_descriptor: background_workspace_descriptor,
                run_queue: Some(task_queue),
            };

            let result = with_parent_context(background_parent, async move {
                crate::openhuman::agent::tinyagents::thread_context::with_thread_id(
                    background_thread_affinity_id,
                    async move {
                        run_subagent(&background_definition, &background_prompt, options).await
                    },
                )
                .await
            })
            .await;

            match result {
                Ok(outcome) => match outcome.status {
                    SubagentRunStatus::Completed => {
                        if let Err(err) = subagent_sessions::mark_finished(
                            &background_store,
                            &background_subagent_session_id,
                            &outcome.task_id,
                            &outcome.status,
                            outcome.final_history.clone(),
                        ) {
                            log::warn!(
                                "[subagent_reuse] mark_completed failed subagent_session_id={} task_id={} agent_id={} error={}",
                                background_subagent_session_id,
                                outcome.task_id,
                                outcome.agent_id,
                                err
                            );
                        }
                        let _ = status_tx.send(SubagentStatus::Completed {
                            output: outcome.output.clone(),
                            iterations: outcome.iterations,
                        });
                        // A workflow proposal produced inside the child's tool
                        // history is durable state, not prose: persist it into
                        // the parent chat thread (survives reload / reconnect —
                        // the old socket-only delivery could silently drop it)
                        // and carry the full payload in the delivery notice so
                        // the follow-up turn can present it faithfully.
                        let delivery_summary = attach_workflow_proposal(
                            &background_workspace_dir,
                            background_parent_thread_id.as_deref(),
                            &outcome.task_id,
                            &outcome.agent_id,
                            &outcome.final_history,
                            outcome.output.clone(),
                        );
                        // Queue the finished result for idle-gated, batched
                        // delivery back into the parent chat (the session
                        // runtime drains this when the session is next idle).
                        crate::openhuman::agent::orchestration::background_completions::record_completion(
                            background_parent_session.clone(),
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            delivery_summary,
                            background_parent_thread_id.clone(),
                        );
                        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                            background_parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.elapsed.as_millis() as u64,
                            outcome.output.chars().count(),
                            outcome.iterations,
                        );
                        if let Some(ref tx) = background_progress {
                            let _ = tx
                                .send(AgentProgress::SubagentCompleted {
                                    agent_id: outcome.agent_id,
                                    task_id: outcome.task_id,
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
                    }
                    SubagentRunStatus::Incomplete { ref reason } => {
                        // Async sub-agent stopped short (stuck halt / iteration
                        // cap). Mark the session finished and deliver the PARTIAL
                        // progress back to the parent, framed so it is not
                        // mistaken for a completed result (#4096).
                        if let Err(err) = subagent_sessions::mark_finished(
                            &background_store,
                            &background_subagent_session_id,
                            &outcome.task_id,
                            &outcome.status,
                            outcome.final_history.clone(),
                        ) {
                            log::warn!(
                                "[subagent_reuse] mark_incomplete failed subagent_session_id={} task_id={} agent_id={} error={}",
                                background_subagent_session_id,
                                outcome.task_id,
                                outcome.agent_id,
                                err
                            );
                        }
                        let framed = format!(
                            "[SUBAGENT_INCOMPLETE] the sub-agent {reason} and did not finish. \
                             Partial progress:\n{}",
                            outcome.output
                        );
                        let _ = status_tx.send(SubagentStatus::Completed {
                            output: framed.clone(),
                            iterations: outcome.iterations,
                        });
                        // An incomplete run may still have produced a full
                        // proposal before stalling — preserve it durably too.
                        let framed = attach_workflow_proposal(
                            &background_workspace_dir,
                            background_parent_thread_id.as_deref(),
                            &outcome.task_id,
                            &outcome.agent_id,
                            &outcome.final_history,
                            framed,
                        );
                        crate::openhuman::agent::orchestration::background_completions::record_completion(
                            background_parent_session.clone(),
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            framed,
                            background_parent_thread_id.clone(),
                        );
                        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                            background_parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.elapsed.as_millis() as u64,
                            outcome.output.chars().count(),
                            outcome.iterations,
                        );
                        if let Some(ref tx) = background_progress {
                            let _ = tx
                                .send(AgentProgress::SubagentCompleted {
                                    agent_id: outcome.agent_id,
                                    task_id: outcome.task_id,
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
                    }
                    SubagentRunStatus::AwaitingUser { ref question, .. } => {
                        if let Err(err) = subagent_sessions::mark_finished(
                            &background_store,
                            &background_subagent_session_id,
                            &outcome.task_id,
                            &outcome.status,
                            outcome.final_history.clone(),
                        ) {
                            log::warn!(
                                "[subagent_reuse] mark_awaiting_user failed subagent_session_id={} task_id={} agent_id={} error={}",
                                background_subagent_session_id,
                                outcome.task_id,
                                outcome.agent_id,
                                err
                            );
                        }
                        let _ = status_tx.send(SubagentStatus::AwaitingUser {
                            question: question.clone(),
                        });
                        let error = format!(
                            "async sub-agent requested user clarification and was not continued: {question}"
                        );
                        // #4896: a detached child that pauses for input won't
                        // continue on its own — queue a framed notice so the
                        // parent chat learns the delegated task needs input,
                        // instead of finalizing silently on "Accepted". Rides the
                        // same idle-gated background_delivery path as a success.
                        crate::openhuman::agent::orchestration::background_completions::record_awaiting_input(
                            background_parent_session.clone(),
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            question,
                            background_parent_thread_id.clone(),
                        );
                        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_failed(
                            background_parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            error.clone(),
                        );
                        if let Some(ref tx) = background_progress {
                            let _ = tx
                                .send(AgentProgress::SubagentFailed {
                                    agent_id: outcome.agent_id,
                                    task_id: outcome.task_id,
                                    error,
                                })
                                .await;
                        }
                    }
                },
                Err(err) => {
                    let error = err.to_string();
                    if let Err(store_err) = subagent_sessions::mark_failed(
                        &background_store,
                        &background_subagent_session_id,
                        &background_task_id,
                        error.clone(),
                    ) {
                        log::warn!(
                            "[subagent_reuse] mark_failed failed subagent_session_id={} task_id={} agent_id={} error={}",
                            background_subagent_session_id,
                            background_task_id,
                            background_agent_id,
                            store_err
                        );
                    }
                    let _ = status_tx.send(SubagentStatus::Failed {
                        error: error.clone(),
                    });
                    // #4896: a detached child that errors previously only
                    // published an event — nothing reached chat, so the parent
                    // turn finalized on "Accepted" and the failure was lost.
                    // Queue a framed failure notice so background_delivery
                    // surfaces it as a follow-up chat turn.
                    crate::openhuman::agent::orchestration::background_completions::record_failure(
                        background_parent_session.clone(),
                        background_task_id.clone(),
                        background_agent_id.clone(),
                        &error,
                        background_parent_thread_id.clone(),
                    );
                    crate::openhuman::agent::orchestration::subagent_events::publish_subagent_failed(
                        background_parent_session,
                        background_task_id.clone(),
                        background_agent_id.clone(),
                        error.clone(),
                    );
                    if let Some(ref tx) = background_progress {
                        let _ = tx
                            .send(AgentProgress::SubagentFailed {
                                agent_id: background_agent_id,
                                task_id: background_task_id,
                                error,
                            })
                            .await;
                    }
                }
            }
        });

        // Register *after* spawn so the AbortHandle is available. The task owns
        // `status_tx`; this side holds `status_rx` for `wait_subagent`.
        running_subagents::register(
            task_id.clone(),
            definition.id.clone(),
            parent_session.clone(),
            parent.session_parent_prefix.clone(),
            Some(durable_session.subagent_session_id.clone()),
            parent.workspace_dir.clone(),
            register_parent_thread_id,
            steer_queue,
            join.abort_handle(),
            status_rx,
        );

        let payload = async_subagent_ref_payload(
            &task_id,
            &durable_session.subagent_session_id,
            &definition.id,
            worker_thread_id.as_deref(),
            reusable.is_some(),
            reuse_decision.as_str(),
            "running",
        );
        let payload_json = match serde_json::to_string(&payload) {
            Ok(serialized) => {
                log::debug!(
                    "[spawn_async_subagent] serialized async reference payload bytes={}",
                    serialized.len()
                );
                serialized
            }
            Err(error) => {
                log::debug!(
                    "[spawn_async_subagent] failed to serialize async reference payload: {}",
                    error
                );
                "{}".to_string()
            }
        };
        log::debug!("[spawn_async_subagent] formatting accepted response");
        Ok(ToolResult::success(format_async_subagent_accepted(
            payload["agent_id"].as_str().unwrap_or("subagent"),
            &payload_json,
        )))
    }
}

/// Format the user-facing acceptance text around a structured async sub-agent reference.
fn format_async_subagent_accepted(agent_id: &str, payload_json: &str) -> String {
    format!(
        "Accepted async sub-agent `{agent_id}`. Use the structured reference below to send more input, \
         wait for completion, or perform a short timeout tick to check status. If the user does not need \
         the result now, continue without blocking.\n\n[async_subagent_ref]\n{payload_json}\n[/async_subagent_ref]"
    )
}

/// Build the machine-readable reference the orchestrator uses to steer, wait, or poll a worker.
fn async_subagent_ref_payload(
    task_id: &str,
    subagent_session_id: &str,
    agent_id: &str,
    worker_thread_id: Option<&str>,
    reused: bool,
    reuse_decision: &str,
    status: &str,
) -> serde_json::Value {
    json!({
        "task_id": task_id,
        "taskId": task_id,
        "subagent_session_id": subagent_session_id,
        "subagentSessionId": subagent_session_id,
        "agent_id": agent_id,
        "agentId": agent_id,
        "mode": "async",
        "status": status,
        "worker_thread_id": worker_thread_id,
        "workerThreadId": worker_thread_id,
        "reused": reused,
        "reuse_decision": reuse_decision,
        "reuseDecision": reuse_decision,
        "instructions": {
            "send_message": {
                "tool": "steer_subagent",
                "description": "Send additional instructions or context to this running async sub-agent.",
                "arguments": {
                    "subagent_session_id": subagent_session_id,
                    "message": "<message>",
                    "mode": "steer"
                }
            },
            "wait": {
                "tool": "wait_subagent",
                "description": "Block until the async sub-agent finishes, up to the timeout.",
                "arguments": {
                    "subagent_session_id": subagent_session_id,
                    "timeout_secs": 120
                }
            },
            "timeout_tick": {
                "tool": "wait_subagent",
                "description": "Perform a short status tick without committing the parent to a long wait.",
                "arguments": {
                    "subagent_session_id": subagent_session_id,
                    "timeout_secs": 1
                }
            },
            "delayed_tick": {
                "tool": "wait",
                "description": "Trigger a delayed callback before checking this async sub-agent again.",
                "arguments": {
                    "duration_secs": 30,
                    "message": format!("Check async sub-agent {agent_id} status with wait_subagent using subagent_session_id {subagent_session_id}.")
                }
            },
            "delayed_loop": {
                "tool": "wait_loop",
                "description": "Trigger repeatable delayed callbacks while this async sub-agent is still relevant.",
                "arguments": {
                    "duration_secs": 30,
                    "message": format!("Check async sub-agent {agent_id} status with wait_subagent using subagent_session_id {subagent_session_id}."),
                    "loop_key": subagent_session_id,
                    "iteration": 1
                }
            }
        },
        "next_actions": [
            "call steer_subagent to send more input",
            "call wait_subagent with timeout_secs to collect the result",
            "call wait_subagent with timeout_secs=1 as a timeout tick/status check",
            "call wait or wait_loop with the returned message to trigger a delayed status check",
            "continue without waiting when the current user reply does not depend on the result"
        ]
    })
}

fn add_background_contract(prompt: &str) -> String {
    format!(
        "[Background Contract]\n\
         Run this task without requiring attention from the parent or user. \
         Do not call ask_user_clarification. If required information is missing, \
         make the safest best-effort progress and record the limitation in your final output.\n\n\
         [Task]\n{prompt}"
    )
}

fn durable_task_key_source(
    args: &serde_json::Value,
    prompt: &str,
    context: Option<&str>,
) -> String {
    if let Some(task_key) = args
        .get("task_key")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return task_key.to_string();
    }

    match context.map(str::trim).filter(|s| !s.is_empty()) {
        Some(context) => format!("{prompt}\n\n[Context]\n{context}"),
        None => prompt.to_string(),
    }
}

/// Scan a finished child's history for the LAST `workflow_proposal` tool
/// result (the workflow_builder's `propose_workflow` / `revise_workflow` /
/// `edit_workflow` all return `{"type":"workflow_proposal", ...}` JSON).
/// Returns the parsed payload, or `None` when the run produced no proposal.
/// Lives here (not in `flows`) so the always-on orchestration path has no
/// dependency on the feature-gated flows domain — it is a generic scan for a
/// structured tool payload.
pub(crate) fn extract_workflow_proposal_from_history(
    history: &[ChatMessage],
) -> Option<serde_json::Value> {
    history
        .iter()
        .rev()
        .filter(|message| message.role == "tool")
        .find_map(|message| {
            let value: serde_json::Value = serde_json::from_str(message.content.trim()).ok()?;
            (value.get("type").and_then(|t| t.as_str()) == Some("workflow_proposal"))
                .then_some(value)
        })
}

/// Durably surface a workflow proposal found in a finished child's history:
/// persist it as a parent-thread conversation message (metadata carries the
/// full payload so the UI can rehydrate the proposal card after reload) and
/// append a `[workflow_proposal]` envelope to the delivery summary so the
/// follow-up turn presents it faithfully. Returns the (possibly extended)
/// summary; on any persistence error the summary still carries the envelope —
/// losing durability must not lose delivery.
fn attach_workflow_proposal(
    workspace_dir: &std::path::Path,
    parent_thread_id: Option<&str>,
    task_id: &str,
    agent_id: &str,
    final_history: &[ChatMessage],
    summary: String,
) -> String {
    let Some(proposal) = extract_workflow_proposal_from_history(final_history) else {
        return summary;
    };
    let proposal_json = match serde_json::to_string(&proposal) {
        Ok(json) => json,
        Err(err) => {
            log::warn!(
                "[spawn_async_subagent] workflow proposal re-serialize failed task_id={task_id} error={err}"
            );
            return summary;
        }
    };
    let name = proposal
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("Untitled workflow");
    log::info!(
        "[spawn_async_subagent] extracted workflow proposal '{name}' task_id={task_id} \
         ({} chars) — persisting to parent thread {:?}",
        proposal_json.len(),
        parent_thread_id
    );
    if let Some(thread_id) = parent_thread_id {
        let persisted = conversations::append_message(
            workspace_dir.to_path_buf(),
            thread_id,
            ConversationMessage {
                id: format!("workflow-proposal:{task_id}"),
                content: format!("Workflow proposal ready: {name}"),
                message_type: "text".to_string(),
                extra_metadata: json!({
                    "scope": "workflow_proposal",
                    "proposal": proposal,
                    "task_id": task_id,
                    "agent_id": agent_id,
                }),
                sender: "agent".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        );
        if let Err(err) = persisted {
            log::warn!(
                "[spawn_async_subagent] workflow proposal persistence failed \
                 thread_id={thread_id} task_id={task_id} error={err} — proposal still \
                 rides the delivery notice"
            );
        }
    }
    format!(
        "{summary}\n\n[workflow_proposal]\n{proposal_json}\n[/workflow_proposal]\n\
         (The full proposal above was also saved to the chat thread; present it to the \
         user for review — do not re-run the builder unless they ask for changes.)"
    )
}

fn reusable_follow_up_message(prompt: &str, context: Option<&str>) -> String {
    let mut message = String::from("[Follow-up instruction for reusable sub-agent]\n");
    if let Some(context) = context.map(str::trim).filter(|s| !s.is_empty()) {
        message.push_str("\n[Context]\n");
        message.push_str(context);
        message.push_str("\n\n");
    }
    message.push_str("[Task]\n");
    message.push_str(prompt);
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::context::prompt::ToolCallFormat;
    use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
    use crate::openhuman::agent::harness::fork_context::{
        with_parent_context, ParentExecutionContext,
    };
    use crate::openhuman::config::AgentConfig;
    use crate::openhuman::memory::{
        Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
    };
    use std::collections::HashSet;
    use std::path::Path;
    use std::sync::Arc;

    #[test]
    fn parameters_schema_advertises_fire_and_forget_fields() {
        let tool = SpawnAsyncSubagentTool::new();
        let schema = tool.parameters_schema();
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("required list");
        assert!(required.iter().any(|v| v.as_str() == Some("agent_id")));
        assert!(required.iter().any(|v| v.as_str() == Some("prompt")));

        let props = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("properties");
        for key in ["context", "model", "toolkit", "task_title"] {
            assert!(props.contains_key(key), "missing {key}");
        }
    }

    #[test]
    fn background_contract_forbids_user_attention() {
        let wrapped = add_background_contract("archive this fact");
        assert!(wrapped.contains("[Background Contract]"));
        assert!(wrapped.contains("Do not call ask_user_clarification"));
        assert!(wrapped.contains("[Task]\narchive this fact"));
    }

    #[test]
    fn accepted_message_hides_task_id_from_prose() {
        let payload = r#"{"task_id":"sub-internal-123","agent_id":"archivist","mode":"async"}"#;
        let message = format_async_subagent_accepted("archivist", payload);
        let prose = message
            .split("[async_subagent_ref]")
            .next()
            .expect("prose before structured reference");

        assert!(prose.contains("Accepted async sub-agent `archivist`"));
        assert!(!prose.contains("sub-internal-123"));
        assert!(message.contains("[async_subagent_ref]"));
        assert!(message.contains("sub-internal-123"));
    }

    #[test]
    fn async_reference_payload_includes_agent_id_and_control_instructions() {
        let payload = async_subagent_ref_payload(
            "sub-123",
            "subsess-456",
            "researcher",
            Some("thread-worker"),
            false,
            "created",
            "running",
        );

        assert_eq!(payload["agent_id"], "researcher");
        assert_eq!(payload["agentId"], "researcher");
        assert_eq!(payload["instructions"]["wait"]["tool"], "wait_subagent");
        assert_eq!(
            payload["instructions"]["timeout_tick"]["arguments"]["timeout_secs"],
            1
        );
        assert_eq!(payload["instructions"]["delayed_tick"]["tool"], "wait");
        assert_eq!(payload["instructions"]["delayed_loop"]["tool"], "wait_loop");
        assert_eq!(
            payload["instructions"]["send_message"]["tool"],
            "steer_subagent"
        );
    }

    #[test]
    fn durable_task_key_defaults_to_prompt_not_display_title() {
        let args = json!({
            "task_title": "Research",
            "prompt": "Research the async subagent cache behavior for example.com"
        });
        assert_eq!(
            durable_task_key_source(&args, args["prompt"].as_str().unwrap(), None),
            "Research the async subagent cache behavior for example.com"
        );
    }

    #[test]
    fn durable_task_key_includes_context_when_no_explicit_key() {
        let args = json!({
            "prompt": "Analyze this issue"
        });
        let source = durable_task_key_source(
            &args,
            args["prompt"].as_str().unwrap(),
            Some("issue body A"),
        );
        assert!(source.contains("Analyze this issue"));
        assert!(source.contains("[Context]\nissue body A"));
        assert_ne!(
            subagent_sessions::normalize_task_key(&source),
            subagent_sessions::normalize_task_key(&durable_task_key_source(
                &args,
                args["prompt"].as_str().unwrap(),
                Some("issue body B")
            ))
        );
    }

    #[test]
    fn durable_task_key_uses_explicit_task_key_when_present() {
        let args = json!({
            "task_key": "audit:example.com",
            "task_title": "Research",
            "prompt": "Research the async subagent cache behavior for example.com"
        });
        assert_eq!(
            durable_task_key_source(&args, args["prompt"].as_str().unwrap(), Some("ignored")),
            "audit:example.com"
        );
    }

    #[test]
    fn reusable_follow_up_message_preserves_context() {
        let rendered = reusable_follow_up_message("Continue the audit", Some("prior result: 42"));
        assert!(rendered.contains("[Context]\nprior result: 42"));
        assert!(rendered.contains("[Task]\nContinue the audit"));
    }

    #[test]
    fn extract_workflow_proposal_finds_last_proposal_tool_result() {
        let history = vec![
            ChatMessage::user("build me a workflow"),
            ChatMessage::tool(r#"{"type":"something_else","x":1}"#),
            ChatMessage::tool(
                r#"{"type":"workflow_proposal","persisted":false,"name":"Old Draft"}"#,
            ),
            ChatMessage::assistant("revising…"),
            ChatMessage::tool(
                r#"{"type":"workflow_proposal","persisted":false,"name":"Daily X Trending Email"}"#,
            ),
            ChatMessage::assistant("Here's the proposed workflow."),
        ];
        let proposal =
            extract_workflow_proposal_from_history(&history).expect("proposal extracted");
        // The LAST proposal wins — later revisions supersede earlier drafts.
        assert_eq!(proposal["name"], "Daily X Trending Email");
    }

    #[test]
    fn extract_workflow_proposal_ignores_non_proposal_history() {
        let history = vec![
            ChatMessage::user("hello"),
            ChatMessage::tool("plain text tool output, not json"),
            ChatMessage::assistant("done"),
        ];
        assert!(extract_workflow_proposal_from_history(&history).is_none());
    }

    #[test]
    fn attach_workflow_proposal_persists_thread_message_and_extends_summary() {
        use crate::openhuman::memory::conversations::CreateConversationThread;
        let temp = tempfile::tempdir().expect("tempdir");
        conversations::ensure_thread(
            temp.path().to_path_buf(),
            CreateConversationThread {
                id: "thread-parent".into(),
                title: "Main chat".into(),
                created_at: chrono::Utc::now().to_rfc3339(),
                parent_thread_id: None,
                labels: None,
                personality_id: None,
            },
        )
        .expect("thread created");

        let history = vec![ChatMessage::tool(
            r#"{"type":"workflow_proposal","persisted":false,"name":"Daily X Trending Email","graph":{"nodes":[],"edges":[]}}"#,
        )];
        let summary = attach_workflow_proposal(
            temp.path(),
            Some("thread-parent"),
            "sub-task-1",
            "workflow_builder",
            &history,
            "Here's the proposed workflow.".to_string(),
        );

        // Delivery notice carries the machine-readable envelope.
        assert!(summary.starts_with("Here's the proposed workflow."));
        assert!(summary.contains("[workflow_proposal]"));
        assert!(summary.contains("\"name\":\"Daily X Trending Email\""));
        assert!(summary.contains("[/workflow_proposal]"));

        // Proposal is durably persisted in the parent thread with rehydratable
        // metadata (this is what survives reload / a dropped socket event).
        let messages = conversations::get_messages(temp.path().to_path_buf(), "thread-parent")
            .expect("messages");
        let proposal_msg = messages
            .iter()
            .find(|m| m.id == "workflow-proposal:sub-task-1")
            .expect("proposal message persisted");
        assert_eq!(proposal_msg.sender, "agent");
        assert!(proposal_msg.content.contains("Daily X Trending Email"));
        assert_eq!(proposal_msg.extra_metadata["scope"], "workflow_proposal");
        assert_eq!(
            proposal_msg.extra_metadata["proposal"]["name"],
            "Daily X Trending Email"
        );
        assert_eq!(proposal_msg.extra_metadata["task_id"], "sub-task-1");
    }

    #[test]
    fn attach_workflow_proposal_without_proposal_returns_summary_unchanged() {
        let temp = tempfile::tempdir().expect("tempdir");
        let summary = attach_workflow_proposal(
            temp.path(),
            Some("thread-x"),
            "sub-task-2",
            "researcher",
            &[ChatMessage::tool("no proposal here")],
            "research done".to_string(),
        );
        assert_eq!(summary, "research done");
    }

    #[tokio::test]
    async fn missing_agent_id_returns_error() {
        let tool = SpawnAsyncSubagentTool::new();
        let result = tool.execute(json!({ "prompt": "do work" })).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("agent_id"));
    }

    #[tokio::test]
    async fn missing_prompt_returns_error() {
        let tool = SpawnAsyncSubagentTool::new();
        let result = tool
            .execute(json!({ "agent_id": "archivist" }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("prompt"));
    }

    /// B40 / Gap 4: a delegating agent (orchestrator/subconscious) calling
    /// `spawn_async_subagent` directly from a thread-less context (flow
    /// `agent` node, CLI, cron) must get a clear, actionable error instead of
    /// silently accepting the spawn and later dropping its result in
    /// `background_delivery`'s "headless batch" path. Sets up a real parent
    /// turn context (so the call gets past the `current_parent()` /
    /// allowlist / registry checks) but deliberately does NOT wrap the call
    /// in `with_thread_id`, so `current_thread_id()` is None — the exact
    /// condition that used to sail through to `tokio::spawn` and lose the
    /// result.
    #[tokio::test]
    async fn errors_clearly_when_no_parent_thread_for_delivery() {
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let workspace = tempfile::TempDir::new().expect("workspace");

        let result = with_parent_context(parent_context(workspace.path()), async {
            SpawnAsyncSubagentTool::new()
                .execute(json!({
                    "agent_id": "researcher",
                    "prompt": "investigate x",
                }))
                .await
        })
        .await
        .unwrap();

        assert!(result.is_error);
        let out = result.output();
        assert!(out.contains("no parent chat thread"), "{out}");
        // The recommended escape hatch must name `blocking: true` — plain
        // `spawn_subagent` defaults to async and would otherwise be steered
        // straight back into this same guard.
        assert!(out.contains("spawn_subagent"), "{out}");
        assert!(out.contains("blocking: true"), "{out}");
        assert!(out.contains("delegate_"), "{out}");
    }

    /// The positive half of the branch above: with a chat thread bound, the
    /// guard must NOT fire. This asserts only that the call gets *past* the
    /// `parent_thread_id.is_none()` check — driving the full spawn/session
    /// machinery to a successful "Accepted" is out of scope for a unit test,
    /// so a later failure is acceptable; a "no parent chat thread" failure is
    /// not. Pins that the guard keys on thread presence and nothing else.
    #[tokio::test]
    async fn guard_does_not_fire_when_parent_thread_is_bound() {
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let workspace = tempfile::TempDir::new().expect("workspace");

        let result = with_parent_context(parent_context(workspace.path()), async {
            crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-parent", async {
                SpawnAsyncSubagentTool::new()
                    .execute(json!({
                        "agent_id": "researcher",
                        "prompt": "investigate x",
                    }))
                    .await
            })
            .await
        })
        .await
        .unwrap();

        assert!(
            !result.output().contains("no parent chat thread"),
            "guard fired despite a bound parent thread: {}",
            result.output()
        );
    }

    fn parent_context(workspace_dir: &Path) -> ParentExecutionContext {
        ParentExecutionContext {
            workspace_descriptor: None,
            agent_definition_id: "orchestrator".into(),
            allowed_subagent_ids: HashSet::from(["researcher".to_string()]),
            turn_model_source: crate::openhuman::agent::tinyagents::TurnModelSource::from_model(
                Arc::new(tinyagents::harness::testkit::ScriptedModel::replies(vec![
                    "done",
                ])),
            ),
            all_tools: Arc::new(Vec::new()),
            all_tool_specs: Arc::new(Vec::new()),
            visible_tool_names: std::collections::HashSet::new(),
            subagent_tool_ceiling_names: std::collections::HashSet::new(),
            model_name: "test-model".into(),
            temperature: 0.0,
            workspace_dir: workspace_dir.to_path_buf(),
            memory: Arc::new(NoopMemory),
            agent_config: AgentConfig::default(),
            workflows: Arc::new(Vec::new()),
            memory_context: Arc::new(None),
            session_id: "parent-session".into(),
            channel: "test".into(),
            connected_integrations: Vec::new(),
            tool_call_format: ToolCallFormat::Native,
            session_key: "parent-key".into(),
            session_parent_prefix: None,
            on_progress: None,
            run_queue: None,
        }
    }

    struct NoopMemory;

    #[async_trait::async_trait]
    impl Memory for NoopMemory {
        fn name(&self) -> &str {
            "noop"
        }

        async fn store(
            &self,
            _namespace: &str,
            _key: &str,
            _content: &str,
            _category: MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _opts: RecallOpts<'_>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _namespace: Option<&str>,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
            Ok(Vec::new())
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }

        async fn health_check(&self) -> bool {
            true
        }
    }
}
