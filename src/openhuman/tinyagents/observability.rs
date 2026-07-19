//! Bridge the `tinyagents` harness event stream onto openhuman's
//! [`AgentProgress`] + cost tracker (issue #4249).
//!
//! tinyagents emits a typed [`AgentEvent`] stream (model started/delta/completed,
//! tool started/completed, usage) through an [`EventSink`] that callers attach
//! to a [`RunContext`]. This listener translates those into the same
//! `AgentProgress` events the legacy `run_turn_engine` produced — restoring the
//! live tool timeline, streaming text, and the cost/token footer on the
//! tinyagents path — and feeds per-call usage into the global cost tracker.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::Sender;

use tinyagents::graph::stream::{GraphEvent, GraphEventSink};
use tinyagents::harness::cache::CacheLayoutEvent;
use tinyagents::harness::events::{AgentEvent, EventListener, EventRecord};
use tinyagents::harness::steering::{SteeringCommand, SteeringHandle};
use tinyagents::harness::usage::Usage;

use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::UsageInfo;
use crate::openhuman::tools::traits::humanize_tool_name;

/// Attribution for child (sub-agent) progress. When present, the bridge routes
/// events to the `Subagent*` [`AgentProgress`] variants (so the parent thread
/// can nest child activity under a live subagent row) instead of the top-level
/// ones. Absent = a parent/top-level turn.
#[derive(Clone)]
pub struct SubagentScope {
    pub agent_id: String,
    pub task_id: String,
    pub extended_policy: bool,
}

/// A shared 1-based model-call (iteration) cursor. The bridge advances it on
/// each `ModelStarted` event; the model adapter reads it to attribute the
/// tool-argument deltas it still forwards out-of-band.
pub(crate) type IterationCursor = Arc<AtomicU32>;

/// A shared `call_id → tool_name` map. The model adapter's `ThinkingForwarder`
/// writes it when a tool call *starts* (the crate `ToolDelta` has no `tool_name`
/// field, so the start-event/name half of the tool-arg contract can't ride the
/// crate stream and stays on the out-of-band forwarder path — see
/// [`super::model::ThinkingForwarder`]). The bridge reads it to label the
/// incremental tool-argument fragments it now projects off the crate stream
/// (`MessageDelta.tool_call`), preserving the UI's `ToolCallArgsDelta`
/// `tool_name` contract without the forwarder emitting those fragments itself.
pub(crate) type ToolNameMap = Arc<Mutex<std::collections::HashMap<String, String>>>;

/// Shared `call_id → (success, classified failure, elapsed_ms, output_chars)`
/// side-channel. The crate's `AgentEvent::ToolCompleted` carries only `call_id`
/// + `tool_name` (no success/error, duration, or output size), so
///
/// `ToolOutcomeCaptureMiddleware::after_tool` — which does see the `ToolResult`
/// (including the executor-measured `elapsed_ms` and the rendered content) —
/// classifies each outcome and writes it here; the bridge reads it when
/// projecting the live `ToolCallCompleted` event, so a failed tool surfaces real
/// `success: false` + a user-facing `failure`, and a completed tool surfaces its
/// real duration + output size instead of `0`/`0` (#4467, item 4). Absent entry
/// (event projected before the middleware ran) falls back to `(true, None, 0, 0)`.
pub(crate) type ToolFailureMap = Arc<
    Mutex<
        std::collections::HashMap<
            String,
            (
                bool,
                Option<crate::openhuman::tool_status::ClassifiedFailure>,
                u64,
                usize,
            ),
        >,
    >,
>;

/// Shared FIFO carry of the per-call provider [`UsageInfo`] the model adapter
/// observed, drained by the bridge when it records that call's usage. The crate
/// `Usage` the harness surfaces on `AgentEvent::UsageRecorded` carries only token
/// counts, so the backend-charged USD, the model's context window, and the
/// cache-creation/reasoning token breakdown have no crate home — the model
/// adapter pushes the full provider `UsageInfo` here (one push per provider
/// response) and the bridge pops it (one pop per recorded model call, after the
/// duplicate-usage dedupe guard) to restore charged-USD precedence and the full
/// accounting (#4467, item 1). A pop that finds nothing (a fallback-route call
/// that did not push, or an out-of-band usage event) degrades gracefully to a
/// catalogue estimate.
pub(crate) type ProviderUsageCarry =
    Arc<Mutex<std::collections::VecDeque<crate::openhuman::inference::provider::UsageInfo>>>;

/// An [`EventListener`] that pauses the run once `cap` model calls have
/// completed, so the loop stops gracefully at the iteration budget (returning
/// the partial transcript) instead of erroring with `LimitExceeded`. The harness
/// checks pending steering at the top of each turn *before* the model-call limit
/// check, so a `Pause` sent here short-circuits the loop cleanly. The caller then
/// inspects the run's finish reason to decide whether to summarize a checkpoint
/// — the tinyagents analogue of the legacy cap checkpoint seam.
pub(crate) struct CapPauser {
    handle: SteeringHandle,
    cap: u32,
    completed: AtomicU32,
}

impl CapPauser {
    /// Pause `handle` once `cap` model calls complete.
    pub(crate) fn new(handle: SteeringHandle, cap: usize) -> Arc<Self> {
        Arc::new(Self {
            handle,
            cap: cap as u32,
            completed: AtomicU32::new(0),
        })
    }
}

impl EventListener for CapPauser {
    fn on_event(&self, record: &EventRecord) {
        if matches!(record.event, AgentEvent::ModelCompleted { .. }) {
            let n = self.completed.fetch_add(1, Ordering::SeqCst) + 1;
            if n >= self.cap {
                tracing::info!(
                    completed = n,
                    cap = self.cap,
                    "[tinyagents] model-call cap reached — requesting graceful pause"
                );
                self.handle.send(SteeringCommand::Pause);
            }
        }
    }
}

#[derive(Default)]
struct BridgeState {
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    charged_amount_usd: f64,
    /// Local response-cache hits observed on this turn (issue #4249, 03.2). A hit
    /// means the harness served a model call from its [`ResponseCache`] without
    /// invoking the provider. Additive counters — a follow-up (coordinated with
    /// workstream 06) wires these into the cost-footer DTO; today they are logged
    /// with a grep-friendly `[cache]` prefix and exposed via [`OpenhumanEventBridge::cache_counts`].
    cache_hits: u64,
    /// Local response-cache misses observed on this turn (provider *was* invoked).
    cache_misses: u64,
}

/// Per-model-call figures `record_usage` resolved from the provider-usage
/// carry (charged>estimate cost precedence + cache/reasoning breakdown),
/// keyed by iteration so the subsequent `ModelCompleted` projection reports
/// the same numbers as the wallet accounting.
#[derive(Clone, Copy, Debug)]
struct ResolvedCallFigures {
    cost_usd: f64,
    cache_creation_tokens: u64,
    reasoning_tokens: u64,
}

