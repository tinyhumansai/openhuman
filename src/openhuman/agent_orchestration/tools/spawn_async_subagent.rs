//! Tool: `spawn_async_subagent` - fire-and-forget sub-agent delegation.
//!
//! Unlike `spawn_subagent`, this tool returns as soon as the child run is
//! accepted. Completion/failure is reported through normal sub-agent lifecycle
//! events and, when possible, persisted in the child worker thread.

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::{current_parent, with_parent_context};
use crate::openhuman::agent::harness::run_queue::RunQueue;
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunOptions, SubagentRunStatus,
};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent_orchestration::running_subagents::{self, SubagentStatus};
use crate::openhuman::agent_orchestration::subagent_sessions::{
    self, DurableSubagentStatus, SubagentSessionSelector, SubagentSessionStore,
    SubagentSessionUpsert,
};
use crate::openhuman::inference::provider::ChatMessage;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

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
         service writes, financial actions, or anything that may need clarification."
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
            crate::openhuman::inference::provider::thread_context::current_thread_id();
        let store = SubagentSessionStore::new(parent.workspace_dir.clone());
        let effective_action_root = crate::openhuman::agent::harness::current_action_dir_override()
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
                    let _ = running_subagents::cancel_by_session(
                        &session.subagent_session_id,
                        &parent_session,
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

        publish_global(DomainEvent::SubagentSpawned {
            parent_session: parent_session.clone(),
            agent_id: definition.id.clone(),
            mode: "async".to_string(),
            task_id: task_id.clone(),
            prompt_chars: prompt.chars().count(),
        });
        if let Some(ref tx) = progress_sink {
            let _ = tx
                .send(AgentProgress::SubagentSpawned {
                    agent_id: definition.id.clone(),
                    task_id: task_id.clone(),
                    mode: "async".to_string(),
                    dedicated_thread: worker_thread_id.is_some(),
                    prompt_chars: prompt.chars().count(),
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
        let background_definition = definition.clone();
        let background_agent_id = definition.id.clone();
        let background_task_id = task_id.clone();
        let background_parent_session = parent_session.clone();
        let background_progress = progress_sink.clone();
        let background_worker_thread_id = worker_thread_id.clone();
        let background_store = store.clone();
        let background_subagent_session_id = durable_session.subagent_session_id.clone();
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
                worktree_action_dir: None,
                run_queue: Some(task_queue),
            };

            let result = with_parent_context(background_parent, async move {
                crate::openhuman::inference::provider::thread_context::with_thread_id(
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
                        // Queue the finished result for idle-gated, batched
                        // delivery back into the parent chat (the session
                        // runtime drains this when the session is next idle).
                        crate::openhuman::agent_orchestration::background_completions::record_completion(
                            background_parent_session.clone(),
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.output.clone(),
                            background_parent_thread_id.clone(),
                        );
                        publish_global(DomainEvent::SubagentCompleted {
                            parent_session: background_parent_session,
                            task_id: outcome.task_id.clone(),
                            agent_id: outcome.agent_id.clone(),
                            elapsed_ms: outcome.elapsed.as_millis() as u64,
                            output_chars: outcome.output.chars().count(),
                            iterations: outcome.iterations,
                        });
                        if let Some(ref tx) = background_progress {
                            let _ = tx
                                .send(AgentProgress::SubagentCompleted {
                                    agent_id: outcome.agent_id,
                                    task_id: outcome.task_id,
                                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                                    iterations: outcome.iterations as u32,
                                    output_chars: outcome.output.chars().count(),
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
                        publish_global(DomainEvent::SubagentFailed {
                            parent_session: background_parent_session,
                            task_id: outcome.task_id.clone(),
                            agent_id: outcome.agent_id.clone(),
                            error: error.clone(),
                        });
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
                    publish_global(DomainEvent::SubagentFailed {
                        parent_session: background_parent_session,
                        task_id: background_task_id.clone(),
                        agent_id: background_agent_id.clone(),
                        error: error.clone(),
                    });
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
}
