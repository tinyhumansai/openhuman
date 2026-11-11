//! The sub-agent turn dispatch: driving one graph turn through the shared
//! tinyagents seam, building its context middleware, and folding a cap-hit
//! checkpoint summary back into the result.

use crate::agent::subagent_host::ops::checkpoint;
use std::collections::HashSet;
use std::sync::Arc;

use crate::agent::subagent_host::types::SubagentRunError;
use crate::agent::tinyagents::{SubagentScope, TurnSpec};
use crate::progress::AgentProgress;
use tinyagents_harness::run_queue::{RunQueue, RunQueueHandle};
use tinyagents_session::transcript::TranscriptMessage;
use tinyjuice_bus::types::AgentTokenjuiceCompression;
use tinytools::WorkspaceDescriptor;
use tinytools::{Tool, ToolSpec};
use tinytools_agent::dialect::TranscriptEntry;

use super::transcript::persist_subagent_transcript;
use crate::agent::subagent_host::ops::mirror_worker_thread;

#[path = "dispatch_helpers.rs"]
mod helpers;
pub(crate) use helpers::inherited_thread_id;
use helpers::{
    build_subagent_context_mw, cap_tool_results, map_tinyagents_subagent_error,
    spawn_subagent_trace_export,
};

/// Cumulative usage stats gathered across a sub-agent graph run.
#[derive(Debug, Clone, Default)]
pub(in super::super) struct AggregatedUsage {
    pub(in super::super) input_tokens: u64,
    pub(in super::super) output_tokens: u64,
    pub(in super::super) cached_input_tokens: u64,
    /// Cost of the run's calls: reported charge, catalog estimate, or unknown.
    pub(in super::super) cost: tinyagents_harness::cost::CostTally,
}

/// Drive a sub-agent turn on the tinyagents harness. Returns
/// `(text, model_calls, AggregatedUsage, early_exit_tool, hit_cap)` — `hit_cap`
/// is `true` when the run stopped at the model-call cap with work still pending
/// (the caller surfaces this as `SubagentRunStatus::Incomplete`, #4096).
#[allow(clippy::too_many_arguments)]
pub(in super::super) async fn run_subagent_via_graph(
    source: crate::agent::tinyagents::TurnModelSource,
    model: &str,
    temperature: f64,
    history: &mut Vec<TranscriptMessage>,
    parent_tools: Arc<Vec<Box<dyn Tool>>>,
    dynamic_tools: Vec<Box<dyn Tool>>,
    specs: Vec<ToolSpec>,
    allowed_names: HashSet<String>,
    max_iterations: usize,
    queues: (
        Option<Arc<RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
        Option<RunQueueHandle>,
    ),
    on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    agent_id: &str,
    task_id: &str,
    extended_policy: bool,
    thread_id: Option<String>,
    run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    worker_thread_id: Option<String>,
    workspace_dir: std::path::PathBuf,
    workspace_descriptor: Option<WorkspaceDescriptor>,
    max_output_tokens: u32,
    model_vision: bool,
    // Transcript-persistence provenance: the resolved child transcript stem
    // (`{parent_chain}__{child_session_key}`) and the provider label (the parent
    // turn's event channel) so the child's raw transcript lands in the turn
    // tree's transcript store with the right stem + provider/model meta — parity with the removed
    // `SubagentObserver::persist_transcript`.
    transcript_stem: &str,
    provider_label: &str,
    // Agent-level TokenJuice profile (`definition.effective_tokenjuice_compression()`,
    // #4466). Threaded into the sub-agent `TurnContextMiddleware` so sub-agent
    // tool outputs get the same content-aware compaction the chat path applies
    // instead of a blunt byte-cap truncation.
    tokenjuice_compression: AgentTokenjuiceCompression,
    // Host config for the `[context]` middleware knobs. Passed in rather than
    // loaded here (plan-agents Phase 3): this function is slated to move into
    // TinyAgents, where there is no config file. `None` yields the safe
    // byte-cap-only defaults.
    config: Option<&crate::config::Config>,
    native_subagent: Option<crate::agent::tinyagents::NativeSubagentRunContext>,
) -> Result<
    (
        String,
        usize,
        AggregatedUsage,
        Option<String>,
        bool,
        // Breaker-halt reason (#4466): `Some` when the repeated-failure /
        // repeat-progress circuit breaker stopped the run; the caller reports
        // `Incomplete` instead of `Completed`.
        Option<String>,
        // Typed cause is carried beside the display reason so callers do not
        // need to reconstruct failure class or operation from prose.
        Option<tinyagents_harness::no_progress::ClassifiedStopCause>,
    ),
    SubagentRunError,