/// An [`EventListener`] that mirrors harness events onto openhuman's progress
/// sink and cost tracker.
pub(crate) struct OpenhumanEventBridge {
    on_progress: Option<Sender<AgentProgress>>,
    model: String,
    /// Telemetry provider id (`"managed"`, `"openai"`, …) — from
    /// [`Provider::telemetry_provider_id`](crate::openhuman::inference::provider::Provider::telemetry_provider_id).
    /// Rides on `ModelCallCompleted` so trace exporters render the Langfuse
    /// model as `{provider_id}.{model}`.
    provider_id: String,
    max_iterations: u32,
    /// `None` for a parent turn; `Some` to emit child-scoped `Subagent*` events.
    scope: Option<SubagentScope>,
    /// Shared with the model adapter so thinking deltas line up with the
    /// model call (iteration) they belong to.
    cursor: IterationCursor,
    /// Shared `call_id → tool_name` map written by the model adapter's
    /// `ThinkingForwarder` on tool-call start; read here to label the
    /// incremental tool-argument fragments projected off the crate stream.
    tool_names: ToolNameMap,
    /// Shared `call_id → (success, failure, elapsed_ms, output_chars)`
    /// side-channel written by `ToolOutcomeCaptureMiddleware`; read when
    /// projecting `ToolCallCompleted`.
    failure_map: ToolFailureMap,
    /// Shared FIFO carry of the per-call provider `UsageInfo` the model adapter
    /// observed; drained in `record_usage` to restore backend-charged USD +
    /// context-window + cache-creation/reasoning tokens the crate `Usage` drops.
    usage_carry: ProviderUsageCarry,
    /// Model-call iterations whose `UsageRecorded` has already been folded into
    /// the global cost tracker (W2-budget-dedupe). A single model call can now
    /// surface **two** `UsageRecorded` events — one from the harness runtime
    /// (`agent_loop`, always) and one from the observe-only crate
    /// `BudgetMiddleware::after_model` — both carrying identical usage and both
    /// delivered to this bridge. Keyed on the run-scoped model-call identity (the
    /// iteration cursor, bumped once per `ModelStarted`) so a given call's usage
    /// is recorded exactly once. See [`OpenhumanEventBridge::record_usage`].
    recorded_iterations: Mutex<std::collections::HashSet<u32>>,
    /// Per-iteration figures resolved by `record_usage` (see
    /// [`ResolvedCallFigures`]); taken by the `ModelCompleted` arm.
    resolved_calls: Mutex<std::collections::HashMap<u32, ResolvedCallFigures>>,
    /// `call_id → start instant` for in-flight tool calls, written on
    /// `ToolStarted` and taken on `ToolCompleted` so the projected completion
    /// event carries a real `elapsed_ms` (the crate event has no timing).
    tool_started_at: Mutex<std::collections::HashMap<String, std::time::Instant>>,
    state: Mutex<BridgeState>,
    /// Ordered overflow buffer for progress events that hit backpressure
    /// (channel `Full`). Once ANY event spills here, `draining` stays set and
    /// every subsequent event queues here too — a single spawned forwarder
    /// drains them to the channel in FIFO order — so a later fast-path
    /// `try_send` can never jump ahead of an earlier spilled event and scramble
    /// start/completed ordering (which would leave a tool row stuck `running`
    /// when a `ToolCallCompleted` overtakes its `ToolCallStarted`) (#4466).
    overflow: Arc<Mutex<OverflowState>>,
}

/// Backpressure overflow state guarded by a single mutex so the "are we
/// draining?" decision and the queue mutation stay atomic together.
#[derive(Default)]
struct OverflowState {
    queue: std::collections::VecDeque<AgentProgress>,
    draining: bool,
}

impl OpenhumanEventBridge {
    /// Build a parent-scoped bridge for `model`.
    pub(crate) fn new(
        on_progress: Option<Sender<AgentProgress>>,
        model: impl Into<String>,
        max_iterations: usize,
    ) -> Arc<Self> {
        Self::with_scope(
            on_progress,
            model,
            "custom",
            max_iterations,
            None,
            Arc::default(),
            Arc::default(),
            Arc::default(),
            Arc::default(),
        )
    }

    /// Build a bridge, optionally child-scoped, sharing `cursor` (iteration
    /// attribution) and `tool_names` (tool-call name lookup for the streamed
    /// argument fragments) with the model adapter.
    pub(crate) fn with_scope(
        on_progress: Option<Sender<AgentProgress>>,
        model: impl Into<String>,
        provider_id: impl Into<String>,
        max_iterations: usize,
        scope: Option<SubagentScope>,
        cursor: IterationCursor,
        tool_names: ToolNameMap,
        failure_map: ToolFailureMap,
        usage_carry: ProviderUsageCarry,
    ) -> Arc<Self> {
        Arc::new(Self {
            on_progress,
            model: model.into(),
            provider_id: provider_id.into(),
            max_iterations: max_iterations as u32,
            scope,
            cursor,
            tool_names,
            failure_map,
            usage_carry,
            recorded_iterations: Mutex::new(std::collections::HashSet::new()),
            resolved_calls: Mutex::new(std::collections::HashMap::new()),
            tool_started_at: Mutex::new(std::collections::HashMap::new()),
            state: Mutex::new(BridgeState::default()),
            overflow: Arc::default(),
        })
    }

    /// Cumulative `(input_tokens, output_tokens, charged_usd)` observed so far.
    fn totals(&self) -> (u64, u64, f64) {
        let s = self.state.lock().unwrap();
        (s.input_tokens, s.output_tokens, s.charged_amount_usd)
    }

    /// Cumulative `(input_tokens, output_tokens, cached_input_tokens, charged_usd)`
    /// observed so far — the full accounting the turn persists (transcript cost /
    /// session meters), so a normal turn no longer records `$0` and zero cached
    /// tokens despite real usage.
    pub(crate) fn totals_with_cost(&self) -> (u64, u64, u64, f64) {
        let s = self.state.lock().unwrap();
        (
            s.input_tokens,
            s.output_tokens,
            s.cached_input_tokens,
            s.charged_amount_usd,
        )
    }

    /// Cumulative `(cache_hits, cache_misses)` observed so far (issue #4249,
    /// 03.2). Exposed so the turn loop can surface response-cache effectiveness;
    /// the cost-footer DTO wiring is a follow-up (workstream 06).
    pub(crate) fn cache_counts(&self) -> (u64, u64) {
        let s = self.state.lock().unwrap();
        (s.cache_hits, s.cache_misses)
    }

