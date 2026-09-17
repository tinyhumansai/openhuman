//! Turn a completed `tinyagents` harness run into a
//! [`TinyagentsTurnOutcome`], for [`run_turn_via_tinyagents_shared`](super::run_turn_via_tinyagents_shared).
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
use crate::agent::tinyagents::tools::EarlyExitHook;
use crate::agent::tinyagents::turn_outcome::{
    HaltSummarySlot, TinyagentsTurnOutcome, ToolOutcomeSink,
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
    early_exit_hook: Option<EarlyExitHook>,
    halt_summary: &HaltSummarySlot,
    wrap_up_fired: &Option<Arc<std::sync::atomic::AtomicBool>>,
    tool_outcome_sink: &ToolOutcomeSink,
    request_base_len: usize,
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
        let records = mw.records();
        if !records.is_empty() {
            tracing::info!(
                model,
                compactions = records.len(),
                "[context] turn performed {} context compaction(s); surfacing provenance",
                records.len()
            );
            for (idx, record) in records.iter().enumerate() {
                let provenance = &record.provenance;
                tracing::info!(
                    model,
                    compaction = idx + 1,
                    of = records.len(),
                    source_count = provenance.source_ids.len(),
                    source_ids = ?provenance.source_ids,
                    from_tokens = provenance.original_token_estimate,
                    to_tokens = provenance.summary_token_estimate,
                    saved_tokens = provenance
                        .original_token_estimate
                        .saturating_sub(provenance.summary_token_estimate),
                    reason = %provenance.reason,
                    "[context] compaction provenance: folded {} source message(s) ({} -> {} tokens)",
                    provenance.source_ids.len(),
                    provenance.original_token_estimate,
                    provenance.summary_token_estimate,
                );
            }
        }
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

    // Terminal turn event (parity with the legacy engine's `progress::emit`): the
    // harness stream has no run-completed event, so emit `TurnCompleted` here with
    // the model-call count as the iteration total. Parent turns only; best-effort.
    // `turn_completed_sink` is `None` for sub-agent turns AND when the caller
    // opted to emit the terminal event itself after its post-run wrap-up
    // (`defer_turn_completed_to_caller`, #4457 defect C) — so this is the single
    // emission point for callers with no post-run streaming (channel/CLI).
    if let Some(sink) = &turn_completed_sink {
        // NOT best-effort. `TurnCompleted` is the web bridge's sole completion
        // signal: drop it and `parent_completed` stays false, so the bridge
        // marks a turn that actually finished as `interrupted` and never emits
        // `chat_done`. The turn's output still reaches the journal, session
        // transcript and memory tree, so the agent "remembers" replying while
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
            })
            .await
        {
            tracing::warn!(
                error = %err,
                "[tinyagents] TurnCompleted not delivered — progress receiver gone"
            );
        }
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

    let bridge_totals = bridge.as_ref().map(|bridge| bridge.totals_with_cost());
    let last_call_tokens = bridge.as_ref().map(|bridge| bridge.last_call_tokens());

    // Prefer the bridge's accumulated usage (per-call, authoritative — including
    // cached tokens and the estimated charged USD) when the observed path ran;
    // otherwise fall back to the run's aggregate totals and estimate the cost from
    // them so a fire-and-forget turn still reports a real (non-$0) cost.
    let (input_tokens, output_tokens, cached_input_tokens, charged_amount_usd) = bridge_totals
        .unwrap_or_else(|| {
            let input = run.usage.usage.input_tokens;
            let output = run.usage.usage.output_tokens;
            let cached = run.usage.usage.cache_read_tokens;
            let charged =
                crate::platform::cost::catalog::estimate_cost_usd(model, input, output, cached);
            crate::agent::tinyagents::turn_outcome::record_unobserved_turn_usage(
                model, input, output, cached, charged,
            );
            (input, output, cached, charged)
        });

    // An early-exit tool fired: the loop paused after its round. Surface the tool
    // name and use its captured question as the turn text (the paused assistant
    // turn carries the tool call, not a final answer) so the caller can
    // checkpoint and prompt the user — matching the legacy `early_exit_tool`.
    let early_exit = early_exit_hook.and_then(|hook| hook.take());

    // Cap detection: the harness sets `final_response` only when the loop
    // finishes naturally (the model stopped requesting tools). When the cap
    // pauser stops the loop mid-work, `final_response` stays `None` — that's the
    // cap hit. An early-exit is a clean pause and takes precedence; under
    // `pause_at_cap` the only other `Pause` source is the cap pauser, so this is
    // unambiguous. (`run_queue` steering injects messages, never pauses.)
    // The repeated-failure breaker halts the run with a root-cause summary instead
    // of a final model turn; surface it as the turn's text so the no-progress cause
    // reaches the caller/user rather than an empty reply.
    let breaker_halt = halt_summary.lock().ok().and_then(|mut s| s.take());

    // Issue #6014: the in-loop conclusion is the primary tell now. When
    // `FinalCallWrapUpMiddleware` fired, the turn reached its cap *and* answered
    // from inside the loop — which means it ended the way a finished turn does
    // (text, no tool request), so `final_response` is `Some` and the original
    // predicate below can no longer see it. The old predicate is kept as the
    // second arm rather than replaced: it still covers every run with the
    // middleware uninstalled, and the case where the concluding call itself came
    // back requesting a tool (a text-protocol model ignoring an empty schema
    // list) and the loop ran out with nothing final.
    let wrap_up_injected = wrap_up_fired
        .as_ref()
        .is_some_and(|fired| fired.load(std::sync::atomic::Ordering::SeqCst));
    let hit_cap = pause_at_cap
        && early_exit.is_none()
        && breaker_halt.is_none()
        && (wrap_up_injected
            || (run.model_calls >= max_iterations && run.final_response.is_none()));

    let (early_exit_tool, mut text) = match early_exit {
        Some(exit) => (Some(exit.tool), exit.question),
        None => (None, run.text().unwrap_or_default()),
    };

    // Carry the breaker halt onto the outcome so the sub-agent runner can report
    // `Incomplete` (#4466). `text` is overridden with the same root-cause summary
    // so callers with no breaker-awareness still surface the cause, not an empty
    // last-model reply.
    if let Some(summary) = &breaker_halt {
        tracing::info!(
            model,
            subagent = subagent_scope.is_some(),
            "[tinyagents] run halted by circuit breaker; surfacing as breaker_halt (#4466)"
        );
        text = summary.clone();
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

    TinyagentsTurnOutcome {
        text,
        history: crate::agent::message_convert::messages_to_history(&run.messages),
        conversation,
        model_calls: run.model_calls,
        tool_calls: run.tool_calls,
        input_tokens,
        output_tokens,
        cached_input_tokens,
        // Observed runs retain the final provider call separately from the
        // cumulative turn totals. For unobserved single-call runs the aggregate
        // is exact; multi-call aggregates are not valid occupancy readings.
        last_call_input_tokens: last_call_tokens
            .map(|tokens| tokens.0)
            .or_else(|| (run.model_calls == 1).then_some(input_tokens))
            .unwrap_or(0),
        last_call_output_tokens: last_call_tokens
            .map(|tokens| tokens.1)
            .or_else(|| (run.model_calls == 1).then_some(output_tokens))
            .unwrap_or(0),
        charged_amount_usd,
        early_exit_tool,
        hit_cap,
        wrap_up_injected,
        breaker_halt,
        tool_outcomes,
    }
}
