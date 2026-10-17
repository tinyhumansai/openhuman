//! Turn a completed `tinyagents` harness run into a
//! [`TinyagentsTurnOutcome`], for [`TurnSpec::run`](super::TurnSpec::run).
//!
//! Split out of the runner: everything here executes only on the success path
//! (after the drive future returns `Ok`), covering journal completion,
//! compaction/cache-layout diagnostics, the terminal `TurnCompleted` event,
//! and assembling the outcome's usage/cap/breaker fields.

use std::sync::Arc;

use tinyagents_harness::middleware::AgentRun;
use tinyagents_harness::middleware::{ContextCompressionMiddleware, PromptCacheGuardMiddleware};
use tokio::sync::mpsc::Sender;

use crate::agent::progress::AgentProgress;
use crate::agent::tinyagents::journal::TurnJournal;
use crate::agent::tinyagents::observability::{self, OpenhumanEventBridge, SubagentScope};
use crate::agent::tinyagents::turn_outcome::TinyagentsTurnOutcome;
use tinyagents_harness::turn_outcome::{HaltSummarySlot, ToolOutcomeSink};
use tinyagents_harness::turn_run_finalize::{
    classify_turn_end, log_compaction_provenance, select_turn_text, system_segment_bytes, TurnEnd,
    TurnEndInputs,
};