> {
    let (run_queue, message_run_queue) = queues;
    let trace_session_group = run_context.thread_id.clone().or_else(|| thread_id.clone());
    // One resolved thread identity drives every child-facing boundary. An
    // explicit worker/task thread deliberately replaces the inherited parent;
    // otherwise a child remains in its parent's conversation. Do this before
    // constructing *any* model so managed requests, transcript metadata, and
    // the child carrier cannot disagree.
    let thread_id = inherited_thread_id(run_context.thread_id.clone(), thread_id);
    // Where this child's transcript is written: its turn tree's store.
    let transcripts = super::transcript::subagent_transcripts(&run_context, &workspace_dir)?;
    tracing::info!(
        model,
        max_iterations,
        agent_id,
        task_id,
        model_vision,
        observed = on_progress.is_some(),
        "[subagent_host:graph] routing sub-agent turn through tinyagents harness"
    );
    // `specs` is derived from the registry inside the runner; the tinyagents
    // adapters advertise each tool via its own `spec()`, so it's unused here.
    let _ = &specs;

    // Child-progress attribution: mirror this sub-agent's iterations / tool calls
    // / text + thinking deltas as `Subagent*` events scoped to (`agent_id`,
    // `task_id`) so the parent thread can nest them under the live subagent row.
    // Always set (not gated on `on_progress`): the scope also tells the shared
    // seam this is a sub-agent turn, so the unknown-tool recovery uses the
    // sub-agent wording. With no progress sink the scoped events simply have
    // nowhere to go, which is harmless.
    let subagent_scope = Some(SubagentScope {
        agent_id: agent_id.to_string(),
        task_id: task_id.to_string(),
        extended_policy,
    });

    // A standalone summarizer model for the cap-hit checkpoint call below (the
    // turn's own model set is consumed by the run). Built off the same source, so
    // the checkpoint invokes a crate `ChatModel` without naming `Provider`
    // (issue #4249, Phase 3 / Motion A).
    let summary_model = source
        .build_summarizer(model, temperature, thread_id.as_deref())
        .await?;

    // Resolve the sub-agent model's effective context window so the harness runs
    // the context-window summarization step (issue #4249) on sub-agent turns too.
    // A long-running / resumed sub-agent (worker threads, durable sessions) can
    // accumulate a transcript past its own window; summarize before each model
    // call rather than relying solely on the parent's one-time trim.
    let context_window = source.effective_context_window(model).await;

    // Build the child turn's crate `ChatModel` set from the source; capability
    // reads (vision/native-tools) + telemetry id now come off the built bundle,
    // so the sub-agent path names crate model types only.
    let turn_models = source
        .build(model, temperature, context_window, thread_id.as_deref())
        .await?;

    // Vision forwarding (parity with the legacy `run_inner_loop`): rehydrate
    // `[IMAGE:…]` placeholders in the sub-agent's history when either the model
    // advertises vision or the sub-agent model is user-flagged as vision-capable
    // (BYOK/custom). The expanded copy is provider-only — the persisted `history`
    // written back below keeps the original markers.
    let dispatch_history = if crate::agent::multimodal::has_image_placeholders(history) {
        crate::agent::multimodal::rehydrate_image_placeholders(history).await
    } else {
        history.clone()
    };

    // Build the sub-agent's context middleware from the live `[context]` config +
    // the agent's TokenJuice profile (#4466), matching how the chat path wires
    // `TurnContextMiddleware` (session/turn/core.rs). The migrated sub-agent path
    // had regressed to `TurnContextMiddleware::defaults()` — compression Off — so
    // sub-agent tool outputs took a blunt 16 KiB truncation instead of the
    // content-aware TokenJuice compaction the definition asked for. Honor the
    // `[context]` enabled / autocompact opt-outs, microcompact keep-recent, and
    // per-result byte budget too, so a sub-agent turn compacts like a chat turn.
    let context_mw = build_subagent_context_mw(tokenjuice_compression, config);

    // Live transcript snapshot sink (#4466): the harness owns the working message
    // vector and drops it on a mid-run `Err`, so a failed sub-agent run used to
    // persist NOTHING (breaking `learning/transcript_ingest`) and leave an empty
    // worker thread. Attach a snapshot middleware that mirrors each `before_model`
    // request's transcript here, so the error path below can still persist the
    // rounds that completed before the failure.
    let transcript_snapshot =
        crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshotSink::default();
    // The seeded history is always kept; only rounds past it are split into
    // accepted and unanswered on failure (#6281).
    transcript_snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .request_base_len = dispatch_history.len();

    // A sub-agent turn runs *nested inside* the parent agent's turn (parent
    // harness → spawn_subagent tool → here), so the child's full
    // `TurnSpec::run` future would otherwise sit on the parent's
    // poll stack. Heap-allocate it (as the legacy `run_inner_loop` did) so the
    // parent+child harness drives don't overflow the stack.
    // Native-tool support drives the durable-history suffix dispatcher; capture it
    // (and the telemetry id) before `turn_models` is moved into the runner.
    let native_tools = turn_models.native_tools();
    let provider_id = turn_models.provider_id().to_string();
    let usage_provider_id = provider_id.clone();
    // `SubagentRunOptions` already carries the child context built at the tool
    // dispatch boundary. Reusing that owned value preserves its immediate
    // parent-ledger link; forking again here would hide nested child usage in a
    // second, unreachable ledger.
    let dispatch = run_context.dispatch_ctx();
    let child_tool_policy = child_tool_policy(
        run_context.tool_policy.as_ref(),
        agent_id,
        &allowed_names,
        &dynamic_tools
            .iter()
            .chain(parent_tools.iter())
            .map(|tool| tool.as_ref())
            .collect::<Vec<_>>(),
    );
    let mut child_context = run_context;
    child_context.tool_policy = child_tool_policy.clone();
    // A parallel task may omit `thread_id`; that means inherit the parent
    // conversation, not erase it. Only an explicit task thread may replace
    // the typed carrier's inherited affinity.
    child_context.thread_id = thread_id.clone();
    child_context.progress = on_progress.clone().or(child_context.progress);
    child_context.workspace = workspace_descriptor.clone().or(child_context.workspace);
    let turn_spec = TurnSpec {
        run_context: child_context,
        turn_models,
        provider_id,
        model,
        history: dispatch_history,
        // Dynamic (per-spawn) tools first so a dynamic tool that intentionally
        // shadows a parent-registry tool of the same name is the one that
        // *executes* — matching the advertisement order (`dedup_visible_tool_specs`
        // keeps the first dynamic spec before parent specs in `runner.rs`). The shared
        // adapter resolves a name by scanning the sets in order, so a
        // parent-first order would run the parent impl for a shadowed name.
        tool_sets: vec![Arc::new(dynamic_tools), parent_tools],
        // Fail-closed (issue #4452): a sub-agent ALWAYS carries a concrete,
        // resolved allowlist (`allowed_names`), so pass it as `Some(..)`. An empty
        // set is therefore a genuine deny-all — a tool-less agent
        // (`ToolScope::Named([])`), a zero-match `skill_filter`, or a `named` list
        // that resolved to nothing registers ZERO tools instead of implicitly
        // inheriting the parent's full surface (shell/file-write/spawn).
        allowed: Some(allowed_names),
        max_iterations,
        subagent_scope,
        // Resolved above — drives the sub-agent context-window summarization step.
        context_window,
        // Mid-flight steering: forward queued steer messages into the run.
        run_queue,
        message_run_queue,
        // Pause + checkpoint when the child asks the user a clarifying question.
        early_exit_tools: &["ask_user_clarification"],
        // Pause gracefully at the model-call cap so we can summarize a resumable
        // checkpoint (below) instead of erroring — legacy cap-summary parity.
        pause_at_cap: true,
        // Bound the sub-agent's per-call output at its configured budget.
        max_output_tokens: Some(max_output_tokens),
        // Context middlewares (#4466): config-sourced TokenJuice compaction +
        // tool-result byte cap + microcompact + summarization opt-outs (built
        // above), plus the live transcript-snapshot sink for error recovery.
        context_mw: {
            let mut mw = context_mw;
            mw.transcript_snapshot = Some(transcript_snapshot.clone());
            mw
        },
        // Preserve the parent session's channel boundary for this child. The
        // host gate also enforces its autonomy and turn-origin rules.
        tool_policy: child_tool_policy,
        // Sub-agent turns run tools with external effects; not a deterministic
        // internal run, so response caching stays off (safe default).
        deterministic_cacheable: false,
        // #4457 (defect C): irrelevant for sub-agents — they carry a
        // `subagent_scope`, so the seam never emits a top-level `TurnCompleted`
        // (they report via `Subagent*` events). Pass `false` for clarity.
        defer_turn_completed_to_caller: false,
    };
    let child_journal_run_id = native_subagent
        .as_ref()
        .map(|native| native.config.run_id.as_str().to_string());
    let run_result = match native_subagent.as_ref() {
        Some(native) => turn_spec.run_with_native_subagent(native.clone()).await,
        None => turn_spec.run().await,
    };

    let mut outcome = match run_result {
        Ok(outcome) => outcome,
        Err(err) => {
            // #4466: the harness dropped its partial transcript, but the snapshot
            // middleware mirrored every completed round. Persist those rounds to
            // the turn tree's transcript store (so the thread's usage and view
            // still count a failed run) and mirror them onto the worker thread, THEN surface the
            // error. Previously the `?`-return skipped both persistence steps, so
            // a failed run left no transcript and an empty worker thread.
            let mapped = map_tinyagents_subagent_error(err);
            // Persist the caller's original history plus only what a provider
            // accepted as structured messages: a request rejected for malformed
            // tool history must not be replayed by a resumed sub-agent, and the
            // snapshot's own seed is the provider-bound copy with rehydrated
            // images. The unanswered suffix rides the failure marker as text
            // (#6281).
            let (recovered, unanswered_steps, recovered_usage, completed_rounds) = {
                let snapshot = transcript_snapshot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let (recovered, unanswered) =
                    super::transcript::failed_run_history(history.as_slice(), &snapshot);
                let usage = AggregatedUsage {
                    input_tokens: snapshot.input_tokens,
                    output_tokens: snapshot.output_tokens,
                    cached_input_tokens: snapshot.cached_input_tokens,
                    // Each answered call was priced as it arrived.
                    cost: snapshot.cost,
                };
                (recovered, unanswered, usage, snapshot.model_calls)
            };
            tracing::warn!(
                agent_id,
                task_id,
                error = %mapped,
                recovered_rounds = recovered.len(),
                unanswered_steps = unanswered_steps.is_some(),
                "[subagent_host:graph] sub-agent run errored; persisting recovered transcript before returning (#4466)"
            );
            super::transcript::persist_failed_run(
                &dispatch,
                transcripts.as_ref(),
                &workspace_dir,
                transcript_stem,
                agent_id,
                task_id,
                provider_label,
                model,
                &recovered,
                &recovered_usage,
                thread_id.as_deref(),
                unanswered_steps.as_deref(),
                completed_rounds,
                context_window.unwrap_or(0),
                if native_tools { "native" } else { "xml" },
                worker_thread_id.as_deref(),
                &mapped,
            );
            if let Some(native) = &native_subagent {
                native.finalize_failed_boundary();
            }
            spawn_subagent_trace_export(
                config,
                &dispatch,
                child_journal_run_id.as_deref(),
                trace_session_group.as_deref(),
                agent_id,
                task_id,
            );
            return Err(mapped);
        }
    };

    // Write the final conversation back so the caller can checkpoint / persist.
    // Keep the original (un-expanded) prior turns and append only this turn's typed
    // suffix, serialized with the matching dispatcher so a native tool round
    // persists as the `{content, tool_calls}` / `{tool_call_id, content}` envelope
    // (re-parsed by `convert::chat_message_to_message` next turn) instead of an
    // assistant with no `tool_calls` followed by an orphan `tool` row. Appending
    // the typed `outcome.conversation` (messages-since-last-user) also avoids
    // indexing a post-trim `outcome.history` with the pre-trim length, and the
    // durable `[IMAGE:…]` markers stay put since the prior user turns are untouched.
    let suffix = if native_tools {
        crate::agent::message_convert::provider_messages_from_conversation(
            &tinytools_agent::dialect::NativeDialect,
            &outcome.conversation,
        )
    } else {
        crate::agent::message_convert::provider_messages_from_conversation(
            &tinytools_agent::dialect::XmlDialect,
            &outcome.conversation,
        )
    };
    history.extend(suffix);

    let mut usage = AggregatedUsage {
        input_tokens: outcome.input_tokens,
        output_tokens: outcome.output_tokens,
        // Carry the child's cached-prefix tokens + estimated cost (the turn
        // outcome now reports both) so sub-agent spend rolls into the parent
        // instead of being recorded as uncached and $0.
        cached_input_tokens: outcome.cached_input_tokens,
        cost: outcome.cost,
    };

    // Cap hit with work still pending: summarize the run-so-far into a resumable
    // checkpoint (the delegating agent continues from partial progress) rather
    // than surfacing an empty/partial answer — the legacy `SubagentCheckpoint`.
    if outcome.hit_cap {
        let results = cap_tool_results(&outcome.conversation, &outcome.tool_outcomes);
        let strategy = checkpoint::SubagentCheckpoint {
            chat_model: summary_model.clone(),
            agent_id: agent_id.to_string(),
            model: model.to_string(),
            // The checkpoint summary call's output cap. #4469 item 5: honour this
            // sub-agent definition's own per-call output budget (the same
            // `max_output_tokens` bounding every task model call above) instead of
            // the process-global `AGENT_TURN_MAX_OUTPUT_TOKENS` floor, so a
            // definition that raised or lowered its output cap is respected by the
            // cap-summary call too.
            max_output_tokens,
            trace: native_subagent.as_ref().and_then(|native| {
                native
                    .child_events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
                    .map(|events| {
                        crate::agent::tinyagents::SupplementalTrace::new(
                            events,
                            native.config.run_id.clone(),
                        )
                    })
            }),
        };
        match strategy.summarize_cap_hit(&results, max_iterations).await {
            Ok(co) => {
                if let Some(u) = co.usage {
                    // Fold ALL four token fields (the legacy cap-summary folded
                    // cached tokens too, not just input/output), then price the
                    // call and feed the global cost tracker directly (#4467,
                    // item 2). The checkpoint summary call bypasses the harness so
                    // the observability bridge never sees it — without this record
                    // its cached tokens are lost and it costs $0 in the footer /
                    // transcript meta / cost dashboard.
                    usage.input_tokens += u.input_tokens;
                    usage.output_tokens += u.output_tokens;
                    usage.cached_input_tokens += u.cached_input_tokens();
                    let call_cost = crate::agent::cost::call_cost(model, &u);
                    usage.cost.add(call_cost);
                    let billed = tinyinference_llm::usage::BilledUsage::from_counts(
                        u.input_tokens,
                        u.output_tokens,
                    )
                    .with_context_window(u.context_window())
                    .with_cached_input_tokens(u.cached_input_tokens())
                    .with_cache_creation_tokens(u.cache_creation_tokens)
                    .with_reasoning_tokens(u.reasoning_tokens);
                    let billed = match call_cost {
                        tinyagents_harness::cost::CallCost::Charged(usd) => {
                            billed.with_charged_usd(usd)
                        }
                        tinyagents_harness::cost::CallCost::Estimated(usd) => {
                            billed.with_estimated_usd(usd)
                        }
                        tinyagents_harness::cost::CallCost::Unknown => billed,
                    };
                    crate::platform::cost::record_provider_usage_scoped(
                        model,
                        &billed,
                        crate::platform::cost::UsageScope::in_ctx(
                            &dispatch,
                            Some(&usage_provider_id),
                            Some((agent_id, task_id)),
                        ),
                    );
                    tracing::debug!(
                        agent_id,
                        input_tokens = u.input_tokens,
                        output_tokens = u.output_tokens,
                        cached_input_tokens = u.cached_input_tokens(),
                        ?call_cost,
                        "[subagent] cap-hit summary call folded + priced + recorded into cost tracker (#4467, item 2)"
                    );
                }
                outcome.text = co.text;
            }
            Err(e) => return Err(SubagentRunError::Provider(e)),
        }
    }

    // Persist the sub-agent's raw transcript to the turn tree's transcript
    // store (parity with the removed `SubagentObserver::persist_transcript`),
    // where the thread's usage and transcript view read every child back.
    // On a cap-hit / early-exit, `outcome.text` is the checkpoint (or clarifying
    // question) that stands in for a final assistant turn — append it so the
    // persisted transcript reflects the actual final state, not the pre-checkpoint
    // history. `history` already carries this turn's typed suffix.
    let transcript_history;
    let history_for_transcript: &[TranscriptMessage] = if (outcome.hit_cap
        || outcome.early_exit_tool.is_some())
        && !outcome.text.trim().is_empty()
    {
        transcript_history = {
            let mut messages = history.clone();
            messages.push(TranscriptMessage::assistant(outcome.text.clone()));
            messages
        };
        &transcript_history
    } else {
        history.as_slice()
    };
    persist_subagent_transcript(
        transcripts.as_ref(),
        transcript_stem,
        agent_id,
        task_id,
        provider_label,
        model,
        history_for_transcript,
        &usage,
        thread_id.as_deref(),
        context_window.unwrap_or(0),
        // Match the dispatcher the history was actually serialized with (text-mode
        // integrations turns write XML), and the real iteration count.
        if native_tools { "native" } else { "xml" },
        outcome.model_calls as u32,
    );

    // Mirror this turn's conversation to the spawn's worker thread (when one is
    // attached), matching the legacy `SubagentObserver`: assistant intents +
    // final answer as `agent` messages, tool results as `user` messages. The
    // initial user prompt was already written when the worker thread was created.
    if let Some(thread_id) = worker_thread_id {
        mirror_worker_thread(
            &dispatch,
            &workspace_dir,
            &thread_id,
            agent_id,
            task_id,
            &outcome.conversation,
            // On a cap/early-exit, `outcome.text` is the checkpoint/question that
            // replaced (or stands in for) a final assistant turn.
            if outcome.hit_cap || outcome.early_exit_tool.is_some() {
                Some(outcome.text.as_str())
            } else {
                None
            },
        );
    }

    // The child run's own loop ends before OpenHuman persists its transcript
    // and any cap checkpoint. Finish the native journal only after that durable
    // host boundary, then acknowledge it on the parent stream so the linked
    // sub-agent span cannot close ahead of its child record.
    if let Some(terminal) = outcome.deferred_terminal.take() {
        let stop = crate::agent::session_host::stop_policy::classify_stop_with_cause(
            outcome.breaker_halt.as_deref(),
            outcome.failure_cause.clone(),
            false,
            outcome.hit_cap,
        );
        terminal
            .finish(stop, (outcome.model_calls as u32).max(1))
            .await;
    }
    if let Some(native) = &native_subagent {
        native.parent_events.emit(
            tinyagents_harness::events::AgentEvent::SubAgentRunCompleted {
                run_id: native.config.run_id.clone(),
            },
        );
    }
    // Export only after the child journal's finalization and status flush so a
    // detached reader cannot race ahead and force-close its spans at wall time.
    spawn_subagent_trace_export(
        config,
        &dispatch,
        child_journal_run_id.as_deref(),
        trace_session_group.as_deref(),
        agent_id,
        task_id,
    );

    // On an early-exit (`ask_user_clarification`), `outcome.text` is the question
    // and the runner checkpoints + returns AwaitingUser. `None` = ran to a final
    // answer (or a cap-hit checkpoint summary).
    Ok((
        outcome.text,
        outcome.model_calls,
        usage,
        outcome.early_exit_tool,
        outcome.hit_cap,
        // #4466: propagate a circuit-breaker halt so the runner reports Incomplete.
        outcome.breaker_halt,
        outcome.failure_cause,
    ))
}

