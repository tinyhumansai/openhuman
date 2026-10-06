//! Subagent dispatch logic shared by all agent delegation tools.

use crate::agent::harness::definition::AgentDefinitionRegistry;
use crate::agent::progress::AgentProgress;
use crate::agent::subagent_host::{
    run_subagent_with_parent, SubagentRunOptions, SubagentRunStatus,
};
use async_trait::async_trait;
use std::sync::Arc;
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::tool::{ToolDispatch, ToolExecutionContext};
use tinytools::ToolRunContext;
use tinytools::{ToolCallOptions, ToolResult};

mod dispatch_outcomes;
mod tool_call_text;
pub(crate) use dispatch_outcomes::*;

/// Typed dispatch for the delegation tools synthesised from the active agent.
///
/// These tools are dynamic, so their common registration resolves the admitted
/// target from its name and passes the live parent carrier explicitly. This is
/// the recursive boundary: it never reconstructs product state from a task
/// local or a downcast.
pub(crate) struct DelegationDispatch {
    tool: Arc<dyn tinytools::Tool>,
    kind: DelegationDispatchKind,
}

enum DelegationDispatchKind {
    Collapsed {
        targets: Result<Vec<super::collapsed_delegation::DelegateTarget>, String>,
    },
    Archetype,
}

impl DelegationDispatch {
    pub(crate) fn for_tool(tool: Arc<dyn tinytools::Tool>) -> Option<Self> {
        let kind = match tool.name() {
            super::collapsed_delegation::DELEGATE_TO_TOOL_NAME => {
                DelegationDispatchKind::Collapsed {
                    targets: super::collapsed_delegation::dispatch_targets_from_schema(
                        &tool.parameters_schema(),
                    ),
                }
            }
            // Only synthesized archetype delegate names enter this path.
            // `delegate_graph` is a concrete durable graph tool with its own
            // typed dispatcher, and arbitrary `delegate_*` tools must not be
            // mistaken for an agent target merely because of their spelling.
            name if AgentDefinitionRegistry::global().is_some_and(|registry| {
                registry.list().into_iter().any(|definition| {
                    definition
                        .delegate_name
                        .clone()
                        .unwrap_or_else(|| format!("delegate_{}", definition.id))
                        == name
                })
            }) =>
            {
                DelegationDispatchKind::Archetype
            }
            _ => return None,
        };
        Some(Self { tool, kind })
    }
}

#[async_trait]
impl ToolDispatch<(), crate::agent::tinyagents::host::OpenHumanRunContext> for DelegationDispatch {
    fn tool(&self) -> Arc<dyn tinytools::Tool> {
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
        let tool_context = ToolExecutionContext::from_run_context(parent, _call_id.clone());
        let child = parent.data.child();
        match &self.kind {
            DelegationDispatchKind::Collapsed { targets } => {
                let targets = match targets {
                    Ok(targets) => targets,
                    Err(reason) => {
                        return Ok(ToolResult::error(format!(
                            "delegate_to: invalid advertised target mapping: {reason}"
                        )));
                    }
                };
                super::collapsed_delegation::execute_collapsed_delegation_with_live_parent(
                    targets,
                    arguments,
                    Some(&tool_context),
                    child,
                    Some(parent),
                )
                .await
            }
            DelegationDispatchKind::Archetype => {
                let Some(agent_id) = AgentDefinitionRegistry::global().and_then(|registry| {
                    registry.list().into_iter().find_map(|definition| {
                        let name = definition
                            .delegate_name
                            .clone()
                            .unwrap_or_else(|| format!("delegate_{}", definition.id));
                        (name == self.tool.name()).then_some(definition.id.clone())
                    })
                }) else {
                    return Ok(ToolResult::error(format!(
                        "{}: delegation target is not registered",
                        self.tool.name()
                    )));
                };
                super::archetype_delegation::execute_archetype_delegation_with_live_parent(
                    &agent_id,
                    self.tool.name(),
                    arguments,
                    Some(&tool_context),
                    child,
                    Some(parent),
                )
                .await
            }
        }
    }
}

