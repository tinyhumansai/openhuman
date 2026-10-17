//! Journal-backed span projection (C4 slice S2).
//!
//! Reconstructs a run's [`AgentProgress`] stream from the durable
//! [`AgentObservation`] journal (the crate `AgentEvent` record) and folds it
//! through the existing [`SpanCollector`], so trace spans no longer require the
//! *live* in-run `AgentProgress` side-observer
//! (`web_chat/progress_bridge.rs`). A UI/supervisor can attach
//! after a run, read the journal, and rebuild identical spans.
//!
//! This is deliberately built on `SpanCollector` (not a re-derivation) so span
//! *shape* parity holds by construction for every `AgentProgress` the journal
//! can produce. The one-way mapping here mirrors `OpenhumanEventBridge`
//! (`tinyagents/observability.rs`) but is **pure** — it depends only on the
//! journalled event, made possible by the crate carrying tool outcome
//! (`duration_ms`/`output_bytes`/`error`) on `ToolCompleted` (tinyagents#18).
//!
//! Known parity gaps:
//! - **Cost is an estimate, not the charge.** The provider's charged USD
//!   reaches the live path through the `usage_carry` side-channel, which is not
//!   an `AgentEvent` and is not journalled, so both the per-call
//!   `ModelCallCompleted.cost_usd` (`0` here — the generation span's cost is
//!   sourced from the persisted per-run cost store at export time) and the
//!   turn roll-up folded from `UsageRecorded` are journal-only figures. Token
//!   counts are exact; treat a projected `gen_ai.usage.cost_usd` as an
//!   estimate. `AgentEvent::CostRecorded` would close this if the crate ever
//!   emits it.
//! - **No sub-agent spans are projected at all** (openhuman#6419). The four
//!   `AgentEvent::SubAgent*` arms below are dead in production: a survey of 900
//!   real run journals found **zero** sub-agent events of any kind, and every
//!   journalled run is its own root (`root_run_id == run_id` in all 588
//!   `run_started` records sampled, and no `root_run_id` spans more than one
//!   run file), so a child run is neither recorded in its parent's journal nor
//!   reachable from it. A replayed delegating turn therefore loses the whole
//!   sub-agent subtree — the delegate's turn span, its iterations, and its tool
//!   and model spans — which is the bulk of the span-count divergence the
//!   `[agent-tracing][journal-shadow]` parity check reports. This is a gap in
//!   what the crate journals, not in the projection: the arms are kept so the
//!   projection is correct the moment the events exist.
//!
//!   An earlier revision of this list said sub-agent spans "carry
//!   lifecycle/timing and child tool/model structure but empty delegated
//!   prompt/final output", i.e. that only their *content* was thin. That was
//!   wrong in kind and stopped readers looking: there are no such spans to
//!   carry anything.
//! - `AgentProgress::SubagentAwaitingUser` has no journal source either (the
//!   crate emits no matching lifecycle event). Moot while the gap above stands,
//!   and listed separately because it survives it: even once sub-agent
//!   lifecycle events are journalled, a span parked on a user prompt would lose
//!   that attribute on replay.
//!
//! Everything else **that the crate journals** is projected. In particular the
//! match in
//! [`observation_to_progress`] is **exhaustive over `AgentEvent`** — a crate
//! that adds a span-bearing event breaks the build here rather than silently
//! diverging, which is exactly how the missing `UsageRecorded` roll-up went
//! unnoticed (openhuman#6148).

use tinyagents_harness::events::AgentEvent;
use tinyagents_harness::observability::AgentObservation;

use super::SpanCollector;
use crate::agent::progress::AgentProgress;
use replay::{json_content_text, turn_outcome_for_failure, user_message_text, Replay};

#[path = "journal_replay.rs"]
mod replay;
use tinyagents_harness::observability::trace_export::{TraceContext, TraceSpan};
use tinytools::status::classify;

