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
//! Known parity gaps (see `docs/.../C4-journal-progress-parity-plan.md` §2a):
//! - **Cost is an estimate, not the charge.** The provider's charged USD
//!   reaches the live path through the `usage_carry` side-channel, which is not
//!   an `AgentEvent` and is not journalled, so both the per-call
//!   `ModelCallCompleted.cost_usd` (`0` here — the generation span's cost is
//!   sourced from the persisted per-run cost store at export time) and the
//!   turn roll-up folded from `UsageRecorded` are journal-only figures. Token
//!   counts are exact; treat a projected `gen_ai.usage.cost_usd` as an
//!   estimate. `AgentEvent::CostRecorded` would close this if the crate ever
//!   emits it.
//! - Sub-agent prompt/output content is not on the crate lifecycle events, so
//!   subagent spans carry lifecycle/timing and child tool/model structure but
//!   empty delegated prompt/final output until a richer journal event exists.
//! - `AgentProgress::SubagentAwaitingUser` has no journal source at all (the
//!   crate emits no matching lifecycle event), so a subagent span parked on a
//!   user prompt loses that attribute on replay.
//!
//! Everything else is projected. In particular the match in
//! [`observation_to_progress`] is **exhaustive over `AgentEvent`** — a crate
//! that adds a span-bearing event breaks the build here rather than silently
//! diverging, which is exactly how the missing `UsageRecorded` roll-up went
//! unnoticed (openhuman#6148).

use tinyagents_harness::events::AgentEvent;
use tinyagents_harness::observability::AgentObservation;

use super::{SpanCollector, TraceContext, TraceSpan};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::tools::status::classify;

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

        AgentEvent::ToolStarted { call_id, tool_name } => match state.active_subagent() {
            Some(scope) => vec![AgentProgress::SubagentToolCallStarted {
                agent_id: scope.agent_id.clone(),
                task_id: scope.task_id.clone(),
                call_id: call_id.as_str().to_string(),
                tool_name: tool_name.clone(),
                arguments: serde_json::Value::Null,
                iteration: scope.iteration,
                display_label: None,
                display_detail: None,
            }],
            None => vec![AgentProgress::ToolCallStarted {
                call_id: call_id.as_str().to_string(),
                tool_name: tool_name.clone(),
                // The journal does not carry the model's raw argument JSON in
                // payload-free mode; the tool span still renders from name + id.
                arguments: serde_json::Value::Null,
                iteration: state.iteration,
                display_label: None,
                display_detail: None,
            }],
        },

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
            let failure = error.as_ref().map(|text| classify(text, false));
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
                    success: error.is_none(),
                    output_chars: output_bytes.unwrap_or(0) as usize,
                    output: output_text,
                    arguments: input.clone(),
                    elapsed_ms: duration_ms.unwrap_or(0),
                    iteration: scope.iteration,
                    failure,
                }],
                None => vec![AgentProgress::ToolCallCompleted {
                    call_id: call_id.as_str().to_string(),
                    tool_name: tool_name.clone(),
                    success: error.is_none(),
                    output_chars: output_bytes.unwrap_or(0) as usize,
                    output: output_text,
                    arguments: input.clone(),
                    elapsed_ms: duration_ms.unwrap_or(0),
                    iteration: state.iteration,
                    failure,
                }],
            }
        }

        AgentEvent::UnknownToolCall {
            call_id,
            requested_name,
            arguments,
            recovery: _,
        } => {
            // #4118: the crate recovers an unavailable tool call without ever
            // emitting `ToolStarted`/`ToolCompleted` for it, so the live bridge
            // synthesises the pair itself. Without this arm the projection was
            // short a whole tool span — a span *count* divergence, not just a
            // missing attribute — on every turn the model named a tool it did
            // not have. Mirrors `observability_part_02.rs`'s `UnknownToolCall`
            // arm exactly, including the `Unknown` (recoverable) class.
            let failure = Some(crate::openhuman::tools::status::describe(
                crate::openhuman::tools::status::ToolFailureClass::Unknown,
            ));
            let label = format!(
                "{} (unavailable)",
                crate::openhuman::tools::traits::humanize_tool_name(requested_name)
            );
            let detail = Some("tool not available".to_string());
            match state.active_subagent() {
                Some(scope) => vec![
                    AgentProgress::SubagentToolCallStarted {
                        agent_id: scope.agent_id.clone(),
                        task_id: scope.task_id.clone(),
                        call_id: call_id.as_str().to_string(),
                        tool_name: requested_name.clone(),
                        arguments: arguments.clone(),
                        iteration: scope.iteration,
                        display_label: Some(label),
                        display_detail: detail,
                    },
                    AgentProgress::SubagentToolCallCompleted {
                        agent_id: scope.agent_id.clone(),
                        task_id: scope.task_id.clone(),
                        call_id: call_id.as_str().to_string(),
                        tool_name: requested_name.clone(),
                        success: false,
                        output_chars: 0,
                        output: String::new(),
                        arguments: Some(arguments.clone()),
                        elapsed_ms: 0,
                        iteration: scope.iteration,
                        failure,
                    },
                ],
                None => vec![
                    AgentProgress::ToolCallStarted {
                        call_id: call_id.as_str().to_string(),
                        tool_name: requested_name.clone(),
                        arguments: arguments.clone(),
                        iteration: state.iteration,
                        display_label: Some(label),
                        display_detail: detail,
                    },
                    AgentProgress::ToolCallCompleted {
                        call_id: call_id.as_str().to_string(),
                        tool_name: requested_name.clone(),
                        success: false,
                        output_chars: 0,
                        output: String::new(),
                        arguments: Some(arguments.clone()),
                        elapsed_ms: 0,
                        iteration: state.iteration,
                        failure,
                    },
                ],
            }
        }

        AgentEvent::ModelCompleted {
            call_id,
            usage,
            input,
            output,
            ..
        } => {
            let model = state
                .models
                .get(call_id.as_str())
                .cloned()
                .unwrap_or_default();
            let usage = usage.unwrap_or_default();
            let scope = state.active_subagent().cloned();
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
                cost_usd: 0.0,
            }];
            if scope.is_none() {
                let turn_input = input.as_ref().map(json_content_text);
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
            // the per-child roll-up (`observability_part_01.rs`, `self.scope`
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
            state.cost_usd += crate::openhuman::agent::cost::estimate_call_cost_usd(
                &state.model,
                &crate::openhuman::inference::provider::UsageInfo {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    context_window: 0,
                    cached_input_tokens: usage.cache_read_tokens,
                    cache_creation_tokens: usage.cache_creation_tokens,
                    reasoning_tokens: usage.reasoning_tokens,
                    charged_amount_usd: 0.0,
                },
            );
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
                worktree_path: None,
                changed_files: Vec::new(),
                dirty_status: None,
            }]
        }

        AgentEvent::RunCompleted { .. } => {
            if state.active_subagent().is_some() {
                Vec::new()
            } else {
                vec![AgentProgress::TurnCompleted {
                    iterations: state.iteration,
                }]
            }
        }

        AgentEvent::RunFailed { error, .. } => {
            let Some(scope) = state.subagents.pop() else {
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
    }
}

fn json_content_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Object(map) => map
            .get("content")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
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
        for progress in observation_to_progress(obs, &mut state) {
            collector.record(&progress, obs.ts_ms);
        }
    }
    collector.finish(last_ts);
    collector.spans().to_vec()
}
