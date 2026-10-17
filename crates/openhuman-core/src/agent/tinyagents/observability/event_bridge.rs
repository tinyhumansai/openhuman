//! [`OpenhumanEventBridge`] — the [`EventListener`] that mirrors the
//! `tinyagents` harness event stream onto openhuman's [`AgentProgress`] +
//! cost tracker (issue #4249). See the parent module doc for the overview.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::Sender;

use tinyinference_llm::usage::Usage;

use crate::agent::progress::AgentProgress;
use crate::inference::provider::BilledUsage;
use tinytools::humanize_tool_name;

use super::bridge_types::{ProviderUsageCarry, SubagentScope, ToolFailureMap, ToolNameMap};
use tinyagents_harness::events::{CallDedupe, IterationCursor, OrderedSender};

#[derive(Default)]
pub(super) struct BridgeState {
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) cached_input_tokens: u64,
    /// Every call's cost: reported charge, catalog estimate, or unknown.
    pub(super) cost: tinyagents_harness::cost::CostTally,
    /// Local response-cache hits observed on this turn (issue #4249, 03.2). A hit
    /// means the harness served a model call from its [`ResponseCache`] without
    /// invoking the provider. Additive counters — a follow-up (coordinated with
    /// workstream 06) wires these into the cost-footer DTO; today they are logged
    /// with a grep-friendly `[cache]` prefix and exposed via [`OpenhumanEventBridge::cache_counts`].
    pub(super) cache_hits: u64,
    /// Local response-cache misses observed on this turn (provider *was* invoked).
    pub(super) cache_misses: u64,
}

/// Per-model-call figures `record_usage` resolved from the provider-usage
/// carry (charged>estimate cost precedence + cache/reasoning breakdown),
/// keyed by iteration so the subsequent `ModelCompleted` projection reports
/// the same numbers as the wallet accounting.
#[derive(Clone, Copy, Debug)]
pub(super) struct ResolvedCallFigures {
    /// `None` when the call's cost is unknown.
    pub(super) cost_usd: Option<f64>,
    pub(super) cache_creation_tokens: u64,
    pub(super) reasoning_tokens: u64,
}

