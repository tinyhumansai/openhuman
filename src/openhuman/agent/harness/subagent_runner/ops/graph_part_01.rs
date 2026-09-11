use std::collections::HashSet;
use std::sync::Arc;

use crate::openhuman::agent::harness::agent_graph::{
    AgentTurnRequest, AgentTurnResult, AgentTurnUsage,
};
use crate::openhuman::agent::harness::subagent_runner::types::SubagentRunError;
use crate::openhuman::agent::messages::{ChatMessage, ConversationMessage};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::tinyagents::{run_turn_via_tinyagents_shared, SubagentScope};
use crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression;
use crate::openhuman::tools::{Tool, ToolSpec};
use tinyagents_harness::workspace::WorkspaceDescriptor;

/// Cumulative usage stats gathered across a sub-agent graph run.
#[derive(Debug, Clone, Default)]
pub(super) struct AggregatedUsage {
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) cached_input_tokens: u64,
    pub(super) charged_amount_usd: f64,
}

/// Run an assembled custom per-agent turn through the shared default sub-agent
/// leaf. Bespoke `AgentGraph::Custom` graphs use this after their own routing
/// nodes so transcript persistence, worker-thread mirroring, progress events,
/// handoff middleware, cap summaries, and usage aggregation stay byte-for-byte
/// on the default path.
pub(crate) async fn run_agent_turn_request_via_default_graph(
    req: AgentTurnRequest,
) -> Result<AgentTurnResult, SubagentRunError> {
    let AgentTurnRequest {
        turn_model_source,
        model,
        temperature,
        mut history,
        parent_tools,
        dynamic_tools,
        specs,
        allowed_names,
        max_iterations,
        run_queue,
        on_progress,
        agent_id,
        task_id,
        extended_policy,
        worker_thread_id,
        workspace_dir,
        workspace_descriptor,
        max_output_tokens,
        model_vision,
        transcript_stem,
        provider_label,
        handoff_cache,
        tokenjuice_compression,
        config,
    } = req;

    let (output, iterations, usage, early_exit_tool, hit_cap, breaker_halt) =
        run_subagent_via_graph(
            turn_model_source,
            &model,
            temperature,
            &mut history,
            parent_tools,
            dynamic_tools,
            specs,
            allowed_names,
            max_iterations,
            run_queue,
            on_progress,
            &agent_id,
            &task_id,
            extended_policy,
            worker_thread_id,
            workspace_dir,
            workspace_descriptor,
            max_output_tokens,
            model_vision,
            &transcript_stem,
            &provider_label,
            handoff_cache,
            tokenjuice_compression,
            config.as_deref(),
        )
        .await?;

    Ok(AgentTurnResult {
        history,
        output,
        iterations,
        usage: AgentTurnUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            charged_amount_usd: usage.charged_amount_usd,
        },
        early_exit_tool,
        hit_cap,
        breaker_halt,
    })
}

/// Drive a sub-agent turn on the tinyagents harness. Returns
/// `(text, model_calls, AggregatedUsage, early_exit_tool, hit_cap)` — `hit_cap`
/// is `true` when the run stopped at the model-call cap with work still pending
/// (the caller surfaces this as `SubagentRunStatus::Incomplete`, #4096).
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_subagent_via_graph(
    source: crate::openhuman::agent::tinyagents::TurnModelSource,
    model: &str,
    temperature: f64,
    history: &mut Vec<ChatMessage>,
    parent_tools: Arc<Vec<Box<dyn Tool>>>,
    dynamic_tools: Vec<Box<dyn Tool>>,
    specs: Vec<ToolSpec>,
    allowed_names: HashSet<String>,
    max_iterations: usize,
    run_queue: Option<Arc<crate::openhuman::agent::harness::run_queue::RunQueue>>,
    on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    agent_id: &str,
    task_id: &str,
    extended_policy: bool,
    worker_thread_id: Option<String>,
    workspace_dir: std::path::PathBuf,
    workspace_descriptor: Option<WorkspaceDescriptor>,
    max_output_tokens: u32,
    model_vision: bool,
    // Transcript-persistence provenance: the resolved child transcript stem
    // (`{parent_chain}__{child_session_key}`) and the provider label (the parent
    // turn's event channel) so the child's raw transcript lands in `session_raw`
    // with the right stem + provider/model meta — parity with the removed
    // `SubagentObserver::persist_transcript`.
    transcript_stem: &str,
    provider_label: &str,
    // Progressive-disclosure handoff cache (integrations_agent with a resolved
    // toolkit); `Some` installs the `HandoffMiddleware` that stashes oversized
    // tool results and shares the cache with the `extract_from_result` tool.
    handoff_cache: Option<
        std::sync::Arc<crate::openhuman::agent::harness::subagent_runner::ResultHandoffCache>,
    >,
    // Agent-level TokenJuice profile (`definition.effective_tokenjuice_compression()`,
    // #4466). Threaded into the sub-agent `TurnContextMiddleware` so sub-agent
    // tool outputs get the same content-aware compaction the chat path applies
    // instead of a blunt byte-cap truncation.
    tokenjuice_compression: AgentTokenjuiceCompression,
    // Host config for the `[context]` middleware knobs. Passed in rather than
    // loaded here (plan-agents Phase 3): this function is slated to move into
    // TinyAgents, where there is no config file. `None` yields the safe
    // byte-cap-only defaults.
    config: Option<&crate::openhuman::config::Config>,
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
    ),
    SubagentRunError,