    /// Forward a progress event without ever silently dropping it under
    /// backpressure (#4466). The crate `EventListener::on_event` callback is
    /// **synchronous**, so we cannot `.await` a bounded `send()` inline the way
    /// the legacy streaming path did. Fast path: `try_send`, which succeeds (and
    /// stays fully synchronous + ordered) whenever the downstream channel has
    /// room — the common case. Only when the channel is momentarily **full** do
    /// we fall back to an awaited `send()` on a spawned task so the delta is
    /// delivered under backpressure instead of being dropped (the old bug). A
    /// `Closed` channel means the receiver is gone (turn tore down), where
    /// dropping is correct.
    fn send(&self, progress: AgentProgress) {
        use tokio::sync::mpsc::error::TrySendError;
        let Some(tx) = &self.on_progress else {
            return;
        };
        // Hold the overflow lock across the ordering decision so "are we
        // draining?" and the queue mutation are atomic (try_send is
        // non-blocking, so holding a std mutex across it is fine).
        let mut ov = self.overflow.lock().unwrap_or_else(|p| p.into_inner());
        if ov.draining {
            // Already spilling: queue in order; the single forwarder delivers it.
            ov.queue.push_back(progress);
            return;
        }
        match tx.try_send(progress) {
            Ok(()) => {}
            Err(TrySendError::Closed(_)) => {}
            Err(TrySendError::Full(progress)) => {
                // Backpressure, not capacity loss. Enter ordered-drain mode:
                // queue this event and spawn ONE forwarder that awaits `send()`
                // per event in FIFO order. `draining` stays set (so every later
                // event also queues here) until the buffer fully drains — that is
                // what stops a later `try_send` from overtaking a spilled earlier
                // event and scrambling start/completed ordering.
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    ov.queue.push_back(progress);
                    ov.draining = true;
                    let overflow = Arc::clone(&self.overflow);
                    let tx = tx.clone();
                    drop(ov);
                    handle.spawn(async move {
                        loop {
                            let next = {
                                let mut ov = overflow.lock().unwrap_or_else(|p| p.into_inner());
                                match ov.queue.pop_front() {
                                    Some(item) => item,
                                    None => {
                                        ov.draining = false;
                                        break;
                                    }
                                }
                            };
                            if tx.send(next).await.is_err() {
                                // Receiver gone: stop draining, discard the rest.
                                let mut ov = overflow.lock().unwrap_or_else(|p| p.into_inner());
                                ov.queue.clear();
                                ov.draining = false;
                                break;
                            }
                        }
                    });
                } else {
                    tracing::debug!(
                        model = %self.model,
                        "[tinyagents] progress channel full and no runtime to defer send; dropping one delta"
                    );
                }
            }
        }
    }

    fn iteration(&self) -> u32 {
        self.cursor.load(Ordering::SeqCst)
    }

    /// Accumulate a usage block, feed the global cost tracker, and emit a
    /// `TurnCostUpdated` so the UI footer stays live.
    fn record_usage(&self, usage: &Usage) {
        let iteration = self.iteration();
        // Dedupe guard (W2-budget-dedupe): record a given model call's usage into
        // the global cost tracker **exactly once**. Installing the observe-only
        // crate `BudgetMiddleware` makes each model call emit two `UsageRecorded`
        // events (the runtime's own at `agent_loop` + the middleware's
        // `after_model` re-emit), both reaching this listener with identical
        // usage. The two events have *distinct* stable ids, so an event-id key
        // would not collapse them — instead we key on the run-scoped model-call
        // identity: the iteration cursor, bumped once per `ModelStarted`. This
        // bridge instance is per-run (parent or child scope), so the set is
        // naturally (run, turn)-scoped. First writer for an iteration records;
        // any later `UsageRecorded` for the same iteration is a duplicate.
        {
            let mut seen = self
                .recorded_iterations
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if !seen.insert(iteration) {
                tracing::debug!(
                    iteration,
                    model = %self.model,
                    child = self.scope.is_some(),
                    "[budget] duplicate UsageRecorded for model call — skipping double record"
                );
                return;
            }
        }
        // Drain the provider-usage side-channel the model adapter fed for this
        // model call (FIFO, one push per provider response). The crate `Usage`
        // the harness surfaces carries only token counts, so the backend-charged
        // USD, the model's context window, and the cache-creation/reasoning
        // breakdown ride this out-of-band carry instead (#4467, item 1). Popped
        // AFTER the dedupe guard above so the duplicate `UsageRecorded` re-emit
        // (crate `BudgetMiddleware`) does not consume a second entry.
        let carried = self
            .usage_carry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_front();

        // Estimate as the floor via the tier-aware `agent::cost` table (managed
        // handles like `chat-v1`/`burst-v1` + the vendor catalog + heuristics —
        // the catalog-only lookup priced every managed-tier call as $0); prefer
        // the provider's own charged amount when it reported one (charged >
        // estimate precedence, so credit-metered backends surface real billing
        // rather than a token-rate estimate).
        let estimate = Self::estimate_call_cost(&self.model, usage);
        let call_cost = carried
            .as_ref()
            .map(|u| u.charged_amount_usd)
            .filter(|c| c.is_finite() && *c > 0.0)
            .unwrap_or(estimate);
        // The context window + cache-creation/reasoning breakdown only exist on
        // the carried provider usage (the crate `Usage` mapping drops them); fall
        // back to the catalogue window and the crate token counts when absent.
        let context_window = carried
            .as_ref()
            .map(|u| u.context_window)
            .filter(|w| *w > 0)
            .unwrap_or_else(|| {
                crate::openhuman::cost::catalog::lookup(&self.model)
                    .map(|p| u64::from(p.context_window))
                    .unwrap_or(0)
            });
        let cache_creation_tokens = carried
            .as_ref()
            .map(|u| u.cache_creation_tokens)
            .filter(|t| *t > 0)
            .unwrap_or(usage.cache_creation_tokens);
        let reasoning_tokens = carried
            .as_ref()
            .map(|u| u.reasoning_tokens)
            .filter(|t| *t > 0)
            .unwrap_or(usage.reasoning_tokens);
        tracing::trace!(
            model = %self.model,
            iteration,
            charged_from_provider = carried
                .as_ref()
                .map(|u| u.charged_amount_usd > 0.0)
                .unwrap_or(false),
            call_cost,
            context_window,
            "[cost] recording per-call usage (charged>estimate precedence via provider carry)"
        );
        // Stash the resolved per-call figures so the `ModelCompleted` arm (which
        // fires right after this event and emits the `ModelCallCompleted`
        // generation telemetry) reports the SAME cost/cache/reasoning numbers as
        // the wallet accounting, instead of re-deriving a bare estimate.
        self.resolved_calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                iteration,
                ResolvedCallFigures {
                    cost_usd: call_cost,
                    cache_creation_tokens,
                    reasoning_tokens,
                },
            );
        let (input, output, cached, charged) = {
            let mut s = self.state.lock().unwrap();
            s.input_tokens += usage.input_tokens;
            s.output_tokens += usage.output_tokens;
            s.cached_input_tokens += usage.cache_read_tokens;
            s.charged_amount_usd += call_cost;
            (
                s.input_tokens,
                s.output_tokens,
                s.cached_input_tokens,
                s.charged_amount_usd,
            )
        };

        // Feed the authoritative global cost tracker (same call the legacy
        // observer made), so the wallet/cost surfaces stay accurate.
        let usage_info = UsageInfo {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            context_window,
            cached_input_tokens: usage.cache_read_tokens,
            cache_creation_tokens,
            reasoning_tokens,
            charged_amount_usd: call_cost,
        };
        if reasoning_tokens > 0 || cache_creation_tokens > 0 {
            log::debug!(
                "[cost] recording reasoning/cache-creation tokens model={} reasoning_tokens={} cache_creation_tokens={}",
                self.model,
                reasoning_tokens,
                cache_creation_tokens
            );
        }
        crate::openhuman::cost::record_provider_usage(&self.model, &usage_info);

        // The cost footer is a top-level surface; for a child run the global
        // cost tracker feed above is the authoritative accounting and the parent
        // emits its own footer, so suppress the per-child `TurnCostUpdated`.
        // Per-call generation telemetry (`ModelCallCompleted`) is emitted from
        // the `AgentEvent::ModelCompleted` arm instead — that event fires after
        // `UsageRecorded` and is the only one carrying the captured request
        // messages + completion, so the generation gets usage AND content in
        // one shot (for parent and child scopes alike).
        if self.scope.is_none() {
            self.send(AgentProgress::TurnCostUpdated {
                model: self.model.clone(),
                iteration,
                input_tokens: input,
                output_tokens: output,
                cached_input_tokens: cached,
                total_usd: charged,
            });
        }
    }

    /// Estimate one call's USD cost. Uses the tier-aware
    /// [`agent::cost`](crate::openhuman::agent::cost) table (managed handles
    /// like `chat-v1`/`burst-v1` + the vendor catalog + heuristics) — the
    /// previous `cost::catalog::estimate_cost_usd` only knew concrete vendor
    /// ids, so every managed-tier call priced as $0 in traces and the footer.
    fn estimate_call_cost(model: &str, usage: &Usage) -> f64 {
        crate::openhuman::agent::cost::estimate_call_cost_usd(
            model,
            &UsageInfo {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                context_window: 0,
                cached_input_tokens: usage.cache_read_tokens,
                cache_creation_tokens: usage.cache_creation_tokens,
                reasoning_tokens: usage.reasoning_tokens,
                charged_amount_usd: 0.0,
            },
        )
    }
}

