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

/// An [`EventListener`] that mirrors harness events onto openhuman's progress
/// sink and cost tracker.
pub(crate) struct OpenhumanEventBridge {
    on_progress: Option<Sender<AgentProgress>>,
    model: String,
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
    state: Mutex<BridgeState>,
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
            max_iterations,
            None,
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
        max_iterations: usize,
        scope: Option<SubagentScope>,
        cursor: IterationCursor,
        tool_names: ToolNameMap,
    ) -> Arc<Self> {
        Arc::new(Self {
            on_progress,
            model: model.into(),
            max_iterations: max_iterations as u32,
            scope,
            cursor,
            tool_names,
            state: Mutex::new(BridgeState::default()),
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

    /// Best-effort, non-blocking progress emit (drops on a full channel, like
    /// the legacy streaming path).
    fn send(&self, progress: AgentProgress) {
        if let Some(tx) = &self.on_progress {
            let _ = tx.try_send(progress);
        }
    }

    fn iteration(&self) -> u32 {
        self.cursor.load(Ordering::SeqCst)
    }

    /// Accumulate a usage block, feed the global cost tracker, and emit a
    /// `TurnCostUpdated` so the UI footer stays live.
    fn record_usage(&self, usage: &Usage) {
        let iteration = self.iteration();
        // Provider-reported charged USD has no home in the crate `Usage` (all
        // token counts), so estimate this call's cost from catalogued per-MTok
        // rates. Fixes the long-standing $0 cost on the tinyagents path, where
        // the charged amount was hardcoded to 0.0 (issue #4249, Phase 5). When a
        // provider genuinely charges (credit-metered backends) preserving that
        // exact amount needs an out-of-band carry — tracked as a follow-up.
        let call_cost = crate::openhuman::cost::catalog::estimate_cost_usd(
            &self.model,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
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
            context_window: 0,
            cached_input_tokens: usage.cache_read_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            reasoning_tokens: usage.reasoning_tokens,
            charged_amount_usd: call_cost,
        };
        if usage.reasoning_tokens > 0 || usage.cache_creation_tokens > 0 {
            log::debug!(
                "[cost] recording reasoning/cache-creation tokens model={} reasoning_tokens={} cache_creation_tokens={}",
                self.model,
                usage.reasoning_tokens,
                usage.cache_creation_tokens
            );
        }
        crate::openhuman::cost::record_provider_usage(&self.model, &usage_info);

        // The cost footer is a top-level surface; for a child run the global
        // cost tracker feed above is the authoritative accounting and the parent
        // emits its own footer, so suppress the per-child `TurnCostUpdated`.
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
                // Tool-call **argument** fragments now ride the crate stream
                // (`MessageDelta.tool_call`) instead of the out-of-band
                // `ThinkingForwarder`. Project them onto the same
                // `ToolCallArgsDelta` the UI timeline consumes so the model can
                // be shown composing the call before it executes. The crate
                // `ToolDelta` carries no `tool_name`, so we recover it from the
                // shared map the forwarder populated on the tool-call start
                // event (empty until the start marker lands — matching the
                // legacy forwarder's own default). There is no `Subagent*`
                // tool-arg variant, so child runs ride the top-level event too
                // (parity with the forwarder's prior behavior).
                if let Some(tool_call) = &delta.tool_call {
                    if !tool_call.content.is_empty() {
                        let tool_name = self
                            .tool_names
                            .lock()
                            .unwrap()
                            .get(&tool_call.call_id)
                            .cloned()
                            .unwrap_or_default();
                        tracing::trace!(
                            call_id = tool_call.call_id.as_str(),
                            tool_name = tool_name.as_str(),
                            len = tool_call.content.len(),
                            child = self.scope.is_some(),
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
            AgentEvent::ToolCompleted { call_id, tool_name } => {
                let iteration = self.iteration();
                match &self.scope {
                    None => self.send(AgentProgress::ToolCallCompleted {
                        call_id: call_id.as_str().to_string(),
                        tool_name: tool_name.clone(),
                        success: true,
                        output_chars: 0,
                        elapsed_ms: 0,
                        iteration,
                    }),
                    Some(s) => self.send(AgentProgress::SubagentToolCallCompleted {
                        agent_id: s.agent_id.clone(),
                        task_id: s.task_id.clone(),
                        call_id: call_id.as_str().to_string(),
                        tool_name: tool_name.clone(),
                        success: true,
                        output_chars: 0,
                        output: String::new(),
                        elapsed_ms: 0,
                        iteration,
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
            _ => {}
        }
    }
}

/// Surface the crate `PromptCacheGuardMiddleware`'s recorded
/// [`CacheLayoutEvent`]s as structured `[cache]` warnings (issue #4249, 03.2).
///
/// The guard records a layout event whenever the cacheable prompt prefix changes
/// between turns (volatile content — a timestamp, uuid, injected memory, etc. —
/// silently busting the provider KV-cache prefix). This is the structured
/// successor to `CacheAlignMiddleware`'s free-text warn-log: instead of a
/// token-pattern heuristic it reports the exact before/after cacheable segment
/// ids. Drained by the turn loop after the run and logged here; `CacheAlign` is
/// kept installed in parallel until parity is shown (its deletion is a gated
/// follow-up).
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