> {
    tracing::info!(
        model,
        max_iterations,
        agent_id,
        task_id,
        model_vision,
        observed = on_progress.is_some(),
        "[subagent_runner:graph] routing sub-agent turn through tinyagents harness"
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
    let summary_model = source.build_summarizer(model, temperature)?;

    // Resolve the sub-agent model's effective context window so the harness runs
    // the context-window summarization step (issue #4249) on sub-agent turns too.
    // A long-running / resumed sub-agent (worker threads, durable sessions) can
    // accumulate a transcript past its own window; summarize before each model
    // call rather than relying solely on the parent's one-time trim.
    let context_window = source.effective_context_window(model).await;

    // Build the child turn's crate `ChatModel` set from the source; capability
    // reads (vision/native-tools) + telemetry id now come off the built bundle,
    // so the sub-agent path names crate model types only.
    let turn_models = source.build(model, temperature, context_window)?;

    // Vision forwarding (parity with the legacy `run_inner_loop`): rehydrate
    // `[IMAGE:…]` placeholders in the sub-agent's history when either the model
    // advertises vision or the sub-agent model is user-flagged as vision-capable
    // (BYOK/custom). The expanded copy is provider-only — the persisted `history`
    // written back below keeps the original markers.
    let dispatch_history = if (turn_models.supports_vision() || model_vision)
        && crate::openhuman::agent::multimodal::has_image_placeholders(history)
    {
        crate::openhuman::agent::multimodal::rehydrate_image_placeholders(history)
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
    let transcript_snapshot: crate::openhuman::agent::tinyagents::TranscriptSnapshotSink =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    // A sub-agent turn runs *nested inside* the parent agent's turn (parent
    // harness → spawn_subagent tool → here), so the child's full
    // `run_turn_via_tinyagents_shared` future would otherwise sit on the parent's
    // poll stack. Heap-allocate it (as the legacy `run_inner_loop` did) so the
    // parent+child harness drives don't overflow the stack.
    // Native-tool support drives the durable-history suffix dispatcher; capture it
    // (and the telemetry id) before `turn_models` is moved into the runner.
    let native_tools = turn_models.native_tools();
    let provider_id = turn_models.provider_id().to_string();
    let run_result = Box::pin(run_turn_via_tinyagents_shared(
        turn_models,
        provider_id,
        model,
        dispatch_history,
        // Dynamic (per-spawn) tools first so a dynamic tool that intentionally
        // shadows a parent-registry tool of the same name is the one that
        // *executes* — matching the advertisement order (`dedup_tool_specs_by_name`
        // lists dynamic specs before parent specs in `runner.rs`). The shared
        // adapter resolves a name by scanning the sets in order, so a
        // parent-first order would run the parent impl for a shadowed name.
        vec![Arc::new(dynamic_tools), parent_tools],
        // Fail-closed (issue #4452): a sub-agent ALWAYS carries a concrete,
        // resolved allowlist (`allowed_names`), so pass it as `Some(..)`. An empty
        // set is therefore a genuine deny-all — a tool-less agent
        // (`ToolScope::Named([])`), a zero-match `skill_filter`, or a `named` list
        // that resolved to nothing registers ZERO tools instead of implicitly
        // inheriting the parent's full surface (shell/file-write/spawn).
        Some(allowed_names),
        max_iterations,
        // Parent's progress sink — child events ride it, scoped below.
        on_progress,
        subagent_scope,
        // Resolved above — drives the sub-agent context-window summarization step.
        context_window,
        // Mid-flight steering: forward queued steer messages into the run.
        run_queue,
        // Pause + checkpoint when the child asks the user a clarifying question.
        &["ask_user_clarification"],
        // Pause gracefully at the model-call cap so we can summarize a resumable
        // checkpoint (below) instead of erroring — legacy cap-summary parity.
        true,
        // Bound the sub-agent's per-call output at its configured budget.
        Some(max_output_tokens),
        // Context middlewares (#4466): config-sourced TokenJuice compaction +
        // tool-result byte cap + microcompact + summarization opt-outs (built
        // above), plus the progressive-disclosure handoff when a cache is
        // attached, plus the live transcript-snapshot sink for error recovery.
        {
            let mut mw = context_mw;
            if let Some(cache) = handoff_cache {
                mw.handoff = Some(crate::openhuman::agent::tinyagents::HandoffConfig {
                    cache,
                    agent_id: agent_id.to_string(),
                    task_id: task_id.to_string(),
                });
            }
            mw.transcript_snapshot = Some(transcript_snapshot.clone());
            mw
        },
        // Sub-agents gate via their own SubagentToolSource policy path, not the
        // session `.tool_policy()`; no enforcement threaded here.
        None,
        // Isolated worker descriptor, when worktree isolation prepared one.
        workspace_descriptor,
        // Sub-agent turns run tools with external effects; not a deterministic
        // internal run, so response caching stays off (safe default).
        false,
        // #4457 (defect C): irrelevant for sub-agents — they carry a
        // `subagent_scope`, so the seam never emits a top-level `TurnCompleted`
        // (they report via `Subagent*` events). Pass `false` for clarity.
        false,
    ))
    .await;

    let mut outcome = match run_result {
        Ok(outcome) => outcome,
        Err(err) => {
            // #4466: the harness dropped its partial transcript, but the snapshot
            // middleware mirrored every completed round. Persist those rounds to
            // `session_raw` (so `learning/transcript_ingest` can still read a
            // failed run) and mirror them onto the worker thread, THEN surface the
            // error. Previously the `?`-return skipped both persistence steps, so
            // a failed run left no transcript and an empty worker thread.
            let mapped = map_tinyagents_subagent_error(err);
            let recovered = transcript_snapshot
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();
            tracing::warn!(
                agent_id,
                task_id,
                error = %mapped,
                recovered_rounds = recovered.len(),
                "[subagent_runner:graph] sub-agent run errored; persisting recovered transcript before returning (#4466)"
            );
            persist_failed_run(
                &workspace_dir,
                transcript_stem,
                agent_id,
                task_id,
                provider_label,
                model,
                &recovered,
                context_window.unwrap_or(0),
                if native_tools { "native" } else { "xml" },
                worker_thread_id.as_deref(),
                &mapped,
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
    use crate::openhuman::agent::dispatcher::ToolDispatcher;
    let suffix = if native_tools {
        crate::openhuman::agent::dispatcher::NativeToolDispatcher
            .to_provider_messages(&outcome.conversation)
    } else {
        crate::openhuman::agent::dispatcher::XmlToolDispatcher
            .to_provider_messages(&outcome.conversation)
    };
    history.extend(suffix);

    let mut usage = AggregatedUsage {
        input_tokens: outcome.input_tokens,
        output_tokens: outcome.output_tokens,
        // Carry the child's cached-prefix tokens + estimated cost (the turn
        // outcome now reports both) so sub-agent spend rolls into the parent
        // instead of being recorded as uncached and $0.
        cached_input_tokens: outcome.cached_input_tokens,
        charged_amount_usd: outcome.charged_amount_usd,
    };

    // Cap hit with work still pending: summarize the run-so-far into a resumable
    // checkpoint (the delegating agent continues from partial progress) rather
    // than surfacing an empty/partial answer — the legacy `SubagentCheckpoint`.
    if outcome.hit_cap {
        let digest = build_cap_digest(&outcome.conversation, &outcome.tool_outcomes);
        let strategy = super::checkpoint::SubagentCheckpoint {
            chat_model: summary_model.clone(),
            agent_id: agent_id.to_string(),
            // The checkpoint summary call's output cap. #4469 item 5: honour this
            // sub-agent definition's own per-call output budget (the same
            // `max_output_tokens` bounding every task model call above) instead of
            // the process-global `AGENT_TURN_MAX_OUTPUT_TOKENS` floor, so a
            // definition that raised or lowered its output cap is respected by the
            // cap-summary call too.
            max_output_tokens,
        };
        match strategy.summarize_cap_hit(&digest, max_iterations).await {
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
                    usage.cached_input_tokens += u.cached_input_tokens;
                    let call_cost =
                        if u.charged_amount_usd.is_finite() && u.charged_amount_usd > 0.0 {
                            u.charged_amount_usd
                        } else {
                            crate::openhuman::platform::cost::catalog::estimate_cost_usd(
                                model,
                                u.input_tokens,
                                u.output_tokens,
                                u.cached_input_tokens,
                            )
                        };
                    usage.charged_amount_usd += call_cost;
                    crate::openhuman::platform::cost::record_provider_usage(
                        model,
                        &crate::openhuman::inference::provider::UsageInfo {
                            input_tokens: u.input_tokens,
                            output_tokens: u.output_tokens,
                            context_window: u.context_window,
                            cached_input_tokens: u.cached_input_tokens,
                            cache_creation_tokens: u.cache_creation_tokens,
                            reasoning_tokens: u.reasoning_tokens,
                            charged_amount_usd: call_cost,
                        },
                    );
                    tracing::debug!(
                        agent_id,
                        input_tokens = u.input_tokens,
                        output_tokens = u.output_tokens,
                        cached_input_tokens = u.cached_input_tokens,
                        call_cost,
                        "[subagent] cap-hit summary call folded + priced + recorded into cost tracker (#4467, item 2)"
                    );
                }
                outcome.text = co.text;
            }
            Err(e) => return Err(SubagentRunError::Provider(e)),
        }
    }

    // Persist the sub-agent's raw transcript to `session_raw` (parity with the
    // removed `SubagentObserver::persist_transcript`). The graph runner replaced
    // the observer but only mirrored to the worker thread, so per-child
    // transcripts stopped being written — breaking downstream learning ingestion
    // (`learning/transcript_ingest`, which reads `session_raw/*.jsonl`).
    // On a cap-hit / early-exit, `outcome.text` is the checkpoint (or clarifying
    // question) that stands in for a final assistant turn — append it so the
    // persisted transcript reflects the actual final state, not the pre-checkpoint
    // history. `history` already carries this turn's typed suffix.
    let transcript_history;
    let history_for_transcript: &[ChatMessage] = if (outcome.hit_cap
        || outcome.early_exit_tool.is_some())
        && !outcome.text.trim().is_empty()
    {
        transcript_history = {
            let mut messages = history.clone();
            messages.push(ChatMessage::assistant(outcome.text.clone()));
            messages
        };
        &transcript_history
    } else {
        history.as_slice()
    };
    persist_subagent_transcript(
        &workspace_dir,
        transcript_stem,
        agent_id,
        task_id,
        provider_label,
        model,
        history_for_transcript,
        &usage,
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
    ))
}

/// Build the sub-agent turn's [`TurnContextMiddleware`] from the live
/// `[context]` config and the agent's TokenJuice profile (#4466), mirroring the
/// chat path (`session/turn/core.rs`). Falls back to
/// [`TurnContextMiddleware::defaults`] when the config can't be loaded so a
/// config glitch degrades to the safe (byte-cap-only) behavior rather than
/// erroring the run.
fn build_subagent_context_mw(
    tokenjuice_compression: AgentTokenjuiceCompression,
    config: Option<&crate::openhuman::config::Config>,
) -> crate::openhuman::agent::tinyagents::TurnContextMiddleware {
    let mut mw = crate::openhuman::agent::tinyagents::TurnContextMiddleware::defaults();
    // Always thread the agent's compression profile — even on the config-default
    // path — so the definition's TokenJuice choice is honored.
    mw.tokenjuice_compression = tokenjuice_compression;
    match config {
        Some(config) => {
            let ctx = &config.context;
            // TokenJuice content-aware compaction gates on the same master
            // `[context].compaction_enabled` the chat path reads
            // (`ContextManager::compaction_enabled`).
            mw.tokenjuice_compaction_enabled = ctx.compaction_enabled;
            mw.tool_result_budget_bytes = ctx.tool_result_budget_bytes;
            // Microcompact keep-recent is `0` (disabled) unless microcompact is on.
            mw.microcompact_keep_recent = if ctx.microcompact_enabled {
                ctx.microcompact_keep_recent
            } else {
                0
            };
            // Summarization step honors the `[context].enabled` + autocompact
            // opt-outs, same as `ContextManager::autocompact_enabled`.
            mw.autocompact_enabled = ctx.enabled && ctx.autocompact_enabled;
            tracing::debug!(
                tokenjuice_compaction_enabled = mw.tokenjuice_compaction_enabled,
                compression = ?mw.tokenjuice_compression,
                tool_result_budget_bytes = mw.tool_result_budget_bytes,
                microcompact_keep_recent = mw.microcompact_keep_recent,
                autocompact_enabled = mw.autocompact_enabled,
                "[subagent_runner:graph] built sub-agent context middleware from config (#4466)"
            );
        }
        None => {
            tracing::debug!(
                "[subagent_runner:graph] no config available building sub-agent context mw; using defaults + compression profile"
            );
        }
    }
    mw
}

fn map_tinyagents_subagent_error(err: anyhow::Error) -> SubagentRunError {
    match err.downcast::<SubagentRunError>() {
        Ok(run_err) => run_err,
        Err(err) => SubagentRunError::Provider(err),
    }
}

/// Persist a sub-agent turn's raw transcript to `session_raw`, mirroring the
/// removed `SubagentObserver::persist_transcript`: `agent_type:"subagent"`, the
/// `task_id`, and the provider/model + usage carried on the last assistant
/// message so per-thread usage reads price the sub-agent at its own model.
#[allow(clippy::too_many_arguments)]
fn persist_subagent_transcript(
    workspace_dir: &std::path::Path,
    transcript_stem: &str,
    agent_id: &str,
    task_id: &str,
    provider_label: &str,
    model: &str,
    history: &[ChatMessage],
    usage: &AggregatedUsage,
    context_window: u64,
    dispatcher: &str,
    iteration: u32,
) {
    use crate::openhuman::agent::harness::session::transcript;

    let path = match transcript::resolve_keyed_transcript_path(workspace_dir, transcript_stem) {
        Ok(p) => p,
        Err(err) => {
            tracing::debug!(
                agent_id,
                error = %err,
                "[subagent_runner:graph] failed to resolve child transcript path"
            );
            return;
        }
    };
    let now = chrono::Utc::now().to_rfc3339();
    let turn_usage = transcript::TurnUsage {
        provider: provider_label.to_string(),
        model: model.to_string(),
        usage: transcript::MessageUsage {
            input: usage.input_tokens,
            output: usage.output_tokens,
            cached_input: usage.cached_input_tokens,
            context_window,
            cost_usd: usage.charged_amount_usd,
        },
        ts: now.clone(),
        reasoning_content: None,
        tool_calls: Vec::new(),
        iteration,
    };
    let meta = transcript::TranscriptMeta {
        agent_name: agent_id.to_string(),
        agent_id: Some(agent_id.to_string()),
        agent_type: Some("subagent".to_string()),
        dispatcher: dispatcher.into(),
        provider: Some(turn_usage.provider.clone()),
        model: Some(turn_usage.model.clone()),
        created: now.clone(),
        updated: now,
        turn_count: 1,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        charged_amount_usd: usage.charged_amount_usd,
        thread_id: crate::openhuman::agent::tinyagents::thread_context::current_thread_id(),
        task_id: Some(task_id.to_string()),
    };
    if let Err(err) = transcript::write_transcript(&path, history, &meta, Some(&turn_usage)) {
        tracing::debug!(
            agent_id,
            error = %err,
            "[subagent_runner:graph] failed to write child transcript"
        );
    }
}