impl EventListener for OpenhumanEventBridge {
    fn on_event(&self, record: &EventRecord) {
        match &record.event {
            AgentEvent::ModelStarted { .. } => {
                let iteration = self.cursor.fetch_add(1, Ordering::SeqCst) + 1;
                match &self.scope {
                    None => self.send(AgentProgress::IterationStarted {
                        iteration,
                        max_iterations: self.max_iterations,
                    }),
                    Some(s) => self.send(AgentProgress::SubagentIterationStarted {
                        agent_id: s.agent_id.clone(),
                        task_id: s.task_id.clone(),
                        iteration,
                        max_iterations: self.max_iterations,
                        extended_policy: s.extended_policy,
                    }),
                }
            }
            AgentEvent::ModelDelta { delta, .. } => {
                let iteration = self.iteration();
                if !delta.text.is_empty() {
                    match &self.scope {
                        None => self.send(AgentProgress::TextDelta {
                            delta: delta.text.clone(),
                            iteration,
                        }),
                        Some(s) => self.send(AgentProgress::SubagentTextDelta {
                            agent_id: s.agent_id.clone(),
                            task_id: s.task_id.clone(),
                            delta: delta.text.clone(),
                            iteration,
                        }),
                    }
                }
                if !delta.reasoning.is_empty() {
                    match &self.scope {
                        None => self.send(AgentProgress::ThinkingDelta {
                            delta: delta.reasoning.clone(),
                            iteration,
                        }),
                        Some(s) => self.send(AgentProgress::SubagentThinkingDelta {
                            agent_id: s.agent_id.clone(),
                            task_id: s.task_id.clone(),
                            delta: delta.reasoning.clone(),
                            iteration,
                        }),
                    }
                }
                // Tool-call **start** + **argument** fragments both ride the crate
                // stream (`MessageDelta.tool_call`) now — the out-of-band
                // `ThinkingForwarder` is gone. The call-opening delta carries the
                // tool name (crate `ToolDelta::tool_name`, G2) with empty content;
                // argument fragments carry content with no name. We record the
                // name on the opening delta so subsequent fragments can be
                // labelled, and project both onto the `ToolCallArgsDelta` the UI
                // timeline consumes so the model can be shown composing the call
                // before it executes.
                if let Some(tool_call) = &delta.tool_call {
                    // Record the tool name as soon as the call opens (matching the
                    // legacy forwarder's `note_tool_call`), and emit the start
                    // marker — an empty-delta `ToolCallArgsDelta` — top-level
                    // regardless of scope, exactly as the forwarder did.
                    if let Some(name) = tool_call.tool_name.as_deref().filter(|n| !n.is_empty()) {
                        self.tool_names
                            .lock()
                            .unwrap()
                            .insert(tool_call.call_id.clone(), name.to_string());
                        if tool_call.content.is_empty() {
                            self.send(AgentProgress::ToolCallArgsDelta {
                                call_id: tool_call.call_id.clone(),
                                tool_name: name.to_string(),
                                delta: String::new(),
                                iteration,
                            });
                        }
                    }
                    // Argument fragments are parent-only: there is no `Subagent*`
                    // tool-arg variant, and an UNSCOPED top-level `ToolCallArgsDelta`
                    // emitted from a child run would render the child's argument
                    // composition as the *parent's* own timeline activity (#4467,
                    // item 6; v0.58.7 dropped child arg fragments). A child run's
                    // Started/Completed rows already carry the final arguments
                    // under the `Subagent*` scope.
                    if self.scope.is_none() && !tool_call.content.is_empty() {
                        let tool_name = tool_call
                            .tool_name
                            .as_deref()
                            .filter(|n| !n.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| {
                                self.tool_names
                                    .lock()
                                    .unwrap()
                                    .get(&tool_call.call_id)
                                    .cloned()
                                    .unwrap_or_default()
                            });
                        tracing::trace!(
                            call_id = tool_call.call_id.as_str(),
                            tool_name = tool_name.as_str(),
                            len = tool_call.content.len(),
                            "[stream] projecting crate tool-arg fragment onto ToolCallArgsDelta"
                        );
                        self.send(AgentProgress::ToolCallArgsDelta {
                            call_id: tool_call.call_id.clone(),
                            tool_name,
                            delta: tool_call.content.clone(),
                            iteration,
                        });
                    }
                }
            }
            // `UsageRecorded` carries the authoritative per-call usage and fires
            // exactly once per model call; prefer it over `ModelCompleted`'s
            // optional usage to avoid double counting.
            AgentEvent::UsageRecorded { usage } => self.record_usage(usage),
            // Per-call generation telemetry. `ModelCompleted` fires exactly once
            // per model call, after `UsageRecorded`, and is the only event
            // carrying the captured request messages (incl. the system prompt)
            // + completion (`RunPolicy.capture.model_io`, enabled in
            // `run_policy_for`). Emitted for parent AND child scopes — the
            // child call carries its owning `subagent_task_id` so the trace
            // exporter nests the generation under the subagent span (this is
            // what makes the Context Scout's model calls visible in Langfuse).
            AgentEvent::ModelCompleted {
                usage,
                input,
                output,
                ..
            } => {
                let iteration = self.iteration();
                let usage = usage.unwrap_or_default();
                // Prefer the figures `record_usage` resolved for this call
                // (charged>estimate cost + carried cache/reasoning breakdown —
                // `UsageRecorded` fires before `ModelCompleted`), so the
                // generation telemetry matches the wallet accounting exactly;
                // fall back to a bare tier-aware estimate when absent.
                let resolved = self
                    .resolved_calls
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&iteration);
                let call_cost = resolved
                    .as_ref()
                    .map(|r| r.cost_usd)
                    .unwrap_or_else(|| Self::estimate_call_cost(&self.model, &usage));
                let cache_creation_tokens = resolved
                    .as_ref()
                    .map(|r| r.cache_creation_tokens)
                    .unwrap_or(usage.cache_creation_tokens);
                let reasoning_tokens = resolved
                    .as_ref()
                    .map(|r| r.reasoning_tokens)
                    .unwrap_or(usage.reasoning_tokens);
                log::debug!(
                    "[tinyagents][usage] model_call_completed model={} provider={} iteration={} \
                     child={} in={} out={} cache_read={} cache_write={} reasoning={} \
                     cost_usd={:.6} input_captured={} output_captured={}",
                    self.model,
                    self.provider_id,
                    iteration,
                    self.scope.is_some(),
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cache_read_tokens,
                    cache_creation_tokens,
                    reasoning_tokens,
                    call_cost,
                    input.is_some(),
                    output.is_some(),
                );
                self.send(AgentProgress::ModelCallCompleted {
                    model: self.model.clone(),
                    provider_id: self.provider_id.clone(),
                    subagent_task_id: self.scope.as_ref().map(|s| s.task_id.clone()),
                    input: input.clone(),
                    output: output.clone(),
                    iteration,
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cached_input_tokens: usage.cache_read_tokens,
                    cache_creation_tokens,
                    reasoning_tokens,
                    cost_usd: call_cost,
                });
            }
            AgentEvent::CostRecorded { cost } => {
                tracing::debug!(
                    cost = ?cost,
                    "[tinyagents] cost event observed without OpenHuman accounting side effect"
                );
            }
            AgentEvent::BudgetReserved {
                estimated_input_tokens,
            } => {
                tracing::debug!(
                    estimated_input_tokens,
                    "[tinyagents] budget reserved estimated input tokens"
                );
            }
            AgentEvent::BudgetReconciled {
                estimated_input_tokens,
                actual_input_tokens,
            } => {
                tracing::debug!(
                    estimated_input_tokens,
                    actual_input_tokens,
                    "[tinyagents] budget reservation reconciled"
                );
            }
            AgentEvent::BudgetWarning { reason } => {
                tracing::debug!(
                    reason,
                    "[tinyagents] budget warning observed without run interruption"
                );
            }
            AgentEvent::BudgetExceeded { reason, blocked } => {
                tracing::debug!(
                    reason,
                    blocked,
                    "[tinyagents] budget exceeded event observed"
                );
            }
            AgentEvent::Steered {
                command_kind,
                accepted,
            } => {
                // Grep-friendly `[steering]` projection of every drained steering
                // command (issue #4249, 07.3). A rejected command means the run's
                // `SteeringPolicy` refused the kind and the crate is aborting the
                // run with `TinyAgentsError::Steering`, so surface it louder. The
                // bespoke ack plumbing in `harness/run_queue/` stays live (gated:
                // web-channel followup/parallel still need a local owner); UI
                // projection of this event remains pending.
                if *accepted {
                    tracing::debug!(
                        command_kind = command_kind.as_str(),
                        accepted,
                        "[steering] command applied at safe boundary"
                    );
                } else {
                    tracing::warn!(
                        command_kind = command_kind.as_str(),
                        accepted,
                        "[steering] command rejected by run policy"
                    );
                }
            }
            AgentEvent::ToolsFiltered {
                by,
                excluded,
                remaining,
            } => {
                tracing::debug!(
                    policy = by.as_str(),
                    excluded_tools = ?excluded,
                    remaining,
                    "[tinyagents] model-visible tools filtered"
                );
            }
            AgentEvent::Compressed {
                from_tokens,
                to_tokens,
            } => {
                tracing::debug!(
                    from_tokens,
                    to_tokens,
                    saved_tokens = from_tokens.saturating_sub(*to_tokens),
                    "[tinyagents] context compressed before model call"
                );
            }
            AgentEvent::UnknownToolCall {
                call_id,
                requested_name,
                arguments,
                recovery,
            } => {
                tracing::debug!(
                    call_id = call_id.as_str(),
                    requested_tool = requested_name.as_str(),
                    recovery = recovery.as_str(),
                    arguments = %arguments,
                    "[tinyagents] recovered unknown tool call without executing a tool"
                );
                // #4118: surface the *attempted* unavailable tool on the timeline
                // as a failed call so the UI shows what the agent tried (and
                // recovered from) rather than silently dropping it — the crate
                // recovers the call without ever emitting Started/Completed for it,
                // so nothing else in this bridge projects it. Two rows (start +
                // failed-complete) keyed by the same call_id, mirroring a real
                // tool call. Classified `Unknown` (recoverable) — the model got the
                // "valid tools: [...]" corrective and can retry a real tool.
                let iteration = self.iteration();
                let failure = Some(crate::openhuman::tool_status::describe(
                    crate::openhuman::tool_status::ToolFailureClass::Unknown,
                ));
                let label = format!("{} (unavailable)", humanize_tool_name(requested_name));
                match &self.scope {
                    None => {
                        self.send(AgentProgress::ToolCallStarted {
                            call_id: call_id.as_str().to_string(),
                            tool_name: requested_name.clone(),
                            arguments: arguments.clone(),
                            iteration,
                            display_label: Some(label),
                            display_detail: Some("tool not available".to_string()),
                        });
                        self.send(AgentProgress::ToolCallCompleted {
                            call_id: call_id.as_str().to_string(),
                            tool_name: requested_name.clone(),
                            success: false,
                            output_chars: 0,
                            output: String::new(),
                            arguments: Some(arguments.clone()),
                            elapsed_ms: 0,
                            iteration,
                            failure,
                        });
                    }
                    Some(s) => {
                        self.send(AgentProgress::SubagentToolCallStarted {
                            agent_id: s.agent_id.clone(),
                            task_id: s.task_id.clone(),
                            call_id: call_id.as_str().to_string(),
                            tool_name: requested_name.clone(),
                            arguments: arguments.clone(),
                            iteration,
                            display_label: Some(label),
                            display_detail: Some("tool not available".to_string()),
                        });
                        self.send(AgentProgress::SubagentToolCallCompleted {
                            agent_id: s.agent_id.clone(),
                            task_id: s.task_id.clone(),
                            call_id: call_id.as_str().to_string(),
                            tool_name: requested_name.clone(),
                            success: false,
                            output_chars: 0,
                            output: String::new(),
                            arguments: Some(arguments.clone()),
                            elapsed_ms: 0,
                            iteration,
                            failure,
                        });
                    }
                }
            }
            AgentEvent::ToolStarted { call_id, tool_name } => {
                // Unknown/invisible tool calls no longer produce a sentinel-named
                // Started event: the migration replaced `UNKNOWN_TOOL_SENTINEL` +
                // `UnknownToolRewriteMiddleware` with the crate
                // `UnknownToolPolicy::ReturnToolError` path (01.2), which recovers
                // the call and emits `AgentEvent::UnknownToolCall` (handled above)
                // instead of a rewritten ToolStarted. So this arm fires only for
                // real, model-visible tools and needs no sentinel guard.
                let iteration = self.iteration();
                // Stamp the start instant so the completion event carries a real
                // elapsed_ms (the crate's ToolCompleted has no timing payload).
                self.tool_started_at
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(call_id.as_str().to_string(), std::time::Instant::now());
                match &self.scope {
                    None => self.send(AgentProgress::ToolCallStarted {
                        call_id: call_id.as_str().to_string(),
                        tool_name: tool_name.clone(),
                        arguments: serde_json::Value::Null,
                        iteration,
                        display_label: Some(humanize_tool_name(tool_name)),
                        display_detail: None,
                    }),
                    Some(s) => self.send(AgentProgress::SubagentToolCallStarted {
                        agent_id: s.agent_id.clone(),
                        task_id: s.task_id.clone(),
                        call_id: call_id.as_str().to_string(),
                        tool_name: tool_name.clone(),
                        arguments: serde_json::Value::Null,
                        iteration,
                        display_label: Some(humanize_tool_name(tool_name)),
                        display_detail: None,
                    }),
                }
            }
            AgentEvent::ToolCompleted {
                call_id,
                tool_name,
                input,
                output,
                // `started_at_ms`/`duration_ms`/`output_bytes`/`error` now ride
                // the crate event (tinyagents 1.7 / tinyagents#18). The bridge
                // still reads its richer side channels below to preserve current
                // success/duration/size behavior; adopting crate fields directly
                // is C4 slice S1.
                ..
            } => {
                let iteration = self.iteration();
                // The crate event carries no success/error, so read what the
                // outcome-capture middleware classified for this call. Absent →
                // the event was projected before the middleware ran; assume
                // success (never worse than the previous hardcoded `true`).
                let outcome = self
                    .failure_map
                    .lock()
                    .ok()
                    .and_then(|mut m| m.remove(call_id.as_str()));
                let success = outcome.as_ref().map(|(ok, ..)| *ok).unwrap_or(true);
                // Real execution duration + output size the capture middleware
                // recorded off the `ToolResult` (#4467, item 4). Fall back to
                // the bridge's own ToolStarted stamp for duration, and to the
                // captured payload for size, when the middleware ran late.
                let stamped_elapsed = self
                    .tool_started_at
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(call_id.as_str())
                    .map(|t| t.elapsed().as_millis() as u64)
                    .unwrap_or(0);
                let elapsed_ms = outcome
                    .as_ref()
                    .map(|(_, _, e, _)| *e)
                    .filter(|e| *e > 0)
                    .unwrap_or(stamped_elapsed);
                // Tool result text, captured by the harness when
                // `RunPolicy.capture.tool_io` is on (the loop emits it as a
                // JSON string). Empty when capture is off.
                let output_text = match output {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                };
                let output_chars = outcome
                    .as_ref()
                    .map(|(_, _, _, c)| *c)
                    .filter(|c| *c > 0)
                    .unwrap_or_else(|| output_text.chars().count());
                // Carry the classified failure onto whichever completion event
                // this projects — main-agent OR sub-agent (#4459). Previously
                // the sub-agent branch dropped it on the floor.
                let failure = outcome.and_then(|(_, f, _, _)| f);
                match &self.scope {
                    None => self.send(AgentProgress::ToolCallCompleted {
                        call_id: call_id.as_str().to_string(),
                        tool_name: tool_name.clone(),
                        success,
                        output_chars,
                        output: output_text,
                        arguments: input.clone(),
                        elapsed_ms,
                        iteration,
                        failure,
                    }),
                    Some(s) => self.send(AgentProgress::SubagentToolCallCompleted {
                        agent_id: s.agent_id.clone(),
                        task_id: s.task_id.clone(),
                        call_id: call_id.as_str().to_string(),
                        tool_name: tool_name.clone(),
                        success,
                        output_chars,
                        output: output_text,
                        arguments: input.clone(),
                        elapsed_ms,
                        iteration,
                        failure,
                    }),
                }
            }
            // Response-cache accounting (issue #4249, 03.2). A hit means the
            // harness served this model call from its local `ResponseCache`
            // without invoking the provider (deterministic internal runs only —
            // interactive chat never attaches a cache). Counters are additive; the
            // cost-footer DTO wiring is a follow-up (workstream 06).
            AgentEvent::CacheHit { call_id, key } => {
                {
                    let mut s = self.state.lock().unwrap();
                    s.cache_hits += 1;
                }
                tracing::debug!(
                    model = %self.model,
                    call_id = call_id.as_str(),
                    key = key.as_str(),
                    "[cache] response-cache hit — provider call skipped"
                );
            }
            AgentEvent::CacheMiss { call_id, key } => {
                {
                    let mut s = self.state.lock().unwrap();
                    s.cache_misses += 1;
                }
                tracing::debug!(
                    model = %self.model,
                    call_id = call_id.as_str(),
                    key = key.as_str(),
                    "[cache] response-cache miss — invoking provider and storing result"
                );
            }
            // Retry/fallback parity (issue #4249, Workstream 02.2). These surface the
            // SDK-owned reliability decisions on the observability bridge so they are
            // no longer silently dropped by the catch-all below. `RetryScheduled` is
            // emitted by the crate's model-retry loop; with the retry pin at a single
            // attempt (`RunPolicy.retry.max_attempts = 1`, pending `ReliableProvider`
            // removal) it will not fire on the live path yet, but the bridge is wired
            // for when it does. `FallbackSelected` is emitted by
            // [`FallbackObserverMiddleware`](super::routes::FallbackObserverMiddleware)
            // whenever the harness fails over to a sibling workload-tier route.
            AgentEvent::RetryScheduled { call_id, attempt } => {
                tracing::info!(
                    model = %self.model,
                    call_id = call_id.as_str(),
                    attempt,
                    "[models] SDK scheduled a model-call retry after a retryable provider error"
                );
            }
            AgentEvent::FallbackSelected { from, to } => {
                tracing::info!(
                    model = %self.model,
                    from = from.as_str(),
                    to = to.as_str(),
                    "[fallback] SDK failed over to a cross-route fallback model"
                );
            }
            other => {
                // Not projected into `AgentProgress` (run lifecycle, sub-agent
                // boundaries — reconstructed from the orchestration tools'
                // manual emits — middleware, workspace, memory, limits). Trace
                // the kind so a dropped-event hypothesis is checkable from
                // logs instead of reading this match.
                tracing::trace!(
                    model = %self.model,
                    kind = ?std::mem::discriminant(other),
                    "[tinyagents:bridge] event observed but not forwarded to UI progress"
                );
            }
        }
    }
}