/// Assemble the [`TinyagentsTurnOutcome`] for a run that returned
/// successfully: stamp the durable journal's completed status, surface
/// compaction/prompt-cache diagnostics, emit the terminal `TurnCompleted`
/// event (parent turns only), and resolve the turn's usage/cap/breaker
/// fields.
#[allow(clippy::too_many_arguments)]
pub(super) async fn finalize_turn_outcome(
    run: AgentRun,
    model: &str,
    max_iterations: usize,
    subagent_scope: &Option<SubagentScope>,
    pause_at_cap: bool,
    turn_journal: Option<&TurnJournal>,
    compression_mw: Option<&Arc<ContextCompressionMiddleware>>,
    prompt_cache_guard: &PromptCacheGuardMiddleware,
    turn_completed_sink: Option<Sender<AgentProgress>>,
    bridge: Option<Arc<OpenhumanEventBridge>>,
    early_exit: Option<tinyagents_harness::tool::EarlyExit>,
    halt_summary: &HaltSummarySlot,
    wrap_up_fired: &Option<Arc<tinyagents_harness::middleware::FinalCallWrapUpMiddleware>>,
    run_instance_id: u64,
    tool_outcome_sink: &ToolOutcomeSink,
    resolved_route: Option<tinyinference_llm::model::ResolvedModelRoute>,
    request_base_len: usize,
    dispatch: &crate::core::runtime::DispatchCtx,
) -> TinyagentsTurnOutcome {
    // Durable journal: the harness returned a transcript, so stamp the terminal
    // completed status (best-effort, non-fatal). The event stream carries no
    // run-terminal event, so this caller-driven write is authoritative.
    if let Some(journal) = turn_journal {
        journal.finish_completed().await;
    }
    // Context-compression provenance (issue #4249, 03.1 item 6): the harness's
    // `AgentEvent::Compressed` projection only carries token deltas, so drain the
    // compression middleware's `records()` here — each carries the full
    // `CompressionProvenance` (source ids + before/after token estimates + policy
    // reason) built by `ModelSummarizer`. Surfaced at info with a
    // grep-friendly `[context]` prefix so every compaction is auditable, not just
    // its net token saving.
    if let Some(mw) = compression_mw {
        log_compaction_provenance(model, &mw.records());
    }

    // Prompt-cache layout diagnostics (issue #4249, 03.2): drain the crate
    // `PromptCacheGuardMiddleware`'s recorded `CacheLayoutEvent`s and surface each
    // as a structured `[cache]` warning. Fires only when the cacheable prompt
    // prefix (system prompt + tool set) changed across model calls — i.e. volatile
    // content silently busting the provider KV-cache prefix. This is now the sole
    // owner of KV-cache-prefix drift detection: the warn-only
    // `CacheAlignMiddleware` was deleted in C3.
    let cache_layout_events = prompt_cache_guard.layout_events();
    if !cache_layout_events.is_empty() {
        tracing::debug!(
            model,
            events = cache_layout_events.len(),
            "[cache] surfacing prompt-cache layout change events"
        );
        observability::surface_cache_layout_events(model, &cache_layout_events);
    }

    // Response-cache effectiveness for this turn (issue #4249, 03.2). Additive —
    // logged with a grep-friendly `[cache]` prefix here; wiring the counts into the
    // cost-footer DTO is a follow-up coordinated with workstream 06. Only the
    // observed (bridge) path accumulates these; deterministic internal runs that
    // attach a `ResponseCache` are where non-zero counts appear.
    if let Some(bridge) = &bridge {
        let (cache_hits, cache_misses) = bridge.cache_counts();
        if cache_hits > 0 || cache_misses > 0 {
            tracing::debug!(
                model,
                cache_hits,
                cache_misses,
                "[cache] turn response-cache summary"
            );
        }
    }

    let bridge_totals = bridge.map(|bridge| bridge.totals_with_cost());

    // Prefer the bridge's accumulated usage (per-call, authoritative — including
    // cached tokens and the estimated charged USD) when the observed path ran;
    // otherwise fall back to the run's aggregate totals and estimate the cost from
    // them so a fire-and-forget turn still reports a real (non-$0) cost.
    let (input_tokens, output_tokens, cached_input_tokens, cost) =
        bridge_totals.unwrap_or_else(|| {
            let input = run.usage.usage.input_tokens;
            let output = run.usage.usage.output_tokens;
            let cached = run.usage.usage.cache_read_tokens;
            // No per-call charges on this path: the catalog estimate when the
            // model has a list price, else the turn's cost is unknown.
            let estimate = crate::agent::cost::estimate_call_cost_usd(
                model,
                &crate::inference::provider::BilledUsage::from_counts(input, output)
                    .with_cached_input_tokens(cached),
            );
            crate::agent::tinyagents::turn_outcome::record_unobserved_turn_usage(
                dispatch, model, input, output, cached, estimate,
            );
            let mut cost = tinyagents_harness::cost::CostTally::default();
            cost.add(match estimate {
                Some(usd) => tinyagents_harness::cost::CallCost::Estimated(usd),
                None => tinyagents_harness::cost::CallCost::Unknown,
            });
            (input, output, cached, cost)
        });

    // An early-exit tool fired: the loop paused after its round. Surface the tool
    // name and use its captured question as the turn text (the paused assistant
    // turn carries the tool call, not a final answer) so the caller can
    // checkpoint and prompt the user — matching the legacy `early_exit_tool`.

    // The repeated-failure breaker halts the run with a root-cause summary
    // instead of a final model turn; surface it as the turn's text so the
    // no-progress cause reaches the caller/user rather than an empty reply.
    let breaker_halt = halt_summary.lock().ok().and_then(|mut s| s.take());
    let wrap_up_injected = wrap_up_fired
        .as_ref()
        .is_some_and(|mw| mw.fired_for(run_instance_id));
    // Cap, wrap-up and truncation classification is the harness's
    // (`classify_turn_end`, #6014 / #6951).
    let TurnEnd { hit_cap, truncated } = classify_turn_end(
        &run,
        &TurnEndInputs {
            max_iterations,
            pause_at_cap,
            early_exit: early_exit.is_some(),
            breaker_halt: breaker_halt.as_deref(),
            wrap_up_injected,
        },
    );

    // Terminal turn event (parity with the legacy engine's `progress::emit`): the
    // harness stream has no run-completed event, so emit `TurnCompleted` here with
    // the model-call count as the iteration total. Parent turns only; best-effort.
    // `turn_completed_sink` is `None` for sub-agent turns AND when the caller
    // opted to emit the terminal event itself after its post-run wrap-up
    // (`defer_turn_completed_to_caller`, #4457 defect C) — so this is the single
    // emission point for callers with no post-run streaming (channel/CLI).
    // A run the harness stopped (breaker / cap) is traced as stopped, not as a
    // clean completion. No deadline wind-down on this path: only the session
    // turn carries one, and it records the stop through its sidecar.
    let stop =
        crate::agent::turn_projection::classify_stop(breaker_halt.as_deref(), false, hit_cap);
    if let Some(stop) = stop.as_ref().filter(|_| turn_completed_sink.is_some()) {
        tracing::debug!(
            model,
            "[tinyagents] turn stopped early; carrying it on TurnCompleted {}",
            stop.status_message()
        );
    }
    if let Some(sink) = &turn_completed_sink {
        // NOT best-effort. `TurnCompleted` is the web bridge's sole completion
        // signal: drop it and `parent_completed` stays false, so the bridge
        // marks a turn that actually finished as `interrupted` and never emits
        // `chat_done`. The turn's output still reaches the journal, session
        // transcript and memory, so the agent "remembers" replying while
        // the user's thread shows silence. A heavy turn (many tools + long
        // streaming) reliably fills the 256-slot channel, which is why only
        // tool-heavy turns were affected.
        //
        // Blocking is safe *here specifically*: this site is guarded by
        // `subagent_scope.is_none()`, so it only ever runs on a parent turn
        // with nothing awaiting it. The sub-agent stall documented on
        // `tool_progress::emit` comes from parking a *sub-agent's* loop while
        // the orchestrator awaits its tool call — unreachable from this path.
        // Deltas and sub-agent lifecycle events stay lossy via `emit`.
        if let Err(err) = sink
            .send(AgentProgress::TurnCompleted {
                iterations: run.model_calls as u32,
                stop,
            })
            .await
        {
            tracing::warn!(
                error = %err,
                "[tinyagents] TurnCompleted not delivered — progress receiver gone"
            );
        }
    }

    let early_exit_tool = early_exit.as_ref().map(|exit| exit.tool.clone());
    let text = select_turn_text(
        &run,
        early_exit.map(|exit| exit.question),
        breaker_halt.as_deref(),
    );
    // Carry the breaker halt onto the outcome so the sub-agent runner can report
    // `Incomplete` (#4466); `text` already carries the same summary.
    if breaker_halt.is_some() {
        tracing::info!(
            model,
            subagent = subagent_scope.is_some(),
            "[tinyagents] run halted by circuit breaker; surfacing as breaker_halt (#4466)"
        );
    }

    // #6951: the closing call names the exhausted output budget, not "finished".
    if truncated {
        tracing::warn!(
            model,
            model_calls = run.model_calls,
            tool_calls = run.tool_calls,
            "[tinyagents] turn ended on a reply that ran out of output tokens while reasoning"
        );
    }

    let tool_outcomes = tool_outcome_sink
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();

    let conversation = crate::agent::message_convert::messages_to_conversation(
        crate::agent::message_convert::messages_since_request(&run.messages, request_base_len),
    );
    tracing::debug!(
        model,
        request_base_len,
        transcript_len = run.messages.len(),
        persisted_messages = run.messages.len().saturating_sub(request_base_len),
        subagent = subagent_scope.is_some(),
        "[tinyagents] persisting post-request transcript (shared path; steer-safe boundary)"
    );
    // Prompt-size and cache summary for the turn (grep `turn prompt summary`):
    // the frozen system prefix in bytes, per tier segment, beside the token
    // counts the provider reported. `cached_input_tokens` rising to roughly
    // the prefix on turn 2 is what a working prefix cache looks like.
    let system_segment_bytes = system_segment_bytes(&run.messages);
    tracing::info!(
        model,
        system_segments = system_segment_bytes.len(),
        system_bytes = system_segment_bytes.iter().sum::<usize>(),
        ?system_segment_bytes,
        model_calls = run.model_calls,
        tool_calls = run.tool_calls,
        input_tokens,
        cached_input_tokens,
        output_tokens,
        "[tinyagents] turn prompt summary"
    );

    // The run's compaction (`AgentRun::compacted_history`), carried to the
    // session driver so the persisted history starts from the checkpoint.
    let compaction = run
        .compacted_history
        .as_deref()
        .and_then(tinyagents_harness::summarization::CompactionCarry::from_compacted_history);
    if let Some(carry) = &compaction {
        tracing::info!(
            model,
            kept_tail = carry.kept_tail,
            transcript_len = run.messages.len(),
            "[tinyagents] turn compacted; the persisted history will start from the checkpoint"
        );
    }

    TinyagentsTurnOutcome {
        text,
        resolved_route,
        history: crate::agent::message_convert::messages_to_history(&run.messages),
        conversation,
        model_calls: run.model_calls,
        tool_calls: run.tool_calls,
        input_tokens,
        output_tokens,
        cached_input_tokens,
        cost,
        early_exit_tool,
        hit_cap,
        wrap_up_injected,
        breaker_halt,
        truncated,
        tool_outcomes,
        compaction,
    }
}
