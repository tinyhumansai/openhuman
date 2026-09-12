//! Subagent dispatch logic shared by all agent delegation tools.

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunOptions, SubagentRunStatus,
};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::tools::traits::{Tool as _, ToolCallOptions, ToolResult};
use tinytools::ToolRunContext;

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

pub(crate) async fn dispatch_subagent(
    agent_id: &str,
    tool_name: &str,
    prompt: &str,
    skill_filter: Option<&str>,
    model_override: Option<&str>,
    tool_context: Option<&dyn ToolRunContext>,
    mode: DispatchMode,
) -> anyhow::Result<ToolResult> {
    let parent_workspace_descriptor = tool_context.and_then(|ctx| ctx.workspace().cloned());
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

    let definition = match registry.get(agent_id) {
        Some(def) => def,
        None => {
            return Ok(ToolResult::error(format!(
                "{tool_name}: agent '{agent_id}' not found in registry"
            )));
        }
    };

    let parent_ctx = current_parent();
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

    // ── Forward the current turn's attached image(s) to a vision sub-agent ──
    // The orchestrator runs on a non-vision tier and keeps the user's image as a
    // text placeholder (`[Image: … #att:<id>]`), so a delegated sub-agent would
    // otherwise get a text-only task and report "no image". When the target
    // sub-agent's model is vision-capable, prepend the placeholder(s) to its
    // prompt so its own turn rehydrates the image from the on-disk sidecar.
    let forwarded_prompt;
    let prompt: &str = {
        let images = crate::openhuman::agent::harness::turn_attachments_context::current_turn_image_placeholders();
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
        if !images.is_empty()
            && crate::openhuman::inference::provider::factory::oh_tier_supports_vision(
                &subagent_model,
            )
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
        let has_delivery_thread =
            crate::openhuman::agent::tinyagents::thread_context::current_thread_id().is_some();
        if has_parent_turn && has_delivery_thread {
            let mut async_args = serde_json::json!({
                "agent_id": definition.id.clone(),
                "prompt": prompt,
                "task_title":
                    crate::openhuman::agent::orchestration::subagent_sessions::task_title_from_prompt(
                        prompt,
                    ),
            });
            if let (Some(obj), Some(model)) = (async_args.as_object_mut(), model_override) {
                obj.insert(
                    "model".to_string(),
                    serde_json::Value::String(model.to_string()),
                );
            }
            if let (Some(obj), Some(toolkit)) = (async_args.as_object_mut(), skill_filter) {
                obj.insert(
                    "toolkit".to_string(),
                    serde_json::Value::String(toolkit.to_string()),
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
                super::spawn_async_subagent::SpawnAsyncSubagentTool::new()
                    .execute_with_context(async_args, ToolCallOptions::default(), tool_context)
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

    crate::openhuman::agent::orchestration::subagent_events::publish_subagent_spawned(
        parent_session.clone(),
        definition.id.clone(),
        "typed".to_string(),
        task_id.clone(),
        prompt.chars().count(),
    );

    // Also send to the per-request progress sink so the web channel bridge
    // emits `subagent_spawned` to the frontend (same pattern as spawn_subagent.rs).
    if let Some(progress) = current_parent().and_then(|p| p.on_progress.clone()) {
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
            })
            .await;
    }

    log::info!(
        "[agent] delegating to {} via {} (skill_filter={}) prompt_chars={}",
        agent_id,
        tool_name,
        skill_filter.unwrap_or("<none>"),
        prompt.chars().count()
    );

    // Propagate the per-call toolkit scope into the subagent runner so
    // that the collapsed `SkillDelegationTool` can narrow
    // `integrations_agent` to a single Composio toolkit (e.g.
    // `delegate_to_integrations_agent { toolkit: "gmail" }` →
    // integrations_agent + toolkit="gmail"). Earlier code plumbed this through
    // `skill_filter_override` (which matches `{skill}__` QuickJS-style
    // names), but Composio actions are named `GMAIL_*` / `NOTION_*` —
    // so the filter excluded every Composio tool instead of narrowing
    // them. `toolkit_override` applies the correct `{TOOLKIT}_` prefix
    // check, restricted to skill-category tools.
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
        toolkit_override: skill_filter.map(str::to_string),
        context: None,
        model_override: model_override.map(str::to_string),
        task_id: Some(task_id.clone()),
        worker_thread_id: None,
        initial_history: None,
        checkpoint_dir: None,
        worktree_action_dir,
        workspace_descriptor: parent_workspace_descriptor,
        run_queue: None,
    };

    match run_subagent(definition, prompt, options).await {
        Ok(outcome) => match &outcome.status {
            // The delegated sub-agent paused on `ask_user_clarification`.
            // The runner has already checkpointed its conversation, so the
            // orchestrator must relay the question and resume via
            // `continue_subagent` — NOT re-spawn a fresh, stateless
            // sub-agent. Dropping this status was the #4291 infinite re-spawn
            // loop: a paused mcp_setup was reported as a plain success, the
            // orchestrator's only continuation was to re-delegate, and the new
            // run paused again. Mirrors the `spawn_subagent` AwaitingUser path.
            SubagentRunStatus::AwaitingUser {
                question,
                checkpoint,
                ..
            } => {
                crate::openhuman::agent::orchestration::subagent_events::publish_subagent_awaiting_user(
                    parent_session,
                    outcome.task_id.clone(),
                    outcome.agent_id.clone(),
                    question.clone(),
                );
                if let Some(progress) = current_parent().and_then(|p| p.on_progress.clone()) {
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
                crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
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
                if let Some(progress) = current_parent().and_then(|p| p.on_progress.clone()) {
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
                            worktree_path: None,
                            changed_files: Vec::new(),
                            dirty_status: None,
                        })
                        .await;
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
                crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
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
                if let Some(progress) = current_parent().and_then(|p| p.on_progress.clone()) {
                    let _ = progress
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
        },
        Err(err) => {
            let message = err.to_string();
            crate::openhuman::agent::orchestration::subagent_events::publish_subagent_failed(
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

/// Map a paused (`AwaitingUser`) sub-agent outcome to the tool result handed
/// back to the orchestrator: a successful `ToolResult` carrying the
/// `[SUBAGENT_AWAITING_USER]` envelope (task_id/agent_id/question + the
/// instruction to resume via `continue_subagent`). Kept as a standalone,
/// side-effect-free fn so the paused-path mapping is unit-testable without a
/// registry or a real model — the #4291 regression guard. Synchronous delegate
/// dispatch has no worker sub-thread, so `worker_thread_id` is always `None`.
///
/// **An unpersisted pause is a failure on this path, not a caveat.** The
/// envelope's "resuming may fail" wording is calibrated for the async path,
/// where a child that lost its checkpoint is still reachable through the
/// durable `subagent_sessions` store. This function serves the *synchronous*
/// delegation, which returns above before any durable session is registered
/// and has no worker thread by construction — so with no checkpoint there is
/// no resume route at all, and `continue_subagent` will find neither. Handing
/// back a success envelope would have the orchestrator put a question to the
/// user whose answer has nowhere to go, and the loss would only surface after
/// they answered. Report it as a failure instead, while the parent can still
/// act on it.
fn awaiting_outcome_to_tool_result(
    outcome: &crate::openhuman::agent::harness::subagent_runner::SubagentRunOutcome,
    question: &str,
    checkpointed: bool,
) -> ToolResult {
    if !checkpointed {
        // `question` is sub-agent-authored free text and this string is read by
        // the orchestrator, so it gets the same treatment as the envelope's:
        // JSON-encoded, not wrapped in quotes. Bare quoting is not containment —
        // the question can close the quote and continue with instructions of its
        // own. This is the hole `awaiting_user_envelope` exists to close, and an
        // error path is not exempt from it.
        let question_json = serde_json::to_string(question)
            .unwrap_or_else(|_| "\"<unserializable question>\"".into());
        return ToolResult::error(format!(
            "The sub-agent `{}` paused to ask a question, but its state could not be saved \
             and this delegation has no durable session to fall back on, so it cannot be \
             resumed. Its progress is lost. Tell the user what it was asking — {} — and \
             that the delegation has to be started again; do NOT call continue_subagent \
             with task_id `{}`, there is nothing for it to resume.",
            outcome.agent_id, question_json, outcome.task_id
        ));
    }
    ToolResult::success(super::awaiting_user::awaiting_user_envelope(
        &outcome.task_id,
        &outcome.agent_id,
        None,
        question,
        checkpointed,
    ))
}

/// Format a subagent-delegation failure so the orchestrator cannot mistake it
/// for success. Kept as a standalone, side-effect-free fn so the exact wording
/// is unit-testable without standing up a registry + failing model (#3193).
fn format_subagent_failure(tool_name: &str, message: &str) -> String {
    format!(
        "{tool_name} failed and did not complete — no work was performed and no \
         results were produced. Do NOT treat this as success or fabricate an \
         output; report the failure to the user. Error: {message}"
    )
}

/// Whether a "completed" sub-agent output is only an unexecuted tool call.
///
/// The marker vocabulary lives in
/// [`crate::openhuman::agent::harness::archivist::helpers::looks_like_unexecuted_tool_call`];
/// this adds the second half of the question — that stripping the markup
/// leaves no prose behind. A reply that merely *mentions* a tool call still
/// carries an answer and passes through untouched.
pub(crate) fn is_unexecuted_tool_call_stub(output: &str) -> bool {
    use crate::openhuman::agent::harness::archivist::helpers::{
        contains_tool_call_payload, strip_tool_calls_from_response,
    };
    if !contains_tool_call_payload(output) {
        // Prose merely naming a protocol field is not a stub.
        return false;
    }
    // A whole-text JSON payload carrying `tool_calls` is a stub however it
    // is formatted — line-based stripping cannot see that the object's
    // inner members belong to the call rather than to an answer.
    if let Ok(serde_json::Value::Object(map)) =
        serde_json::from_str::<serde_json::Value>(output.trim())
    {
        if map.contains_key("tool_calls") {
            return true;
        }
    }
    // Otherwise (XML spans, mixed prose): a stub is what leaves no words
    // behind once the markup is stripped. Structural punctuation is not an
    // answer.
    !strip_tool_calls_from_response(output)
        .chars()
        .any(char::is_alphanumeric)
}

/// The sentence appended to a blocking delegation's result.
///
/// Integration delegations run [`DispatchMode::Blocking`] and register no
/// durable worker, but the orchestrator's `[active_subagents]` guidance
/// tells it to collect completed work with `wait_subagent`. Saying so on
/// the result itself stops it hunting for a worker that never existed
/// (#6033).
const INLINE_RESULT_NOTE: &str = "\n\n[INLINE_RESULT] This delegation ran inline and is complete as returned — there is no sub-agent worker for it. Do NOT call wait_subagent, list_subagents or continue_subagent for this delegation.";

/// The same disclosure for a run that did **not** finish. It must not claim
/// the result is complete — that would contradict the
/// `[SUBAGENT_INCOMPLETE]` guardrail it is appended to — so it says only
/// that there is no worker, and what to do instead.
const NO_WORKER_NOTE: &str = "\n\n[INLINE_RESULT] This delegation ran inline and registered no sub-agent worker, so there is nothing to collect: do NOT call wait_subagent, list_subagents or continue_subagent for it. Re-delegate with a corrected prompt instead.";

/// Append [`INLINE_RESULT_NOTE`] when the dispatch was blocking.
pub(crate) fn with_inline_result_note(output: String, mode: DispatchMode) -> String {
    if mode == DispatchMode::Blocking {
        format!("{output}{INLINE_RESULT_NOTE}")
    } else {
        output
    }
}

/// The partial-progress envelope shared by every non-finishing outcome.
pub(crate) fn incomplete_envelope(
    tool_name: &str,
    reason: &str,
    output: &str,
    mode: DispatchMode,
) -> String {
    let envelope = format!(
        "[SUBAGENT_INCOMPLETE] the {tool_name} sub-agent {reason} and did not \
         finish. Below is partial progress only — do NOT report it as done or \
         re-run the identical delegation unchanged.\n\nPartial progress:\n{output}"
    );
    if mode == DispatchMode::Blocking {
        format!("{envelope}{NO_WORKER_NOTE}")
    } else {
        envelope
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