/// Surface the crate `PromptCacheGuardMiddleware`'s recorded
/// [`CacheLayoutEvent`]s as structured `[cache]` warnings (issue #4249, 03.2).
///
/// The guard records a layout event whenever the cacheable prompt prefix changes
/// between turns (volatile content — a timestamp, uuid, injected memory, etc. —
/// silently busting the provider KV-cache prefix). This is the crate-native
/// replacement for the deleted `CacheAlignMiddleware` free-text warn-log:
/// instead of a token-pattern heuristic it reports the exact before/after
/// cacheable segment ids. Drained by the turn loop after the run and logged
/// here. The warn-only `CacheAlignMiddleware` shadow was deleted in C3; this
/// guard is now the sole owner of KV-cache-prefix drift detection.
pub(crate) fn surface_cache_layout_events(model: &str, events: &[CacheLayoutEvent]) {
    for event in events {
        tracing::warn!(
            model,
            changed_prefix = event.changed_prefix,
            volatile_only = event.volatile_only,
            segments_before = ?event.segment_ids_before,
            segments_after = ?event.segment_ids_after,
            "[cache] prompt-cache prefix changed across turns — KV-cache prefix may not hit; keep dynamic content out of the system prompt / stable tool set"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tinyagents::harness::events::EventSink;

    #[tokio::test]
    async fn bridge_forwards_tool_and_cost_progress() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
        let sink = EventSink::new();
        sink.subscribe(bridge.clone());

        sink.emit(AgentEvent::ModelStarted {
            call_id: "c1".into(),
            model: "mock-model".to_string(),
        });
        sink.emit(AgentEvent::ToolStarted {
            call_id: "c1".into(),
            tool_name: "echo".to_string(),
        });
        sink.emit(AgentEvent::ToolCompleted {
            call_id: "c1".into(),
            tool_name: "echo".to_string(),
            started_at_ms: None,
            input: None,
            output: None,
            duration_ms: None,
            output_bytes: None,
            error: None,
        });
        sink.emit(AgentEvent::UsageRecorded {
            usage: Usage::new(100, 40),
        });

        let mut kinds = Vec::new();
        while let Ok(p) = rx.try_recv() {
            kinds.push(match p {
                AgentProgress::IterationStarted { .. } => "iter",
                AgentProgress::ToolCallStarted { .. } => "tool_start",
                AgentProgress::ToolCallCompleted { .. } => "tool_done",
                AgentProgress::TurnCostUpdated { input_tokens, .. } => {
                    assert_eq!(input_tokens, 100);
                    "cost"
                }
                _ => "other",
            });
        }
        assert!(kinds.contains(&"iter"));
        assert!(kinds.contains(&"tool_start"));
        assert!(kinds.contains(&"tool_done"));
        assert!(kinds.contains(&"cost"));

        let (input, output, _) = bridge.totals();
        assert_eq!((input, output), (100, 40));
    }

    #[tokio::test]
    async fn model_completed_projects_generation_with_content_and_provider() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let bridge = OpenhumanEventBridge::with_scope(
            Some(tx),
            "chat-v1",
            "managed",
            10,
            None,
            Arc::default(),
            Arc::default(),
            Arc::default(),
            Arc::default(),
        );
        let sink = EventSink::new();
        sink.subscribe(bridge.clone());

        sink.emit(AgentEvent::ModelStarted {
            call_id: "m1".into(),
            model: "chat-v1".to_string(),
        });
        sink.emit(AgentEvent::ModelCompleted {
            call_id: "m1".into(),
            started_at_ms: None,
            usage: Some(Usage::new(1_000, 50)),
            input: Some(serde_json::json!([
                {"role": "system", "content": "You are OpenHuman."}
            ])),
            output: Some(serde_json::json!({"role": "assistant", "content": "hi"})),
        });

        let mut seen = None;
        while let Ok(p) = rx.try_recv() {
            if let AgentProgress::ModelCallCompleted {
                model,
                provider_id,
                subagent_task_id,
                input,
                output,
                input_tokens,
                cost_usd,
                ..
            } = p
            {
                seen = Some((
                    model,
                    provider_id,
                    subagent_task_id,
                    input,
                    output,
                    input_tokens,
                    cost_usd,
                ));
            }
        }
        let (model, provider_id, task, input, output, input_tokens, cost_usd) =
            seen.expect("ModelCallCompleted projected from ModelCompleted");
        assert_eq!(model, "chat-v1");
        assert_eq!(provider_id, "managed");
        assert!(task.is_none(), "parent scope carries no task id");
        assert!(input.unwrap().to_string().contains("You are OpenHuman."));
        assert!(output.unwrap().to_string().contains("hi"));
        assert_eq!(input_tokens, 1_000);
        // chat-v1 is a managed tier handle — the tier-aware estimator must
        // price it (> $0); the old catalog-only lookup returned exactly 0.
        assert!(cost_usd > 0.0, "managed tier call must not price as $0");
    }

    #[tokio::test]
    async fn subagent_model_completed_carries_task_attribution() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let bridge = OpenhumanEventBridge::with_scope(
            Some(tx),
            "burst-v1",
            "managed",
            8,
            Some(SubagentScope {
                agent_id: "context_scout".to_string(),
                task_id: "ctx-1".to_string(),
                extended_policy: true,
            }),
            Arc::default(),
            Arc::default(),
            Arc::default(),
            Arc::default(),
        );
        let sink = EventSink::new();
        sink.subscribe(bridge.clone());
        sink.emit(AgentEvent::ModelCompleted {
            call_id: "m1".into(),
            started_at_ms: None,
            usage: Some(Usage::new(10, 5)),
            input: None,
            output: None,
        });
        let mut task = None;
        while let Ok(p) = rx.try_recv() {
            if let AgentProgress::ModelCallCompleted {
                subagent_task_id, ..
            } = p
            {
                task = subagent_task_id;
            }
        }
        assert_eq!(
            task.as_deref(),
            Some("ctx-1"),
            "child model calls must carry the owning subagent task id"
        );
    }

    #[tokio::test]
    async fn tool_completed_projects_output_arguments_and_elapsed() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
        let sink = EventSink::new();
        sink.subscribe(bridge.clone());

        sink.emit(AgentEvent::ToolStarted {
            call_id: "t1".into(),
            tool_name: "echo".to_string(),
        });
        sink.emit(AgentEvent::ToolCompleted {
            call_id: "t1".into(),
            tool_name: "echo".to_string(),
            started_at_ms: None,
            input: Some(serde_json::json!({"text": "ping"})),
            output: Some(serde_json::Value::String("pong".to_string())),
            duration_ms: None,
            output_bytes: None,
            error: None,
        });

        let mut seen = None;
        while let Ok(p) = rx.try_recv() {
            if let AgentProgress::ToolCallCompleted {
                output,
                output_chars,
                arguments,
                ..
            } = p
            {
                seen = Some((output, output_chars, arguments));
            }
        }
        let (output, output_chars, arguments) = seen.expect("tool completion projected");
        assert_eq!(output, "pong");
        assert_eq!(output_chars, 4);
        assert!(arguments.unwrap().to_string().contains("ping"));
    }

    #[tokio::test]
    async fn unknown_tool_call_projects_attempted_name_as_failed_timeline_row() {
        // #4118: the crate recovers an unavailable-tool call via ReturnToolError
        // without ever emitting Started/Completed for it. The bridge must still
        // surface the *attempted* tool on the timeline (a failed call) so the UI
        // shows what the agent tried, instead of the attempt vanishing.
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
        let sink = EventSink::new();
        sink.subscribe(bridge.clone());

        sink.emit(AgentEvent::UnknownToolCall {
            call_id: "c9".into(),
            requested_name: "search_files".to_string(),
            arguments: serde_json::json!({ "query": "config" }),
            recovery: "tool_error".to_string(),
        });

        let mut started_name = None;
        let mut completed: Option<(String, bool)> = None;
        while let Ok(p) = rx.try_recv() {
            match p {
                AgentProgress::ToolCallStarted { tool_name, .. } => started_name = Some(tool_name),
                AgentProgress::ToolCallCompleted {
                    tool_name, success, ..
                } => completed = Some((tool_name, success)),
                _ => {}
            }
        }
        assert_eq!(
            started_name.as_deref(),
            Some("search_files"),
            "the attempted unavailable tool name must appear on the timeline"
        );
        assert_eq!(
            completed,
            Some(("search_files".to_string(), false)),
            "the attempted tool must be projected as a *failed* call"
        );
    }

    /// W2-budget-dedupe: two `UsageRecorded` events for the *same* model call
    /// (as happens once the observe-only crate `BudgetMiddleware` re-emits usage
    /// its `after_model` folded, on top of the runtime's own emit) must be
    /// recorded into the bridge accounting **exactly once**. Without the dedupe
    /// guard the totals would double.
    #[tokio::test]
    async fn duplicate_usage_for_same_model_call_is_recorded_once() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
        let sink = EventSink::new();
        sink.subscribe(bridge.clone());

        // One model call → one `ModelStarted` (iteration cursor → 1).
        sink.emit(AgentEvent::ModelStarted {
            call_id: "c1".into(),
            model: "mock-model".to_string(),
        });
        // Same call surfaces usage twice (runtime emit + BudgetMiddleware re-emit).
        sink.emit(AgentEvent::UsageRecorded {
            usage: Usage::new(100, 40),
        });
        sink.emit(AgentEvent::UsageRecorded {
            usage: Usage::new(100, 40),
        });

        // Totals reflect a single record, not two.
        let (input, output, _) = bridge.totals();
        assert_eq!(
            (input, output),
            (100, 40),
            "the duplicate UsageRecorded for the same iteration must be skipped"
        );

        // Exactly one `TurnCostUpdated` footer emit for the call.
        let mut cost_updates = 0;
        while let Ok(p) = rx.try_recv() {
            if matches!(p, AgentProgress::TurnCostUpdated { .. }) {
                cost_updates += 1;
            }
        }
        assert_eq!(cost_updates, 1, "footer must update once per model call");

        // A genuinely new model call (iteration cursor → 2) records again.
        sink.emit(AgentEvent::ModelStarted {
            call_id: "c2".into(),
            model: "mock-model".to_string(),
        });
        sink.emit(AgentEvent::UsageRecorded {
            usage: Usage::new(10, 5),
        });
        let (input, output, _) = bridge.totals();
        assert_eq!(
            (input, output),
            (110, 45),
            "a distinct model call (new iteration) must still record"
        );
    }

    // NOTE: the former `sentinel_tool_started_is_not_forwarded` test was removed
    // here. The #4249 migration (commit 60097ba8d, "use sdk unknown tool
    // recovery") deleted `UNKNOWN_TOOL_SENTINEL` + `UnknownToolRewriteMiddleware`
    // in favour of the crate `UnknownToolPolicy::ReturnToolError` path, so a
    // `ToolStarted` now only ever fires for real, model-visible tools (see the
    // `ToolStarted` arm above — it no longer special-cases a sentinel). The test
    // referenced the deleted constant (a stale reference reintroduced by a merge)
    // and asserted behaviour that no longer exists.
}