/// Mutable state threaded across a single run's observations while replaying.
#[derive(Default)]
struct ReplayState {
    /// 1-based iteration index, bumped once per `ModelStarted` — the same
    /// attribution the live `IterationCursor` provides.
    iteration: u32,
    /// Max iterations configured for the turn (carried onto iteration spans).
    max_iterations: u32,
    /// `call_id → model name`, learned from `ModelStarted` so the matching
    /// `ModelCompleted` can name its generation span (the crate `ModelCompleted`
    /// event carries no model name).
    models: std::collections::HashMap<String, String>,
    /// Model of the most recent top-level `ModelStarted`. `UsageRecorded`
    /// carries no `call_id`, so this stands in for the live bridge's
    /// per-run `self.model` when naming the turn's cost roll-up.
    model: String,
    /// Stack of currently-open sub-agent runs. The crate lifecycle event only
    /// carries name/depth; ordered replay brackets child model/tool events.
    subagents: Vec<ReplaySubagent>,
    /// Monotonic suffix to make repeated invocations of the same child name
    /// distinct in the span tree.
    next_subagent_seq: u64,
    /// Top-level iterations whose `UsageRecorded` has already been folded in.
    /// Mirrors the live bridge's `recorded_iterations` dedupe guard — see the
    /// `UsageRecorded` arm.
    recorded_iterations: std::collections::HashSet<u32>,
    /// Cumulative top-level usage, folded exactly as the live bridge folds it
    /// so the roll-up carried by `TurnCostUpdated` is a running total.
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    /// Cumulative estimated cost. The provider's *charged* amount rides the
    /// un-journalled `usage_carry` side-channel, so this is an estimate — see
    /// the module header.
    cost_usd: f64,
    /// Model of the most recent `ModelStarted` in any scope: the name for a
    /// `ModelCompleted` whose own `ModelStarted` is missing from the journal.
    last_model_any: String,
    /// Collector steps produced by the current observation that must run
    /// before its returned progress (see [`Replay`]). Drained per observation.
    pre: Vec<Replay>,
    /// `call_id → requested tool name` for calls the crate answered as an
    /// unknown tool. The crate's own `ToolStarted`/`ToolCompleted` pair for the
    /// same call id follows; this lets it carry the "unavailable" label and the
    /// `NotFound` class. Mirrors the live bridge's `unknown_calls`.
    unknown_calls: std::collections::HashMap<String, String>,
}

#[derive(Clone)]
struct ReplaySubagent {
    agent_id: String,
    task_id: String,
    depth: usize,
    iteration: u32,
    started_ts_ms: u64,
}

impl ReplayState {
    fn active_subagent(&self) -> Option<&ReplaySubagent> {
        self.subagents.last()
    }

    fn active_subagent_mut(&mut self) -> Option<&mut ReplaySubagent> {
        self.subagents.last_mut()
    }

    /// The "unavailable" label/detail for a call recorded as an unknown tool.
    fn unknown_tool_display(&self, call_id: &str) -> (Option<String>, Option<String>) {
        match self.unknown_calls.get(call_id) {
            Some(requested) => (
                Some(format!(
                    "{} (unavailable)",
                    tinytools::humanize_tool_name(requested)
                )),
                Some("tool not available".to_string()),
            ),
            None => (None, None),
        }
    }
}

