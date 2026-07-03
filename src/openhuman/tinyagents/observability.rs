//! Bridge the `tinyagents` harness event stream onto openhuman's
//! [`AgentProgress`] + cost tracker (issue #4249, tinyagents 0.2.0).
//!
//! 0.2.0 emits a typed [`AgentEvent`] stream (model started/delta/completed,
//! tool started/completed, usage) through an [`EventSink`] that callers attach
//! to a [`RunContext`]. This listener translates those into the same
//! `AgentProgress` events the legacy `run_turn_engine` produced — restoring the
//! live tool timeline, streaming text, and the cost/token footer on the
//! tinyagents path — and feeds per-call usage into the global cost tracker.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::Sender;

use tinyagents::graph::stream::{GraphEvent, GraphEventSink};
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
/// thinking deltas it forwards out-of-band (tinyagents 0.2.0's `MessageDelta`
/// carries no reasoning channel, so reasoning can't ride the harness stream).
pub type IterationCursor = Arc<AtomicU32>;

/// An [`EventListener`] that pauses the run once `cap` model calls have
/// completed, so the loop stops gracefully at the iteration budget (returning
/// the partial transcript) instead of erroring with `LimitExceeded`. The harness
/// checks pending steering at the top of each turn *before* the model-call limit
/// check, so a `Pause` sent here short-circuits the loop cleanly. The caller then
/// inspects the run's finish reason to decide whether to summarize a checkpoint
/// — the tinyagents analogue of the legacy `CheckpointStrategy::on_max_iter`.
pub struct CapPauser {
    handle: SteeringHandle,
    cap: u32,
    completed: AtomicU32,
}

impl CapPauser {
    /// Pause `handle` once `cap` model calls complete.
    pub fn new(handle: SteeringHandle, cap: usize) -> Arc<Self> {
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
}

/// An [`EventListener`] that mirrors harness events onto openhuman's progress
/// sink and cost tracker.
pub struct OpenhumanEventBridge {
    on_progress: Option<Sender<AgentProgress>>,
    model: String,
    max_iterations: u32,
    /// `None` for a parent turn; `Some` to emit child-scoped `Subagent*` events.
    scope: Option<SubagentScope>,
    /// Shared with the model adapter so thinking deltas line up with the
    /// model call (iteration) they belong to.
    cursor: IterationCursor,
    state: Mutex<BridgeState>,
}

impl OpenhumanEventBridge {
    /// Build a parent-scoped bridge for `model`.
    pub fn new(
        on_progress: Option<Sender<AgentProgress>>,
        model: impl Into<String>,
        max_iterations: usize,
    ) -> Arc<Self> {
        Self::with_scope(on_progress, model, max_iterations, None, Arc::default())
    }

    /// Build a bridge, optionally child-scoped, sharing `cursor` with the model
    /// adapter so out-of-band thinking deltas carry the same iteration index.
    pub fn with_scope(
        on_progress: Option<Sender<AgentProgress>>,
        model: impl Into<String>,
        max_iterations: usize,
        scope: Option<SubagentScope>,
        cursor: IterationCursor,
    ) -> Arc<Self> {
        Arc::new(Self {
            on_progress,
            model: model.into(),
            max_iterations: max_iterations as u32,
            scope,
            cursor,
            state: Mutex::new(BridgeState::default()),
        })
    }

    /// Cumulative `(input_tokens, output_tokens, charged_usd)` observed so far.
    pub fn totals(&self) -> (u64, u64, f64) {
        let s = self.state.lock().unwrap();
        (s.input_tokens, s.output_tokens, s.charged_amount_usd)
    }

    /// Cumulative `(input_tokens, output_tokens, cached_input_tokens, charged_usd)`
    /// observed so far — the full accounting the turn persists (transcript cost /
    /// session meters), so a normal turn no longer records `$0` and zero cached
    /// tokens despite real usage.
    pub fn totals_with_cost(&self) -> (u64, u64, u64, f64) {
        let s = self.state.lock().unwrap();
        (
            s.input_tokens,
            s.output_tokens,
            s.cached_input_tokens,
            s.charged_amount_usd,
        )
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
            charged_amount_usd: call_cost,
        };
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
                if !delta.text.is_empty() {
                    let iteration = self.iteration();
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
            }
            // `UsageRecorded` carries the authoritative per-call usage and fires
            // exactly once per model call; prefer it over `ModelCompleted`'s
            // optional usage to avoid double counting.
            AgentEvent::UsageRecorded { usage } => self.record_usage(usage),
            AgentEvent::ToolStarted { call_id, tool_name }
                if tool_name.as_str() != super::tools::UNKNOWN_TOOL_SENTINEL =>
            {
                // Skip the sentinel Started event. When the model calls a tool the
                // agent can't see, `UnknownToolRewriteMiddleware` rewrites the name
                // to `UNKNOWN_TOOL_SENTINEL` *before* this event fires. The frontend
                // keys tool-timeline rows by `call_id` and overwrites the name on
                // Started, so a real streamed `web_fetch` row would be clobbered to
                // the sentinel — dropping the attempted tool name from the timeline
                // (regression vs. the pre-tinyagents engine, which emitted the real
                // name before the availability block). The streamed
                // `tool_args_delta` row (carrying the attempted name) survives, and
                // the sentinel `ToolCompleted` only updates status by `call_id`.
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
            _ => {}
        }
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

    #[tokio::test]
    async fn sentinel_tool_started_is_not_forwarded() {
        // #4249 regression guard: a `ToolStarted` for the unknown-tool sentinel
        // must NOT emit a `ToolCallStarted`. The frontend keys tool-timeline rows
        // by `call_id` and overwrites the name on Started, so forwarding the
        // sentinel would clobber the real streamed row (e.g. `web_fetch`) and drop
        // the attempted tool from the UI timeline. A real tool name still forwards.
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
        let sink = EventSink::new();
        sink.subscribe(bridge.clone());

        sink.emit(AgentEvent::ToolStarted {
            call_id: "c1".into(),
            tool_name: crate::openhuman::tinyagents::tools::UNKNOWN_TOOL_SENTINEL.to_string(),
        });
        sink.emit(AgentEvent::ToolStarted {
            call_id: "c2".into(),
            tool_name: "web_fetch".to_string(),
        });

        let mut started_names = Vec::new();
        while let Ok(p) = rx.try_recv() {
            if let AgentProgress::ToolCallStarted { tool_name, .. } = p {
                started_names.push(tool_name);
            }
        }
        assert_eq!(
            started_names,
            vec!["web_fetch".to_string()],
            "sentinel Started must be skipped; the real tool name must still forward"
        );
    }
}

/// A [`GraphEventSink`] that mirrors the `tinyagents` graph executor's lifecycle
/// stream onto openhuman's `tracing` diagnostics — an observability journal for
/// graph runs (issue #4249 / #28). Node/step/run/route transitions land as
/// grep-friendly `[graph]` lines tagged with `label`; the running event count is
/// exposed for tests. Shared by every openhuman graph (council fan-out,
/// sub-agent delegation, …).
pub struct GraphTracingSink {
    label: String,
    count: Arc<std::sync::atomic::AtomicUsize>,
}

impl GraphTracingSink {
    /// Build a sink tagging its lines with `label` (e.g. `"delegation:graph"`).
    /// Accepts both string literals and runtime-built labels.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Shared counter of events observed, for assertions.
    pub fn counter(&self) -> Arc<std::sync::atomic::AtomicUsize> {
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