/// How a delegated sub-agent run should be scheduled relative to the parent
/// turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DispatchMode {
    /// Run the sub-agent as a durable async worker (the default for
    /// interactive archetype delegations): the tool returns immediately with
    /// an `[async_subagent_ref]` carrying `task_id` + `subagent_session_id`,
    /// and the finished result is delivered back into the parent chat as a
    /// new system-injected turn (`background_delivery`). Falls back to
    /// [`DispatchMode::Blocking`] when there is no parent agent turn or no
    /// current chat thread to deliver the result into (cron/CLI contexts) —
    /// an async result with nowhere to land would be silently lost.
    PreferAsync,
    /// Run the sub-agent inline and return its final output in this turn.
    Blocking,
}

/// Dispatch one inline child against the caller's actual TinyAgents parent
/// when the typed tool boundary has one. Standalone callers retain the
/// explicit-carrier fallback above because no live parent exists for them.
pub(crate) async fn dispatch_subagent_with_live_parent(
    agent_id: &str,
    tool_name: &str,
    prompt: &str,
    model_override: Option<&str>,
    tool_context: Option<&dyn ToolRunContext>,
    mode: DispatchMode,
    run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    live_parent: Option<&RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>>,
) -> anyhow::Result<ToolResult> {
    let parent_workspace_descriptor = tool_context
        .and_then(|ctx| ctx.workspace().cloned())
        .or_else(|| run_context.workspace.clone());
    let registry = match AgentDefinitionRegistry::global() {
        Some(reg) => reg,
        None => {
            return Ok(ToolResult::error(
                "Agent registry not initialised. This usually means the \
                 core process started without calling \
                 AgentDefinitionRegistry::init_global at startup.",
            ));
        }
    };

    // Harness registry first, then an enabled custom agent in the session's
    // config — a user-authored sub-agent lives only in the latter (#6934).
    let config = run_context
        .parent
        .as_ref()
        .and_then(|parent| parent.runtime_config.as_deref());
    let definition =
        match crate::agent::registry::resolve_spawnable_definition(registry, config, agent_id) {
            Some(def) => def,
            None => {
                return Ok(ToolResult::error(format!(
                    "{tool_name}: agent '{agent_id}' not found in registry"
                )));
            }
        };

    let parent_ctx = run_context.parent.clone();
    if let Some(ctx) = &parent_ctx {
        if !ctx.allowed_subagent_ids.contains(&definition.id) {
            log::warn!(
                "[agent] blocked delegation via {}: parent={} requested={} allowed={:?}",
                tool_name,
                ctx.agent_definition_id,
                definition.id,
                ctx.allowed_subagent_ids
            );
            return Ok(ToolResult::error(format!(
                "{tool_name}: agent '{}' is not in parent agent '{}' subagents.allowlist",
                definition.id, ctx.agent_definition_id
            )));
        }
    }

    // Registry and policy failures are deterministic and safe to report even
    // to a raw tool caller. Executing a valid delegation still requires the
    // typed harness carrier below, which supplies cancellation and authority.
    let Some(live_parent) = live_parent else {
        return Ok(ToolResult::error(
            "delegation requires a live harness run context.",
        ));
    };

    // ── Forward the current turn's attached image(s) to a vision sub-agent ──
    // The orchestrator runs on a non-vision tier and keeps the user's image as a
    // text placeholder (`[Image: … #att:<id>]`), so a delegated sub-agent would
    // otherwise get a text-only task and report "no image". When the target
    // sub-agent's model is vision-capable, prepend the placeholder(s) to its
    // prompt so its own turn rehydrates the image from the on-disk sidecar.
    let forwarded_prompt;
    let prompt: &str = {
        let images = &run_context.attachment_placeholders;
        let subagent_model = match model_override {
            Some(m) => m.to_string(),
            None => {
                let parent_model = parent_ctx
                    .as_ref()
                    .map(|p| p.model_name.as_str())
                    .unwrap_or("");
                definition.model.resolve(parent_model)
            }
        };
        if crate::agent::attachments::should_forward_parent_images(prompt)
            && !images.is_empty()
            && (agent_id == "vision_agent"
                || crate::inference::provider::factory::oh_tier_supports_vision(&subagent_model))
        {
            log::info!(
                "[agent] forwarding {} image placeholder(s) to vision sub-agent '{}'",
                images.len(),
                agent_id
            );
            forwarded_prompt = format!("{}\n\n{}", images.join("\n"), prompt);
            &forwarded_prompt
        } else {
            prompt
        }
    };

    if agent_id == "vision_agent" {
        match crate::agent::attachments::has_resolvable_image(
            prompt,
            parent_workspace_descriptor.as_ref(),
            run_context.origin.as_ref(),
        )
        .await
        {
            Ok(true) => (),
            Ok(false) => {
                return Ok(ToolResult::error(
                    "vision_agent requires a resolvable image attachment or image_paths.",
                ));
            }
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "vision image unavailable: {error}"
                )));
            }
        }
    }

    // ── Async-by-default delegation (#continuity) ─────────────────────────
    // Interactive delegations route through the durable async sub-agent
    // machinery: the parent gets an immediate `[async_subagent_ref]` with a
    // stable `subagent_session_id` it can steer/wait/continue by, the session
    // (including full history) is persisted in the per-workspace
    // `subagent_sessions` store, and the finished result is inserted into the
    // parent thread as a NEW turn via `background_completions` +
    // `background_delivery`. This is what keeps a `build_workflow` proposal
    // resumable on the next user turn instead of respawning a fresh,
    // stateless builder (the "day 0 context" bug).
    if mode == DispatchMode::PreferAsync {
        let has_parent_turn = parent_ctx.is_some();
        let has_delivery_thread = tool_context
            .and_then(ToolRunContext::thread_id)
            .or(run_context.thread_id.as_deref())
            .is_some();
        if has_parent_turn && has_delivery_thread {
            let mut async_args = serde_json::json!({
                "agent_id": definition.id.clone(),
                "prompt": prompt,
                "task_title":
                    crate::agent::orchestration::subagent_sessions::task_title_from_prompt(
                        prompt,
                    ),
            });
            if let (Some(obj), Some(model)) = (async_args.as_object_mut(), model_override) {
                obj.insert(
                    "model".to_string(),
                    serde_json::Value::String(model.to_string()),
                );
            }
            log::info!(
                "[agent] routing {tool_name} delegation of '{}' to durable async sub-agent \
                 (result will be delivered as a follow-up turn)",
                definition.id
            );
            // Box the forwarded future: `SpawnAsyncSubagentTool`'s
            // `execute_with_context` future is large, and embedding it inline
            // in every delegation tool's future (which itself nests inside
            // agent-turn futures) overflows the test-thread stack on deep
            // parallel-delegation flows.
            return Box::pin(async move {
                let detached_data = live_parent.data.detached_child();
                let detached_cancellation = detached_data.cancellation.clone();
                let detached_parent = live_parent
                    .child(
                        RunConfig::new(format!("async-subagent-{}", uuid::Uuid::new_v4())),
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
            })
            .await;
        }
        log::info!(
            "[agent] {tool_name}: async delegation requested but parent_turn={} \
             delivery_thread={} — falling back to blocking dispatch",
            has_parent_turn,
            has_delivery_thread
        );
    }

    // Past this point the run is synchronous whichever mode was requested:
    // the async branch above returns when it is taken, so a `PreferAsync`
    // that fell through registers no durable worker either and its result
    // must say so (#6033).
    let mode = DispatchMode::Blocking;

    let parent_session = parent_ctx
        .as_ref()
        .map(|p| p.session_id.clone())
        .unwrap_or_else(|| "standalone".into());
    let task_id = format!("sub-{}", uuid::Uuid::new_v4());

    crate::agent::orchestration::subagent_events::publish_subagent_spawned(
        parent_session.clone(),
        definition.id.clone(),
        "typed".to_string(),
        task_id.clone(),
        prompt.chars().count(),
    );

    // Also send to the per-request progress sink so the web channel bridge
    // emits `subagent_spawned` to the frontend (same pattern as spawn_subagent.rs).
    if let Some(progress) = run_context.progress.clone() {
        let _ = progress
            .send(AgentProgress::SubagentSpawned {
                agent_id: definition.id.clone(),
                task_id: task_id.clone(),
                mode: "typed".to_string(),
                dedicated_thread: false,
                prompt_chars: prompt.chars().count(),
                prompt: prompt.to_string(),
                worker_thread_id: None,
                display_name: Some(definition.display_name().to_string()),
                parent_call_id: crate::tools::host_extensions::tool_call_id(tool_context),
            })
            .await;
    }

    log::info!(
        "[agent] delegating to {} via {} prompt_chars={}",
        agent_id,
        tool_name,
        prompt.chars().count()
    );

    let worktree_action_dir = parent_workspace_descriptor
        .as_ref()
        .map(|descriptor| descriptor.root.clone());
    if let Some(descriptor) = parent_workspace_descriptor.as_ref() {
        tracing::debug!(
            agent_id,
            tool_name,
            workspace_root = %descriptor.root.display(),
            policy_id = %descriptor.policy_id,
            "[agent] using ToolExecutionContext workspace root for delegated subagent"
        );
    }
    let options = SubagentRunOptions {
        skill_filter_override: None,
        context: None,
        model_override: model_override.map(str::to_string),
        task_id: Some(task_id.clone()),
        thread_id: tool_context
            .and_then(ToolRunContext::thread_id)
            .or(run_context.thread_id.as_deref())
            .map(str::to_owned),
        run_context: run_context.clone(),
        worker_thread_id: None,
        initial_history: None,
        checkpoint_dir: None,
        worktree_action_dir,
        workspace_descriptor: parent_workspace_descriptor,
        run_queue: None,
    };

    let run =
        run_subagent_with_parent(live_parent, definition.clone(), prompt.to_owned(), options).await;
    match run {
        Ok(outcome) => {
            let emit_lifecycle_effects = outcome.should_emit_lifecycle_effects();
            match &outcome.status {
                // The delegated sub-agent paused on `ask_user_clarification`.
                // The runner has already checkpointed its conversation, so the
                // orchestrator must relay the question and resume via
                // `continue_subagent` — NOT re-spawn a fresh, stateless
                // sub-agent. Dropping this status was the #4291 infinite re-spawn
                // loop: a paused sub-agent was reported as a plain success, the
                // orchestrator's only continuation was to re-delegate, and the new
                // run paused again. Mirrors the `spawn_subagent` AwaitingUser path.
                SubagentRunStatus::AwaitingUser {
                    question,
                    checkpoint,
                    ..
                } => {
                    if emit_lifecycle_effects {
                        crate::agent::orchestration::subagent_events::publish_subagent_awaiting_user(
                    parent_session,
                    outcome.task_id.clone(),
                    outcome.agent_id.clone(),
                    question.clone(),
                );
                        if let Some(progress) = run_context.progress.clone() {
                            let _ = progress
                                .send(AgentProgress::SubagentAwaitingUser {
                                    agent_id: outcome.agent_id.clone(),
                                    task_id: outcome.task_id.clone(),
                                    question: question.clone(),
                                    // Synchronous delegate dispatch has no worker
                                    // sub-thread (that is a `spawn_subagent` concept).
                                    worker_thread_id: None,
                                    checkpoint_path: checkpoint
                                        .as_ref()
                                        .map(|p| p.to_string_lossy().to_string()),
                                })
                                .await;
                        }
                    }
                    log::info!(
                        "[agent] {} paused for user input via {} (task_id={}) — \
                     returning awaiting-user envelope; orchestrator must resume \
                     with continue_subagent, not re-delegate",
                        agent_id,
                        tool_name,
                        outcome.task_id,
                    );
                    Ok(awaiting_outcome_to_tool_result(
                        &outcome,
                        question,
                        checkpoint.is_some(),
                    ))
                }
                SubagentRunStatus::Completed => {
                    if emit_lifecycle_effects {
                        crate::agent::orchestration::subagent_events::publish_subagent_completed(
                            parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.elapsed.as_millis() as u64,
                            outcome.output.chars().count(),
                            outcome.iterations,
                        );
                        // Also send to the per-request progress sink (mirrors
                        // `spawn_subagent.rs`) so the web channel bridge emits
                        // `subagent_done` to the frontend. Without this the delegated
                        // subagent's timeline row (created on `SubagentSpawned` above)
                        // stays "running" forever — `publish_subagent_completed` only
                        // fires the internal DomainEvent bus, not the per-request
                        // progress channel the UI's timeline is driven from.
                        if let Some(progress) = run_context.progress.clone() {
                            let _ = progress
                                .send(AgentProgress::SubagentCompleted {
                                    agent_id: outcome.agent_id.clone(),
                                    task_id: outcome.task_id.clone(),
                                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                                    iterations: outcome.iterations as u32,
                                    output_chars: outcome.output.chars().count(),
                                    output: outcome.output.clone(),
                                    // Synchronous delegate dispatch has no worktree
                                    // isolation (that is a `spawn_subagent` concept).
                                    // Not audited for whether this child's spend reached the
                                    // parent turn's ledger, so it stays silent: omission adds
                                    // nothing, which is the status quo. See the field's docs.
                                    usage: None,
                                    worktree_path: None,
                                    changed_files: Vec::new(),
                                    dirty_status: None,
                                })
                                .await;
                        }
                    }
                    log::info!(
                        "[agent] {} completed via {} iterations={} output_chars={}",
                        agent_id,
                        tool_name,
                        outcome.iterations,
                        outcome.output.chars().count()
                    );
                    // A sub-agent that emitted a tool call instead of executing one
                    // "completes" with markup where the answer should be. Passing
                    // that through reads as a finished result, so frame it the way
                    // an iteration-cap stop is framed (#6033, #4096 precedent).
                    if is_unexecuted_tool_call_stub(&outcome.output) {
                        log::info!(
                            "[agent] {} returned an unexecuted tool-call stub (task_id={} output_chars={}) — reframing as incomplete",
                            tool_name,
                            outcome.task_id,
                            outcome.output.chars().count()
                        );
                        return Ok(ToolResult::success(incomplete_envelope(
                            tool_name,
                            "returned an unexecuted tool call instead of a result",
                            &outcome.output,
                            mode,
                        )));
                    }
                    Ok(ToolResult::success(with_inline_result_note(
                        outcome.output,
                        mode,
                    )))
                }
                // A stuck halt / iteration-cap stop returns `Incomplete`; frame the
                // partial progress so the orchestrator can't mistake it for a
                // finished result or re-run the identical delegation unchanged
                // (#4096). Still a lifecycle-completed run, so publish
                // SubagentCompleted like the `Completed` arm.
                SubagentRunStatus::Incomplete { reason } => {
                    if emit_lifecycle_effects {
                        crate::agent::orchestration::subagent_events::publish_subagent_completed(
                            parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.elapsed.as_millis() as u64,
                            outcome.output.chars().count(),
                            outcome.iterations,
                        );
                        // Same progress-sink mirror as the `Completed` arm above —
                        // an incomplete stop is still lifecycle-completed, so the
                        // timeline row must be released from "running" here too.
                        if let Some(progress) = run_context.progress.clone() {
                            let _ = progress
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
                                })
                                .await;
                        }
                    }
                    log::info!(
                        "[agent] {} stopped incomplete via {} (task_id={}) iterations={} — \
                     returning partial-progress envelope, not a finished result",
                        agent_id,
                        tool_name,
                        outcome.task_id,
                        outcome.iterations,
                    );
                    Ok(ToolResult::success(incomplete_envelope(
                        tool_name,
                        reason,
                        &outcome.output,
                        mode,
                    )))
                }
                SubagentRunStatus::Cancelled => {
                    log::info!(
                        "[agent] {} was cancelled via {} (task_id={})",
                        agent_id,
                        tool_name,
                        outcome.task_id,
                    );
                    if emit_lifecycle_effects {
                        let message = "sub-agent was cancelled".to_string();
                        crate::agent::orchestration::subagent_events::publish_subagent_failed(
                            parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            message.clone(),
                        );
                        if let Some(progress) = run_context.progress.clone() {
                            let _ = progress
                                .send(AgentProgress::SubagentFailed {
                                    agent_id: outcome.agent_id.clone(),
                                    task_id: outcome.task_id.clone(),
                                    error: message,
                                })
                                .await;
                        }
                    }
                    Ok(ToolResult::error(format!(
                        "{tool_name}: delegated sub-agent was cancelled"
                    )))
                }
            }
        }
        Err(err) => {
            let message = err.to_string();
            crate::agent::orchestration::subagent_events::publish_subagent_failed(
                parent_session,
                task_id,
                definition.id.clone(),
                message.clone(),
            );
            // Make the failure unmistakable to the orchestrator: the delegated
            // task did NOT run, so it must not be reported as success or have
            // its output fabricated. Without this guardrail a weak orchestrator
            // can narrate a plausible success from the bare error text — the
            // "hallucinated success" half of #3193 (e.g. claiming `run_code`
            // wrote a file when the coding model 404'd and nothing executed).
            Ok(ToolResult::error(format_subagent_failure(
                tool_name, &message,
            )))
        }
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