/// Rebuilds the inherited channel ceiling for the child's callable tool set.
/// A parent's `HideFromPrompt` entries describe only that parent's projection;
/// carrying them forward would hide tools explicitly granted by the child
/// definition. Explicit denials and approval requirements still narrow the
/// child when they refer to tools in its resolved allowlist.
pub(super) fn child_tool_policy(
    parent: Option<&crate::agent::tinyagents::ToolPolicyEnforcement>,
    child_agent_id: &str,
    child_allowed_names: &HashSet<String>,
    child_tools: &[&dyn Tool],
) -> Option<crate::agent::tinyagents::ToolPolicyEnforcement> {
    let parent = parent?;
    let profile = &parent.session.profile;
    let channel_permissions = [(
        profile.channel.clone(),
        profile.allowed_permission.to_string(),
    )]
    .into_iter()
    .collect();
    let mut session = tinytools::agent_policy::ToolPolicyEngine::build_session_from_refs(
        child_agent_id,
        profile.channel.clone(),
        profile.entrypoint.clone(),
        &channel_permissions,
        child_tools,
        child_allowed_names,
    );

    for name in child_allowed_names {
        let Some(decision) = parent.session.decisions.get(name) else {
            continue;
        };
        if !matches!(
            decision.action,
            tinytools::ToolPolicyAction::Deny | tinytools::ToolPolicyAction::RequireApproval
        ) {
            continue;
        }
        // The parent closes the packs of the specialists it can hand off to
        // (`close_handed_off_packs`) so it uses the hand-off instead of the raw
        // tools. That denial is the parent's own route policy: inheriting it
        // would strip the specialist of its own belt (`skill_setup` could neither
        // search nor install). Every other denial narrows the child.
        if decision.action == tinytools::ToolPolicyAction::Deny
            && crate::tools::toolpacks::pack_for_tool(name)
                .is_some_and(|pack| pack.is_owner(child_agent_id))
        {
            continue;
        }
        session.decisions.insert(name.clone(), decision.clone());
        session.allowed_tool_names.remove(name);
        session.blocked_tool_names.insert(name.clone());
    }

    Some(crate::agent::tinyagents::ToolPolicyEnforcement {
        policy: parent.policy.clone(),
        session,
        channel: parent.channel.clone(),
        agent_definition_id: child_agent_id.to_string(),
    })
}

#[cfg(test)]
#[path = "dispatch_policy_tests.rs"]
mod policy_tests;