/// A [`GraphEventSink`] that mirrors the `tinyagents` graph executor's lifecycle
/// stream onto openhuman's `tracing` diagnostics — an observability journal for
/// graph runs (issue #4249 / #28). Node/step/run/route transitions land as
/// grep-friendly `[graph]` lines tagged with `label`; the running event count is
/// exposed for tests. Shared by every openhuman graph (council fan-out,
/// sub-agent delegation, …).
pub(crate) struct GraphTracingSink {
    label: String,
    count: Arc<std::sync::atomic::AtomicUsize>,
}

impl GraphTracingSink {
    /// Build a sink tagging its lines with `label` (e.g. `"delegation:graph"`).
    /// Accepts both string literals and runtime-built labels.
    pub(crate) fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Shared counter of events observed, for assertions.
    fn counter(&self) -> Arc<std::sync::atomic::AtomicUsize> {
        self.count.clone()
    }
}

impl GraphEventSink for GraphTracingSink {
    fn emit(&self, event: GraphEvent) {
        self.count.fetch_add(1, Ordering::Relaxed);
        let label = self.label.as_str();
        match &event {
            GraphEvent::RunStarted { run_id } => {
                tracing::debug!(label, ?run_id, "[graph] run started")
            }
            GraphEvent::RunCompleted { steps, .. } => {
                tracing::debug!(label, steps, "[graph] run completed")
            }
            GraphEvent::RunFailed { error, .. } => {
                tracing::warn!(label, %error, "[graph] run failed")
            }
            GraphEvent::NodeStarted { node, step } => {
                tracing::debug!(label, ?node, step, "[graph] node started")
            }
            GraphEvent::NodeCompleted { node, step } => {
                tracing::debug!(label, ?node, step, "[graph] node completed")
            }
            GraphEvent::NodeFailed { node, error, .. } => {
                tracing::warn!(label, ?node, %error, "[graph] node failed")
            }
            GraphEvent::RouteSelected { node, target } => {
                tracing::trace!(label, ?node, ?target, "[graph] route selected")
            }
            _ => tracing::trace!(label, "[graph] event"),
        }
    }
}