/// Maps one journalled observation to zero or more [`AgentProgress`] events,
/// updating `state`. Non-span-bearing events (deltas, budget/cache/steering
/// diagnostics) map to nothing — `SpanCollector` ignores them anyway.
fn observation_to_progress(obs: &AgentObservation, state: &mut ReplayState) -> Vec<AgentProgress> {
    let event = &obs.event;
    match event {
        AgentEvent::RunStarted { .. } => {
            if state.active_subagent().is_some() {
                Vec::new()
            } else {
                vec![AgentProgress::TurnStarted]
            }
        }

        AgentEvent::ModelStarted { call_id, model } => {
            let iteration = match state.active_subagent_mut() {
                Some(scope) => {
                    scope.iteration += 1;
                    scope.iteration
                }
                None => {
                    state.iteration += 1;
                    // `UsageRecorded` carries no `call_id`; the live bridge names
                    // the roll-up with its per-run model, and the last top-level
                    // `ModelStarted` is the journal's equivalent. Only top-level
                    // calls are recorded — a child's model must not name the
                    // parent's turn.
                    state.model = model.clone();
                    state.iteration
                }
            };
            state
                .models
                .insert(call_id.as_str().to_string(), model.clone());
            state.last_model_any = model.clone();
            match state.active_subagent() {
                Some(scope) => vec![AgentProgress::SubagentIterationStarted {
                    agent_id: scope.agent_id.clone(),
                    task_id: scope.task_id.clone(),
                    iteration,
                    max_iterations: state.max_iterations,
                    extended_policy: false,
                }],
                None => vec![AgentProgress::IterationStarted {
                    iteration,
                    max_iterations: state.max_iterations,
                }],
            }
        }

        // The harness answers `tool_search` without running a tool, so the
        // journal carries no `ToolStarted`/`ToolCompleted` for it. Replay the
        // same synthetic pair the live bridge emits, so a replayed trace has
        // the `tool.tool_search` span with the ranking facts.
        AgentEvent::ToolSearched {
            call_id,
            query,
            matched,
            ranker,
            top_confidence,
            fallback,
            shadow_matched,
            latency_ms,
        } => {
            let tool_name = tinyagents_harness::tool::discover::TOOL_SEARCH_NAME.to_string();
            let arguments = serde_json::json!({ "query": query });
            let output = serde_json::json!({
                "matched": matched,
                "ranker": ranker,
                "top_confidence": top_confidence,
                "fallback": fallback,
                "shadow_matched": shadow_matched,
                "latency_ms": latency_ms,
            })
            .to_string();
            let output_chars = output.chars().count();
            match state.active_subagent() {
                Some(scope) => vec![
                    AgentProgress::SubagentToolCallStarted {
                        agent_id: scope.agent_id.clone(),
                        task_id: scope.task_id.clone(),
                        call_id: call_id.as_str().to_string(),
                        tool_name: tool_name.clone(),
                        arguments: arguments.clone(),
                        iteration: scope.iteration,
                        display_label: Some("Searching tools".to_string()),
                        display_detail: None,
                    },
                    AgentProgress::SubagentToolCallCompleted {
                        agent_id: scope.agent_id.clone(),
                        task_id: scope.task_id.clone(),
                        call_id: call_id.as_str().to_string(),
                        tool_name,
                        success: true,
                        output_chars,
                        output,
                        arguments: Some(arguments),
                        elapsed_ms: *latency_ms,
                        iteration: scope.iteration,
                        failure: None,
                        display_label: Some("Searching tools".to_string()),
                        display_detail: None,
                        structured: None,
                    },
                ],
                None => vec![
                    AgentProgress::ToolCallStarted {
                        call_id: call_id.as_str().to_string(),
                        tool_name: tool_name.clone(),
                        arguments: arguments.clone(),
                        iteration: state.iteration,
                        display_label: Some("Searching tools".to_string()),
                        display_detail: None,
                    },
                    AgentProgress::ToolCallCompleted {
                        call_id: call_id.as_str().to_string(),
                        tool_name,
                        success: true,
                        output_chars,
                        output,
                        arguments: Some(arguments),
                        elapsed_ms: *latency_ms,
                        iteration: state.iteration,
                        failure: None,
                        display_label: Some("Searching tools".to_string()),
                        display_detail: None,
                        structured: None,
                    },
                ],
            }
        }

        AgentEvent::ToolStarted {
            call_id, tool_name, ..
        } => {
            let (display_label, display_detail) = state.unknown_tool_display(call_id.as_str());
            match state.active_subagent() {
                Some(scope) => vec![AgentProgress::SubagentToolCallStarted {
                    agent_id: scope.agent_id.clone(),
                    task_id: scope.task_id.clone(),
                    call_id: call_id.as_str().to_string(),
                    tool_name: tool_name.clone(),
                    arguments: serde_json::Value::Null,
                    iteration: scope.iteration,
                    display_label,
                    display_detail,
                }],
                None => vec![AgentProgress::ToolCallStarted {
                    call_id: call_id.as_str().to_string(),
                    tool_name: tool_name.clone(),
                    // The journal does not carry the model's raw argument JSON in
                    // payload-free mode; the tool span still renders from name + id.
                    arguments: serde_json::Value::Null,
                    iteration: state.iteration,
                    display_label,
                    display_detail,
                }],
            }
        }

        AgentEvent::ToolCompleted {
            call_id,
            tool_name,
            input,
            output,
            duration_ms,
            output_bytes,
            error,
            ..
        } => {
            // Outcome now rides the event (tinyagents#18): success, duration and
            // size are self-describing, and the same `classify` the live path
            // uses reproduces the identical `ClassifiedFailure` from the
            // journalled error string. `output` is present only when the run
            // captured payloads (full-content journals).
            let (display_label, display_detail) = state.unknown_tool_display(call_id.as_str());
            let was_unknown = state.unknown_calls.remove(call_id.as_str()).is_some();
            // An unknown tool is `NotFound` from the typed event (#6277),
            // whatever the text classifier made of the echoed names.
            let failure = if was_unknown {
                Some(tinytools::status::describe(
                    tinytools::status::ToolFailureClass::NotFound,
                ))
            } else {
                error.as_ref().map(|text| classify(text, false))
            };
            let success = error.is_none() && !was_unknown;
            let output_text = match output {
                Some(serde_json::Value::String(text)) => text.clone(),
                Some(value) => value.to_string(),
                None => String::new(),
            };
            match state.active_subagent() {
                Some(scope) => vec![AgentProgress::SubagentToolCallCompleted {
                    agent_id: scope.agent_id.clone(),
                    task_id: scope.task_id.clone(),
                    call_id: call_id.as_str().to_string(),
                    tool_name: tool_name.clone(),
                    success,
                    output_chars: output_bytes.unwrap_or(0) as usize,
                    output: output_text,
                    arguments: input.clone(),
                    elapsed_ms: duration_ms.unwrap_or(0),
                    iteration: scope.iteration,
                    failure,
                    // The journal has no live tool registry to recompute a
                    // real label/detail from, and no `ToolResult.metadata` to
                    // replay structured payloads from.
                    display_label,
                    display_detail,
                    structured: None,
                }],
                None => vec![AgentProgress::ToolCallCompleted {
                    call_id: call_id.as_str().to_string(),
                    tool_name: tool_name.clone(),
                    success,
                    output_chars: output_bytes.unwrap_or(0) as usize,
                    output: output_text,
                    arguments: input.clone(),
                    elapsed_ms: duration_ms.unwrap_or(0),
                    iteration: state.iteration,
                    failure,
                    display_label,
                    display_detail,
                    structured: None,
                }],
            }
        }

        AgentEvent::UnknownToolCall {
            call_id,
            requested_name,
            recovery,
            ..
        } => {
            // #4118 synthesised a failed Started/Completed pair here because
            // the crate emitted none for an unknown tool. Since TOOL-11 the
            // crate answers it through `recover_tool_call`, which journals an
            // ordinary `ToolStarted`/`ToolCompleted` pair under the same call
            // id, so a synthesised pair doubled the tool span. Record the call
            // so that pair carries the "unavailable" label and the `NotFound`
            // class instead. Mirrors `observability/event_projection.rs`. A
            // `rewrite:` recovery runs its target under its own name.
            if !recovery.starts_with("rewrite:") {
                state
                    .unknown_calls
                    .insert(call_id.as_str().to_string(), requested_name.clone());
            }
            Vec::new()
        }

        AgentEvent::ModelCompleted {
            call_id,
            started_at_ms,
            usage,
            input,
            output,
        } => {
            let usage = usage.unwrap_or_default();
            let start = started_at_ms.unwrap_or(obs.ts_ms).min(obs.ts_ms);
            let model = match state.models.remove(call_id.as_str()) {
                Some(model) => model,
                None => {
                    // The harness emits one `ModelStarted` per `ModelCompleted`,
                    // so a completion with no start means the journal lost the
                    // start. Without one the call joined the previous iteration,
                    // shared its start instant and carried no model name (seen
                    // in production as several `llm.` generations starting in
                    // the same millisecond). Open the iteration it belongs to at
                    // the call's own start instead.
                    log::debug!(
                        "[agent-tracing][journal] ModelCompleted without ModelStarted \
                         call_id={} run_id={}; synthesizing its iteration",
                        call_id.as_str(),
                        obs.run_id.as_str()
                    );
                    let iteration = match state.active_subagent_mut() {
                        Some(scope) => {
                            scope.iteration += 1;
                            scope.iteration
                        }
                        None => {
                            state.iteration += 1;
                            state.iteration
                        }
                    };
                    let started = match state.active_subagent() {
                        Some(scope) => AgentProgress::SubagentIterationStarted {
                            agent_id: scope.agent_id.clone(),
                            task_id: scope.task_id.clone(),
                            iteration,
                            max_iterations: state.max_iterations,
                            extended_policy: false,
                        },
                        None => AgentProgress::IterationStarted {
                            iteration,
                            max_iterations: state.max_iterations,
                        },
                    };
                    state.pre.push(Replay::At(started, start));
                    state.last_model_any.clone()
                }
            };
            let scope = state.active_subagent().cloned();
            state.pre.push(Replay::CallStart(
                scope.as_ref().map(|s| s.task_id.clone()),
                start,
            ));
            let iteration = scope
                .as_ref()
                .map(|s| s.iteration)
                .unwrap_or(state.iteration);
            let mut progress = vec![AgentProgress::ModelCallCompleted {
                model,
                // Provider qualification/cost are filled at export time from the
                // persisted cost store, not the journal (§2a).
                provider_id: String::new(),
                subagent_task_id: scope.as_ref().map(|s| s.task_id.clone()),
                input: input.clone(),
                output: output.clone(),
                iteration,
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_input_tokens: usage.cache_read_tokens,
                // The crate `Usage` DOES carry cache-creation tokens; this used
                // to be hardcoded `0`, which dropped the
                // `gen_ai.usage.cache_creation_tokens` attribute from every
                // projected generation span (`record_model_call` inserts it only
                // when `> 0`) and so diverged from live whenever a provider
                // reported a cache write.
                cache_creation_tokens: usage.cache_creation_tokens,
                reasoning_tokens: usage.reasoning_tokens,
                cost_usd: None,
            }];
            if scope.is_none() {
                // The request's last user message is the user's own words on
                // the turn's first call; the collector keeps only the first
                // turn input, so later calls (whose last user message is tool
                // results or a harness nudge) do not replace it.
                let turn_input = input.as_ref().map(user_message_text);
                let turn_output = output.as_ref().map(json_content_text);
                if turn_input.is_some() || turn_output.is_some() {
                    progress.push(AgentProgress::TurnContent {
                        input: turn_input,
                        output: turn_output,
                    });
                }
            }
            progress
        }

        AgentEvent::UsageRecorded { usage } => {
            // The cost footer is a top-level surface: the live bridge suppresses
            // the per-child roll-up (`observability/event_bridge.rs`, `self.scope`
            // guard), and a child run's usage is accounted separately. Skipping
            // entirely — rather than accumulating silently — is what keeps the
            // projected parent total equal to the live one, because live the
            // parent and the child are *different* bridge instances with
            // different accumulators, while the journal interleaves both runs
            // into one observation stream.
            if state.active_subagent().is_some() {
                return Vec::new();
            }
            // Dedupe guard, mirroring the live bridge's (W2-budget-dedupe): the
            // observe-only crate `BudgetMiddleware` makes each model call emit —
            // and therefore journal — TWO `UsageRecorded` events with identical
            // usage and *distinct* event ids, so an event-id key would not
            // collapse them. Key on the run-scoped model-call identity instead:
            // the iteration cursor, bumped once per `ModelStarted`. Without this
            // every projected total would be double the live one.
            if !state.recorded_iterations.insert(state.iteration) {
                return Vec::new();
            }
            // The provider's *charged* amount reaches the live path through the
            // `usage_carry` side-channel, which is not an `AgentEvent` and is not
            // journalled (§2a of the C4 parity plan), so the projection prices
            // the call with the same estimator the live path uses as its floor.
            // The token counts are exact; `cost_usd` is an estimate.
            // An unpriced model adds nothing rather than a placeholder rate.
            state.cost_usd += crate::agent::cost::estimate_call_cost_usd(
                &state.model,
                &crate::inference::provider::BilledUsage::from_counts(
                    usage.input_tokens,
                    usage.output_tokens,
                )
                .with_cached_input_tokens(usage.cache_read_tokens)
                .with_cache_creation_tokens(usage.cache_creation_tokens)
                .with_reasoning_tokens(usage.reasoning_tokens),
            )
            .unwrap_or(0.0);
            state.input_tokens += usage.input_tokens;
            state.output_tokens += usage.output_tokens;
            state.cached_input_tokens += usage.cache_read_tokens;
            vec![AgentProgress::TurnCostUpdated {
                model: state.model.clone(),
                iteration: state.iteration,
                input_tokens: state.input_tokens,
                output_tokens: state.output_tokens,
                cached_input_tokens: state.cached_input_tokens,
                total_usd: state.cost_usd,
            }]
        }

        AgentEvent::SubAgentStarted { name, depth } => {
            state.next_subagent_seq += 1;
            let task_id = format!("{name}-d{depth}-{}", state.next_subagent_seq);
            state.subagents.push(ReplaySubagent {
                agent_id: name.clone(),
                task_id: task_id.clone(),
                depth: *depth,
                iteration: 0,
                started_ts_ms: obs.ts_ms,
            });
            vec![AgentProgress::SubagentSpawned {
                agent_id: name.clone(),
                task_id,
                mode: "typed".to_string(),
                dedicated_thread: false,
                prompt_chars: 0,
                worker_thread_id: None,
                display_name: Some(name.clone()),
                prompt: String::new(),
                parent_call_id: None,
            }]
        }

        AgentEvent::SubAgentCompleted { name, depth } => {
            let pos = state
                .subagents
                .iter()
                .rposition(|scope| scope.agent_id == *name && scope.depth == *depth);
            let Some(scope) = pos.map(|index| state.subagents.remove(index)) else {
                return Vec::new();
            };
            vec![AgentProgress::SubagentCompleted {
                agent_id: scope.agent_id,
                task_id: scope.task_id,
                elapsed_ms: obs.ts_ms.saturating_sub(scope.started_ts_ms),
                iterations: scope.iteration,
                output_chars: 0,
                output: String::new(),
                // Projection rebuild, not an originating emit; it has no
                // outcome to read usage from. See the field's docs.
                usage: None,
                worktree_path: None,
                changed_files: Vec::new(),
                dirty_status: None,
                stop: None,
            }]
        }

        AgentEvent::RunCompleted { .. } => {
            if state.active_subagent().is_some() {
                Vec::new()
            } else {
                vec![AgentProgress::TurnCompleted {
                    iterations: state.iteration,
                    stop: None,
                }]
            }
        }

        AgentEvent::RunFailed { error, outcome, .. } => {
            let Some(scope) = state.subagents.pop() else {
                // The top-level run failed: the turn span reports it instead of
                // closing as an unremarkable, status-less span.
                state
                    .pre
                    .push(Replay::Outcome(turn_outcome_for_failure(error, outcome.as_ref())));
                return Vec::new();
            };
            vec![AgentProgress::SubagentFailed {
                agent_id: scope.agent_id,
                task_id: scope.task_id,
                error: error.clone(),
            }]
        }

        // Everything below carries no span data. This is written out rather than
        // left as a `_ =>` catch-all on purpose: a wildcard is what let the
        // missing `UsageRecorded` arm above sit here silently, diverging from
        // live on every turn with nothing to catch it. With the match
        // exhaustive, the next `AgentEvent` the crate adds is a compile error
        // here — a decision to make, not a gap to discover in a log.
        //
        // Streaming/incremental progress. `SpanCollector` folds deltas into no
        // span (`progress_tracing.rs`), so replaying them would change nothing.
        AgentEvent::ModelDelta { .. }
        | AgentEvent::ToolProgress { .. }
        // Failure detail already carried by a span-bearing sibling: a tool's
        // outcome rides `ToolCompleted.error`, a model's and a run's ride
        // `RunFailed`, and `InvalidToolArgs` precedes the `ToolCompleted` that
        // reports it.
        | AgentEvent::ToolFailed { .. }
        | AgentEvent::ModelFailed { .. }
        | AgentEvent::SubAgentFailed { .. }
        | AgentEvent::InvalidToolArgs { .. }
        | AgentEvent::MiddlewareFailed { .. }
        // Run-shaping diagnostics: they change what the model is asked, never
        // what the trace records. The live bridge logs them and emits no
        // `AgentProgress` for any of them either.
        | AgentEvent::ToolsFiltered { .. }
        | AgentEvent::ToolsAdvertised { .. }
        | AgentEvent::DeferredToolCall { .. }
        | AgentEvent::WorkspacePrepared { .. }
        | AgentEvent::WorkspaceViolation { .. }
        | AgentEvent::WorkspaceCleanup { .. }
        | AgentEvent::ControlApplied { .. }
        | AgentEvent::StateUpdate
        | AgentEvent::MiddlewareStarted { .. }
        | AgentEvent::MiddlewareCompleted { .. }
        | AgentEvent::CacheHit { .. }
        | AgentEvent::CacheMiss { .. }
        | AgentEvent::RetryScheduled { .. }
        | AgentEvent::RateLimitWaited { .. }
        | AgentEvent::FallbackSelected { .. }
        | AgentEvent::ModelOverrideSkipped { .. }
        | AgentEvent::FallbackSkipped { .. }
        | AgentEvent::SubAgentReused { .. }
        | AgentEvent::Steered { .. }
        | AgentEvent::Compressed { .. }
        | AgentEvent::RouteSelected { .. }
        | AgentEvent::MemoryLoaded
        | AgentEvent::MemorySaved
        | AgentEvent::StreamClosed
        // Budget accounting. The span roll-up is folded from `UsageRecorded`
        // above; these report headroom, not usage. `CostRecorded` is declared
        // upstream for future emit and is never emitted today — if the crate
        // starts emitting it, it becomes the authoritative source for
        // `TurnCostUpdated.total_usd` and should replace the estimate there.
        | AgentEvent::CostRecorded { .. }
        | AgentEvent::BudgetWarning { .. }
        | AgentEvent::BudgetReserved { .. }
        | AgentEvent::BudgetReconciled { .. }
        | AgentEvent::BudgetExceeded { .. }
        | AgentEvent::LimitReached { .. } => Vec::new(),
        // `AgentEvent` is `#[non_exhaustive]`: a variant added upstream after
        // this projection was written carries no span this replay models.
        _ => Vec::new(),
    }
}

/// Projects a run's journalled `observations` into trace spans by replaying
/// them into [`AgentProgress`] and folding through a fresh [`SpanCollector`],
/// stamped with each observation's journal timestamp (`ts_ms`).
pub(crate) fn spans_from_observations(
    ctx: TraceContext,
    max_iterations: u32,
    observations: &[AgentObservation],
) -> Vec<TraceSpan> {
    let capture_content = ctx.capture_content;
    let mut collector = SpanCollector::new(ctx).with_content_capture(capture_content);
    let mut state = ReplayState {
        max_iterations,
        ..ReplayState::default()
    };
    let mut last_ts = 0;
    for obs in observations {
        last_ts = obs.ts_ms;
        let progress = observation_to_progress(obs, &mut state);
        for step in std::mem::take(&mut state.pre) {
            match step {
                Replay::At(event, at) => collector.record(&event, at),
                Replay::CallStart(task_id, at) => {
                    collector.set_next_call_start(task_id.as_deref(), at)
                }
                Replay::Outcome(outcome) => collector.apply_turn_outcome(outcome, obs.ts_ms),
            }
        }
        for progress in progress {
            collector.record(&progress, obs.ts_ms);
        }
    }
    collector.finish(last_ts);
    collector.spans().to_vec()
}