/// An [`EventListener`] that mirrors harness events onto openhuman's progress
/// sink and cost tracker.
pub(crate) struct OpenhumanEventBridge {
    pub(super) model: String,
    /// Telemetry provider id (`"managed"`, `"openai"`, …) — from
    /// [`Provider::telemetry_provider_id`](crate::inference::provider::Provider::telemetry_provider_id).
    /// Rides on `ModelCallCompleted` so trace exporters render the Langfuse
    /// model as `{provider_id}.{model}`.
    pub(super) provider_id: String,
    pub(super) max_iterations: u32,
    /// `None` for a parent turn; `Some` to emit child-scoped `Subagent*` events.
    pub(super) scope: Option<SubagentScope>,
    /// The agent definition the run acts as (its memory identity's agent),
    /// attributed on usage records of a top-level turn. Set once by the turn
    /// that owns the run ([`Self::set_definition`]).
    pub(super) definition: std::sync::OnceLock<String>,
    /// Shared with the model adapter so thinking deltas line up with the
    /// model call (iteration) they belong to.
    pub(super) cursor: IterationCursor,
    /// Shared `call_id → tool_name` map written by the model adapter's
    /// `ThinkingForwarder` on tool-call start; read here to label the
    /// incremental tool-argument fragments projected off the crate stream.
    pub(super) tool_names: ToolNameMap,
    /// Shared `call_id → (success, failure, elapsed_ms, output_chars)`
    /// side-channel written by `ToolOutcomeCaptureMiddleware`; read when
    /// projecting `ToolCallCompleted`.
    pub(super) failure_map: ToolFailureMap,
    /// Shared FIFO carry of the per-call provider `BilledUsage` the model adapter
    /// observed; drained in `record_usage` to restore backend-charged USD +
    /// context-window + cache-creation/reasoning tokens the crate `Usage` drops.
    pub(super) usage_carry: ProviderUsageCarry,
    /// Model-call iterations whose `UsageRecorded` has already been folded into
    /// the global cost tracker (W2-budget-dedupe). A single model call can now
    /// surface **two** `UsageRecorded` events — one from the harness runtime
    /// (`agent_loop`, always) and one from the observe-only crate
    /// `BudgetMiddleware::after_model` — both carrying identical usage and both
    /// delivered to this bridge. Keyed on the run-scoped model-call identity (the
    /// iteration cursor, bumped once per `ModelStarted`) so a given call's usage
    /// is recorded exactly once. See [`OpenhumanEventBridge::record_usage`].
    pub(super) recorded_iterations: CallDedupe,
    /// Per-iteration figures resolved by `record_usage` (see
    /// [`ResolvedCallFigures`]); taken by the `ModelCompleted` arm.
    pub(super) resolved_calls: Mutex<std::collections::HashMap<u32, ResolvedCallFigures>>,
    /// `call_id → start instant` for in-flight tool calls, written on
    /// `ToolStarted` and taken on `ToolCompleted` so the projected completion
    /// event carries a real `elapsed_ms` (the crate event has no timing).
    pub(super) tool_started_at: Mutex<std::collections::HashMap<String, std::time::Instant>>,
    /// `call_id → requested tool name` for calls the crate answered as an
    /// unknown tool (`AgentEvent::UnknownToolCall`, recovery `tool_error`).
    /// The crate then emits an ordinary `ToolStarted`/`ToolCompleted` pair for
    /// the same call id (`recover_tool_call`); this map lets that pair carry
    /// the "unavailable" label and the `NotFound` class instead of the bridge
    /// projecting a second row of its own.
    pub(super) unknown_calls: Mutex<std::collections::HashMap<String, String>>,
    /// The turn's registered tool sets, retained (cheap `Arc` clones — never
    /// the tools themselves) so the bridge can resolve a live `&dyn Tool` by
    /// name and call its own [`tinytools::Tool::display_label`] /
    /// [`tinytools::Tool::display_detail`] instead of only ever guessing from
    /// the bare tool name (issue: tool-call presentation). Empty for a bridge
    /// built without a turn's tool sets (e.g. a bare unit-test bridge), in
    /// which case every lookup falls back to [`humanize_tool_name`].
    pub(super) tool_sets: Vec<Arc<Vec<Box<dyn tinytools::Tool>>>>,
    pub(super) state: Mutex<BridgeState>,
    /// Ordered overflow buffer for progress events that hit backpressure
    /// (channel `Full`). Once ANY event spills here, `draining` stays set and
    /// every subsequent event queues here too — a single spawned forwarder
    /// drains them to the channel in FIFO order — so a later fast-path
    /// `try_send` can never jump ahead of an earlier spilled event and scramble
    /// start/completed ordering (which would leave a tool row stuck `running`
    /// when a `ToolCallCompleted` overtakes its `ToolCallStarted`) (#4466).
    pub(super) sender: OrderedSender<AgentProgress>,
    /// The context the run acts under; usage records take their tenant from it.
    pub(super) dispatch: crate::core::runtime::DispatchCtx,
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
            Vec::new(),
            crate::core::runtime::DispatchCtx::unbound(),
        )
    }

    /// Build a bridge, optionally child-scoped, sharing `cursor` (iteration
    /// attribution) and `tool_names` (tool-call name lookup for the streamed
    /// argument fragments) with the model adapter. `tool_sets` is the turn's
    /// registered tool sets (cheap `Arc` clones), used to resolve a live
    /// `&dyn Tool` for `display_label`/`display_detail` — pass `Vec::new()`
    /// when none are available (e.g. tests).
    #[allow(clippy::too_many_arguments)]
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
        tool_sets: Vec<Arc<Vec<Box<dyn tinytools::Tool>>>>,
        dispatch: crate::core::runtime::DispatchCtx,
    ) -> Arc<Self> {
        Arc::new(Self {
            sender: OrderedSender::new(on_progress),
            model: model.into(),
            provider_id: provider_id.into(),
            max_iterations: max_iterations as u32,
            scope,
            definition: std::sync::OnceLock::new(),
            cursor,
            tool_names,
            failure_map,
            usage_carry,
            recorded_iterations: CallDedupe::default(),
            resolved_calls: Mutex::new(std::collections::HashMap::new()),
            tool_started_at: Mutex::new(std::collections::HashMap::new()),
            unknown_calls: Mutex::new(std::collections::HashMap::new()),
            tool_sets,
            state: Mutex::new(BridgeState::default()),
            dispatch,
        })
    }

    /// Names the agent definition this run acts as, for usage attribution. The
    /// first call wins.
    pub(crate) fn set_definition(&self, definition: Option<&str>) {
        if let Some(definition) = definition.filter(|definition| !definition.is_empty()) {
            let _ = self.definition.set(definition.to_owned());
        }
    }

    /// Resolve `tool_name` against the turn's registered tool sets and
    /// compute the presentation pair from the tool's OWN
    /// [`tinytools::Tool::display_label`] / [`tinytools::Tool::display_detail`]
    /// using `args` (the real call arguments when known, `Null` at call-start
    /// before they've arrived). Unknown tools (not found in any set — the
    /// unknown-tool-call path never registers one) fall back to a humanized
    /// name with no detail, matching the pre-existing behavior.
    pub(super) fn resolve_display(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> (Option<String>, Option<String>) {
        match self
            .tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == tool_name)
        {
            Some(tool) => {
                let label = tool.display_label(args);
                let detail = tool.display_detail(args);
                tracing::trace!(
                    tool_name,
                    label = ?label,
                    detail = ?detail,
                    "[tool-presentation] resolved display label/detail from registered tool"
                );
                (label, detail)
            }
            None => {
                tracing::debug!(
                    tool_name,
                    "[tool-presentation] tool not found in turn's registered sets — \
                     falling back to humanized name"
                );
                (Some(humanize_tool_name(tool_name)), None)
            }
        }
    }

    /// Cumulative `(input_tokens, output_tokens, charged_usd)` observed so far.
    #[cfg(test)]
    pub(super) fn totals(&self) -> (u64, u64, f64) {
        let s = self.state.lock().unwrap();
        (s.input_tokens, s.output_tokens, s.cost.known_usd)
    }

    /// Cumulative `(input_tokens, output_tokens, cached_input_tokens, cost)`
    /// observed so far — the full accounting the turn persists (transcript cost /
    /// session meters). `cost` says whether every call was priced.
    pub(crate) fn totals_with_cost(&self) -> (u64, u64, u64, tinyagents_harness::cost::CostTally) {
        let s = self.state.lock().unwrap();
        (
            s.input_tokens,
            s.output_tokens,
            s.cached_input_tokens,
            s.cost,
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
    /// **synchronous**, so the harness `OrderedSender` tries `try_send` first and,
    /// when the channel is full, spills to a bounded FIFO drain task so later
    /// items never overtake earlier ones. A closed channel means the receiver is
    /// gone (turn tore down), where dropping is correct. If the bounded overflow
    /// queue itself is full the delta is dropped and logged below.
    pub(super) fn send(&self, progress: AgentProgress) {
        use tinyagents_harness::events::SendFailure;
        match self.sender.send(progress) {
            Ok(()) | Err(SendFailure::Closed(_)) => {}
            Err(SendFailure::Backpressure(_)) => tracing::debug!(
                model = %self.model,
                "[tinyagents] progress channel full and could not defer send; dropping one delta"
            ),
        }
    }

    pub(super) fn iteration(&self) -> u32 {
        self.cursor.load(Ordering::SeqCst)
    }

    /// Accumulate a usage block, feed the global cost tracker, and emit a
    /// `TurnCostUpdated` so the UI footer stays live.
    pub(super) fn record_usage(&self, usage: &Usage) {
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
        if !self.recorded_iterations.first_claim(iteration) {
            tracing::debug!(
                iteration,
                model = %self.model,
                child = self.scope.is_some(),
                "[budget] duplicate UsageRecorded for model call — skipping double record"
            );
            return;
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

        // The provider's reported charge wins; without one, the catalog's list
        // price; without that, the call's cost is unknown (never a default
        // rate).
        let provider_cost = carried
            .as_ref()
            .filter(|u| u.charge_reported && !u.cost_is_estimate)
            .map(|u| u.charged_amount_usd)
            .filter(|c| c.is_finite() && *c >= 0.0);
        let cost = match provider_cost {
            Some(charged) => tinyagents_harness::cost::CallCost::Charged(charged),
            None => match Self::estimate_call_cost(&self.model, usage) {
                Some(estimate) => tinyagents_harness::cost::CallCost::Estimated(estimate),
                None => tinyagents_harness::cost::CallCost::Unknown,
            },
        };
        let call_cost = cost.usd();
        // The context window + cache-creation/reasoning breakdown only exist on
        // the carried provider usage (the crate `Usage` mapping drops them); fall
        // back to the catalogue window and the crate token counts when absent.
        let context_window = carried
            .as_ref()
            .map(|u| u.context_window())
            .filter(|w| *w > 0)
            .unwrap_or_else(|| {
                tinyagents_registry::pricing::lookup(&self.model)
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
            ?cost,
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
            s.cost.add(cost);
            (
                s.input_tokens,
                s.output_tokens,
                s.cached_input_tokens,
                s.cost.known_usd,
            )
        };

        // Feed the authoritative global cost tracker (same call the legacy
        // observer made), so the wallet/cost surfaces stay accurate.
        let usage_info = BilledUsage::from_counts(usage.input_tokens, usage.output_tokens)
            .with_context_window(context_window)
            .with_cached_input_tokens(usage.cache_read_tokens)
            .with_cache_creation_tokens(cache_creation_tokens)
            .with_reasoning_tokens(reasoning_tokens);
        let usage_info = match cost {
            tinyagents_harness::cost::CallCost::Charged(usd) => usage_info.with_charged_usd(usd),
            tinyagents_harness::cost::CallCost::Estimated(usd) => {
                usage_info.with_estimated_usd(usd)
            }
            tinyagents_harness::cost::CallCost::Unknown => usage_info,
        };
        if reasoning_tokens > 0 || cache_creation_tokens > 0 {
            log::debug!(
                "[cost] recording reasoning/cache-creation tokens model={} reasoning_tokens={} cache_creation_tokens={}",
                self.model,
                reasoning_tokens,
                cache_creation_tokens
            );
        }
        let subagent = self
            .scope
            .as_ref()
            .map(|child| (child.agent_id.as_str(), child.task_id.as_str()));
        crate::platform::cost::record_provider_usage_scoped(
            &self.model,
            &usage_info,
            crate::platform::cost::UsageScope::in_ctx(
                &self.dispatch,
                Some(&self.provider_id),
                subagent,
            )
            .with_definition(self.definition.get().map(String::as_str)),
        );

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
    /// [`agent::cost`](crate::agent::cost) table (managed handles
    /// like `hint:chat`/`hint:burst` + the vendor catalog + heuristics) — the
    /// previous `tinyagents_registry::pricing::estimate_cost_usd` only knew concrete vendor
    /// ids, so every managed-tier call priced as $0 in traces and the footer.
    /// `None` for a model the catalog does not price.
    pub(super) fn estimate_call_cost(model: &str, usage: &Usage) -> Option<f64> {
        crate::agent::cost::estimate_call_cost_usd(
            model,
            &BilledUsage::from_counts(usage.input_tokens, usage.output_tokens)
                .with_cached_input_tokens(usage.cache_read_tokens)
                .with_cache_creation_tokens(usage.cache_creation_tokens)
                .with_reasoning_tokens(usage.reasoning_tokens),
        )
    }
}
