use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::Sender;

use tinyagents_graph::stream::{GraphEvent, GraphEventSink};
use tinyagents_harness::cache::CacheLayoutEvent;
use tinyagents_harness::events::{AgentEvent, EventListener, EventRecord};
use tinyagents_harness::steering::{SteeringCommand, SteeringHandle};
use tinyinference::usage::Usage;

use crate::openhuman::agent::harness::turn_dispatch_guard::TurnDispatchState;
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
                Option<crate::openhuman::tools::status::ClassifiedFailure>,
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
    /// The current turn's dispatch guard, when this run is a turn (rather than
    /// a CLI/direct invocation). Recording the pause here is what makes it
    /// *binding* on new sub-agent dispatch instead of merely advisory — see
    /// [`crate::openhuman::agent::harness::turn_dispatch_guard`] and #5804.
    dispatch_guard: Option<Arc<TurnDispatchState>>,
}

impl CapPauser {
    /// Pause `handle` once `cap` model calls complete, recording the pause on
    /// `dispatch_guard` when the run is executing inside a turn scope.
    pub(crate) fn new(
        handle: SteeringHandle,
        cap: usize,
        dispatch_guard: Option<Arc<TurnDispatchState>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            handle,
            cap: cap as u32,
            completed: AtomicU32::new(0),
            dispatch_guard,
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
                // Record BEFORE sending the advisory command. The crate drains
                // its event queue synchronously, notifying listeners in
                // insertion order on the emitting task
                // (`vendor/tinyagents/src/harness/events/mod.rs:163-195`), so
                // this store happens-before any tool call the loop dispatches
                // afterwards. That ordering is the whole fix: the pause stops
                // being something a dispatch can race and becomes something a
                // dispatch must observe.
                if let Some(guard) = self.dispatch_guard.as_ref() {
                    guard.record_pause_requested(u64::from(n), u64::from(self.cap));
                }
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
                crate::openhuman::platform::cost::catalog::lookup(&self.model)
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
        crate::openhuman::platform::cost::record_provider_usage(&self.model, &usage_info);

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
