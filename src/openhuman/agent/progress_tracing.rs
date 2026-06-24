//! Structured tracing export off the agent [`progress`](super::progress)
//! channel (issue #3886).
//!
//! OpenHuman already emits rich real-time [`AgentProgress`] events for the UI,
//! but there was no first-class trace export for offline inspection,
//! regression analysis, or debugging long multi-agent runs. This module turns
//! that same event stream into OpenTelemetry/Langfuse-style **spans** —
//!
//! ```text
//! agent.turn                      (root, trace_id = session id)
//! ├─ agent.iteration #1
//! │  ├─ tool.web_search
//! │  └─ subagent.researcher
//! │     ├─ subagent.iteration #1
//! │     │  └─ tool.read_file
//! │     └─ (closed on SubagentCompleted)
//! └─ agent.iteration #2
//! ```
//!
//! correlated by **session id** (the trace id) with **user attribution**
//! (a span attribute), so a run that fans out across many subagents over
//! minutes-to-hours is inspectable end to end.
//!
//! ## Privacy
//!
//! Spans intentionally carry only *metadata* — span names, counts, timings,
//! and token/cost figures. Prompt text, tool arguments, streamed text/thinking
//! deltas, raw error strings, and filesystem paths are **never** recorded,
//! honoring the project's "never log secrets or full PII" rule. The
//! content-bearing [`AgentProgress`] variants (`TextDelta`, `ThinkingDelta`,
//! `ToolCallArgsDelta`) are dropped on the floor here.
//!
//! ## Wiring
//!
//! [`SpanCollector`] is a pure state machine: feed it the progress events plus
//! a millisecond timestamp and it accumulates finished spans. The consumer
//! side (the web progress bridge) owns the clock and the export — see
//! [`export_spans`]. The collector has no I/O and no async, so the span shape
//! is exhaustively unit-testable.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::config::schema::{AgentTracingBackend, AgentTracingConfig};

/// Trace-level correlation context, stamped onto the root span.
#[derive(Debug, Clone)]
pub struct TraceContext {
    /// Trace id — the agent session id. All spans of a run share it.
    pub session_id: String,
    /// User attribution (e.g. the broadcast client id / "system" for
    /// autonomous runs). `None` when the caller is anonymous.
    pub user_id: Option<String>,
}

impl TraceContext {
    pub fn new(session_id: impl Into<String>, user_id: Option<String>) -> Self {
        Self {
            session_id: session_id.into(),
            user_id,
        }
    }
}

/// Derive the trace id (session id) for a run: prefer the UI session id when
/// present, otherwise fall back to the thread id so headless/autonomous runs
/// (which carry no UI session) still correlate their spans.
pub fn trace_session_id(ui_session_id: Option<u64>, thread_id: &str) -> String {
    ui_session_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| thread_id.to_string())
}

/// What a span represents. Mirrors the [`AgentProgress`] lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    /// The whole turn (root span).
    Turn,
    /// One LLM iteration of the parent turn.
    Iteration,
    /// A tool call.
    Tool,
    /// A spawned subagent.
    Subagent,
    /// One LLM iteration inside a subagent.
    SubagentIteration,
}

/// OTel-style span status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanStatus {
    /// Not yet completed, or completed without an explicit success signal.
    Unset,
    /// Completed successfully.
    Ok,
    /// Completed with an error.
    Error,
}

/// A single finished (or in-flight) span. Field names follow OpenTelemetry
/// conventions so the NDJSON drops cleanly into an OTel/Langfuse importer.
#[derive(Debug, Clone, Serialize)]
pub struct TraceSpan {
    /// Trace id (the session id) — shared by every span in the run.
    pub trace_id: String,
    /// Unique id of this span within the trace.
    pub span_id: String,
    /// Parent span id, or `None` for the root turn span.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    /// Human-readable span name, e.g. `agent.turn`, `tool.web_search`.
    pub name: String,
    /// Structured kind for programmatic filtering.
    pub kind: SpanKind,
    /// Wall-clock start (Unix epoch milliseconds).
    pub start_unix_ms: u64,
    /// Wall-clock end (Unix epoch milliseconds); `None` while in flight.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_unix_ms: Option<u64>,
    /// Completion status.
    pub status: SpanStatus,
    /// Metadata-only attributes (no secrets/PII).
    pub attributes: BTreeMap<String, serde_json::Value>,
}

impl TraceSpan {
    /// Duration in milliseconds, or `None` while the span is still open.
    pub fn duration_ms(&self) -> Option<u64> {
        self.end_unix_ms
            .map(|end| end.saturating_sub(self.start_unix_ms))
    }
}

/// Per-subagent bookkeeping so child iterations / tool calls nest correctly.
#[derive(Debug)]
struct SubagentState {
    /// Index of the subagent span in [`SpanCollector::spans`].
    span_index: usize,
    /// Currently-open child iteration span id, if any.
    current_iteration_span_id: Option<String>,
    /// Open child tool spans keyed by `call_id` → span index.
    open_tools: BTreeMap<String, usize>,
}

/// Pure state machine that folds an [`AgentProgress`] stream into spans.
///
/// Call [`record`](Self::record) for each event (with a millisecond
/// timestamp), then [`finish`](Self::finish) once the stream closes to seal
/// any still-open spans. [`spans`](Self::spans) returns the accumulated tree.
#[derive(Debug)]
pub struct SpanCollector {
    ctx: TraceContext,
    spans: Vec<TraceSpan>,
    next_span_seq: u64,

    turn_span_id: Option<String>,
    turn_span_index: Option<usize>,
    current_iteration_span_id: Option<String>,
    current_iteration_index: Option<usize>,

    /// Open parent-turn tool spans keyed by `call_id` → span index.
    open_tools: BTreeMap<String, usize>,
    /// Live subagents keyed by `task_id`.
    subagents: BTreeMap<String, SubagentState>,
}

impl SpanCollector {
    pub fn new(ctx: TraceContext) -> Self {
        Self {
            ctx,
            spans: Vec::new(),
            next_span_seq: 0,
            turn_span_id: None,
            turn_span_index: None,
            current_iteration_span_id: None,
            current_iteration_index: None,
            open_tools: BTreeMap::new(),
            subagents: BTreeMap::new(),
        }
    }

    /// All spans recorded so far (finished and in-flight).
    pub fn spans(&self) -> &[TraceSpan] {
        &self.spans
    }

    /// Index of the span with `span_id`, if any.
    fn span_index_by_id(&self, span_id: &str) -> Option<usize> {
        self.spans.iter().position(|sp| sp.span_id == span_id)
    }

    /// Consume the collector and return its spans.
    pub fn into_spans(self) -> Vec<TraceSpan> {
        self.spans
    }

    /// OTel-style 16-hex span id derived from a monotonic sequence. Stable
    /// and deterministic within a run, which keeps the tests reproducible.
    fn mint_span_id(&mut self) -> String {
        self.next_span_seq += 1;
        format!("{:016x}", self.next_span_seq)
    }

    fn open_span(
        &mut self,
        kind: SpanKind,
        name: impl Into<String>,
        parent_span_id: Option<String>,
        start_unix_ms: u64,
        attributes: BTreeMap<String, serde_json::Value>,
    ) -> (String, usize) {
        let span_id = self.mint_span_id();
        let index = self.spans.len();
        self.spans.push(TraceSpan {
            trace_id: self.ctx.session_id.clone(),
            span_id: span_id.clone(),
            parent_span_id,
            name: name.into(),
            kind,
            start_unix_ms,
            end_unix_ms: None,
            status: SpanStatus::Unset,
            attributes,
        });
        (span_id, index)
    }

    /// Seal a span: set its end timestamp + status and merge in any extra
    /// attributes. A no-op if `index` is out of range (defensive).
    fn close_span(
        &mut self,
        index: usize,
        end_unix_ms: u64,
        status: SpanStatus,
        extra: BTreeMap<String, serde_json::Value>,
    ) {
        if let Some(span) = self.spans.get_mut(index) {
            // Don't let a late event drag end before start.
            span.end_unix_ms = Some(end_unix_ms.max(span.start_unix_ms));
            span.status = status;
            span.attributes.extend(extra);
        }
    }

    /// Lazily open the root turn span so a stream that begins mid-flight
    /// (or never sends `TurnStarted`) still produces a correlated tree.
    fn ensure_turn_span(&mut self, start_unix_ms: u64) -> String {
        if let Some(id) = &self.turn_span_id {
            return id.clone();
        }
        let mut attrs = BTreeMap::new();
        attrs.insert(
            "session.id".to_string(),
            serde_json::Value::String(self.ctx.session_id.clone()),
        );
        if let Some(user) = &self.ctx.user_id {
            attrs.insert(
                "user.id".to_string(),
                serde_json::Value::String(user.clone()),
            );
        }
        let (id, index) = self.open_span(SpanKind::Turn, "agent.turn", None, start_unix_ms, attrs);
        self.turn_span_id = Some(id.clone());
        self.turn_span_index = Some(index);
        id
    }

    /// The parent any iteration / tool / subagent span should hang off:
    /// the current iteration if one is open, else the turn root.
    fn active_parent_id(&mut self, now_unix_ms: u64) -> String {
        if let Some(id) = &self.current_iteration_span_id {
            return id.clone();
        }
        self.ensure_turn_span(now_unix_ms)
    }

    fn close_current_iteration(&mut self, end_unix_ms: u64) {
        if let Some(index) = self.current_iteration_index.take() {
            self.close_span(index, end_unix_ms, SpanStatus::Ok, BTreeMap::new());
        }
        self.current_iteration_span_id = None;
    }

    /// Fold a single progress event into the span tree, stamped at
    /// `now_unix_ms` (the consumer's wall clock when it observed the event).
    pub fn record(&mut self, event: &AgentProgress, now_unix_ms: u64) {
        match event {
            AgentProgress::TurnStarted => {
                self.ensure_turn_span(now_unix_ms);
            }

            AgentProgress::IterationStarted {
                iteration,
                max_iterations,
            } => {
                self.close_current_iteration(now_unix_ms);
                let parent = self.ensure_turn_span(now_unix_ms);
                let mut attrs = BTreeMap::new();
                attrs.insert("agent.iteration".to_string(), json_u32(*iteration));
                attrs.insert(
                    "agent.max_iterations".to_string(),
                    json_u32(*max_iterations),
                );
                let (id, index) = self.open_span(
                    SpanKind::Iteration,
                    format!("agent.iteration#{iteration}"),
                    Some(parent),
                    now_unix_ms,
                    attrs,
                );
                self.current_iteration_span_id = Some(id);
                self.current_iteration_index = Some(index);
            }

            AgentProgress::ToolCallStarted {
                call_id,
                tool_name,
                iteration,
                ..
            } => {
                let parent = self.active_parent_id(now_unix_ms);
                let mut attrs = BTreeMap::new();
                attrs.insert("tool.name".to_string(), json_str(tool_name));
                attrs.insert("tool.call_id".to_string(), json_str(call_id));
                attrs.insert("agent.iteration".to_string(), json_u32(*iteration));
                let (_, index) = self.open_span(
                    SpanKind::Tool,
                    format!("tool.{tool_name}"),
                    Some(parent),
                    now_unix_ms,
                    attrs,
                );
                self.open_tools.insert(call_id.clone(), index);
            }

            AgentProgress::ToolCallCompleted {
                call_id,
                success,
                output_chars,
                elapsed_ms,
                ..
            } => {
                if let Some(index) = self.open_tools.remove(call_id) {
                    let start = self.spans[index].start_unix_ms;
                    let mut extra = BTreeMap::new();
                    extra.insert(
                        "tool.success".to_string(),
                        serde_json::Value::Bool(*success),
                    );
                    extra.insert("tool.output_chars".to_string(), json_usize(*output_chars));
                    extra.insert("tool.elapsed_ms".to_string(), json_u64(*elapsed_ms));
                    self.close_span(index, start + elapsed_ms, status_of(*success), extra);
                }
            }

            AgentProgress::SubagentSpawned {
                agent_id,
                task_id,
                mode,
                dedicated_thread,
                prompt_chars,
                display_name,
                ..
            } => {
                let parent = self.active_parent_id(now_unix_ms);
                let label = display_name.clone().unwrap_or_else(|| agent_id.clone());
                let mut attrs = BTreeMap::new();
                attrs.insert("subagent.agent_id".to_string(), json_str(agent_id));
                attrs.insert("subagent.task_id".to_string(), json_str(task_id));
                attrs.insert("subagent.mode".to_string(), json_str(mode));
                attrs.insert(
                    "subagent.dedicated_thread".to_string(),
                    serde_json::Value::Bool(*dedicated_thread),
                );
                attrs.insert(
                    "subagent.prompt_chars".to_string(),
                    json_usize(*prompt_chars),
                );
                if let Some(name) = display_name {
                    attrs.insert("subagent.display_name".to_string(), json_str(name));
                }
                let (_, index) = self.open_span(
                    SpanKind::Subagent,
                    format!("subagent.{label}"),
                    Some(parent),
                    now_unix_ms,
                    attrs,
                );
                self.subagents.insert(
                    task_id.clone(),
                    SubagentState {
                        span_index: index,
                        current_iteration_span_id: None,
                        open_tools: BTreeMap::new(),
                    },
                );
            }

            AgentProgress::SubagentIterationStarted {
                task_id,
                iteration,
                max_iterations,
                extended_policy,
                ..
            } => {
                // Resolve parent + prior child iteration up front so we don't
                // hold a borrow across the mutating open_span call.
                let (parent_id, prior_iteration_id) = match self.subagents.get(task_id) {
                    Some(state) => (
                        self.spans[state.span_index].span_id.clone(),
                        state.current_iteration_span_id.clone(),
                    ),
                    None => return,
                };
                if let Some(prior) = prior_iteration_id {
                    if let Some(idx) = self.span_index_by_id(&prior) {
                        self.close_span(idx, now_unix_ms, SpanStatus::Ok, BTreeMap::new());
                    }
                }
                let mut attrs = BTreeMap::new();
                attrs.insert("agent.iteration".to_string(), json_u32(*iteration));
                attrs.insert(
                    "agent.max_iterations".to_string(),
                    json_u32(*max_iterations),
                );
                attrs.insert(
                    "agent.extended_policy".to_string(),
                    serde_json::Value::Bool(*extended_policy),
                );
                let (id, _) = self.open_span(
                    SpanKind::SubagentIteration,
                    format!("subagent.iteration#{iteration}"),
                    Some(parent_id),
                    now_unix_ms,
                    attrs,
                );
                if let Some(state) = self.subagents.get_mut(task_id) {
                    state.current_iteration_span_id = Some(id);
                }
            }

            AgentProgress::SubagentToolCallStarted {
                task_id,
                call_id,
                tool_name,
                iteration,
                ..
            } => {
                let parent_id = match self.subagents.get(task_id) {
                    Some(state) => match &state.current_iteration_span_id {
                        Some(id) => id.clone(),
                        None => self.spans[state.span_index].span_id.clone(),
                    },
                    None => return,
                };
                let mut attrs = BTreeMap::new();
                attrs.insert("tool.name".to_string(), json_str(tool_name));
                attrs.insert("tool.call_id".to_string(), json_str(call_id));
                attrs.insert("agent.iteration".to_string(), json_u32(*iteration));
                let (_, index) = self.open_span(
                    SpanKind::Tool,
                    format!("tool.{tool_name}"),
                    Some(parent_id),
                    now_unix_ms,
                    attrs,
                );
                if let Some(state) = self.subagents.get_mut(task_id) {
                    state.open_tools.insert(call_id.clone(), index);
                }
            }

            AgentProgress::SubagentToolCallCompleted {
                task_id,
                call_id,
                success,
                output_chars,
                elapsed_ms,
                ..
            } => {
                let Some(index) = self
                    .subagents
                    .get_mut(task_id)
                    .and_then(|state| state.open_tools.remove(call_id))
                else {
                    return;
                };
                let start = self.spans[index].start_unix_ms;
                let mut extra = BTreeMap::new();
                extra.insert(
                    "tool.success".to_string(),
                    serde_json::Value::Bool(*success),
                );
                extra.insert("tool.output_chars".to_string(), json_usize(*output_chars));
                extra.insert("tool.elapsed_ms".to_string(), json_u64(*elapsed_ms));
                self.close_span(index, start + elapsed_ms, status_of(*success), extra);
            }

            AgentProgress::SubagentCompleted {
                task_id,
                elapsed_ms,
                iterations,
                output_chars,
                ..
            } => {
                let Some(state) = self.subagents.remove(task_id) else {
                    return;
                };
                if let Some(id) = state.current_iteration_span_id.clone() {
                    if let Some(idx) = self.span_index_by_id(&id) {
                        self.close_span(idx, now_unix_ms, SpanStatus::Ok, BTreeMap::new());
                    }
                }
                let start = self.spans[state.span_index].start_unix_ms;
                let mut extra = BTreeMap::new();
                extra.insert("subagent.iterations".to_string(), json_u32(*iterations));
                extra.insert(
                    "subagent.output_chars".to_string(),
                    json_usize(*output_chars),
                );
                extra.insert("subagent.elapsed_ms".to_string(), json_u64(*elapsed_ms));
                self.close_span(state.span_index, start + elapsed_ms, SpanStatus::Ok, extra);
            }

            AgentProgress::SubagentFailed { task_id, error, .. } => {
                let Some(state) = self.subagents.remove(task_id) else {
                    return;
                };
                if let Some(id) = state.current_iteration_span_id.clone() {
                    if let Some(idx) = self.span_index_by_id(&id) {
                        self.close_span(idx, now_unix_ms, SpanStatus::Error, BTreeMap::new());
                    }
                }
                let mut extra = BTreeMap::new();
                // Record only that an error occurred and its length — never the
                // raw error text (may embed paths / payloads / secrets).
                extra.insert("error".to_string(), serde_json::Value::Bool(true));
                extra.insert("error.length".to_string(), json_usize(error.len()));
                self.close_span(state.span_index, now_unix_ms, SpanStatus::Error, extra);
            }

            AgentProgress::TurnCostUpdated {
                model,
                input_tokens,
                output_tokens,
                cached_input_tokens,
                total_usd,
                ..
            } => {
                // Cumulative cost/usage rides on the root turn span so a trace
                // viewer shows the whole-run total at the top.
                let index = match self.turn_span_index {
                    Some(idx) => idx,
                    None => {
                        self.ensure_turn_span(now_unix_ms);
                        self.turn_span_index.expect("turn span just created")
                    }
                };
                if let Some(span) = self.spans.get_mut(index) {
                    span.attributes
                        .insert("gen_ai.request.model".to_string(), json_str(model));
                    span.attributes.insert(
                        "gen_ai.usage.input_tokens".to_string(),
                        json_u64(*input_tokens),
                    );
                    span.attributes.insert(
                        "gen_ai.usage.output_tokens".to_string(),
                        json_u64(*output_tokens),
                    );
                    span.attributes.insert(
                        "gen_ai.usage.cached_input_tokens".to_string(),
                        json_u64(*cached_input_tokens),
                    );
                    span.attributes
                        .insert("gen_ai.usage.cost_usd".to_string(), json_f64(*total_usd));
                }
            }

            AgentProgress::TurnCompleted { iterations } => {
                self.close_current_iteration(now_unix_ms);
                if let Some(index) = self.turn_span_index {
                    let mut extra = BTreeMap::new();
                    extra.insert("agent.iterations".to_string(), json_u32(*iterations));
                    self.close_span(index, now_unix_ms, SpanStatus::Ok, extra);
                }
            }

            // Content-bearing / streaming events carry prompt text, tool
            // arguments, or model output — never exported (privacy rule).
            AgentProgress::TextDelta { .. }
            | AgentProgress::ThinkingDelta { .. }
            | AgentProgress::ToolCallArgsDelta { .. }
            | AgentProgress::SubagentTextDelta { .. }
            | AgentProgress::SubagentThinkingDelta { .. }
            | AgentProgress::SubagentAwaitingUser { .. }
            | AgentProgress::TaskBoardUpdated { .. } => {}
        }
    }

    /// Seal every span still open after the stream closes. Idempotent.
    pub fn finish(&mut self, now_unix_ms: u64) {
        let open: Vec<usize> = self
            .spans
            .iter()
            .enumerate()
            .filter(|(_, span)| span.end_unix_ms.is_none())
            .map(|(idx, _)| idx)
            .collect();
        for idx in open {
            self.close_span(idx, now_unix_ms, SpanStatus::Unset, BTreeMap::new());
        }
        self.current_iteration_span_id = None;
        self.current_iteration_index = None;
        self.open_tools.clear();
        self.subagents.clear();
    }
}

fn status_of(success: bool) -> SpanStatus {
    if success {
        SpanStatus::Ok
    } else {
        SpanStatus::Error
    }
}

fn json_str(s: &str) -> serde_json::Value {
    serde_json::Value::String(s.to_string())
}

fn json_u32(n: u32) -> serde_json::Value {
    serde_json::Value::Number(n.into())
}

fn json_u64(n: u64) -> serde_json::Value {
    serde_json::Value::Number(n.into())
}

fn json_usize(n: usize) -> serde_json::Value {
    serde_json::Value::Number((n as u64).into())
}

fn json_f64(n: f64) -> serde_json::Value {
    serde_json::Number::from_f64(n)
        .map(serde_json::Value::Number)
        // NaN/inf can't be JSON numbers — degrade to null rather than panic.
        .unwrap_or(serde_json::Value::Null)
}

/// Serialize spans to NDJSON (one span object per line) in the requested
/// backend envelope. Both backends share the [`TraceSpan`] body; Langfuse
/// wraps each line with a `{"type":"span-create", ...}` observation envelope
/// so it can be POSTed to the Langfuse ingestion API, while OTel emits the
/// bare span. Returns an empty string for an empty slice.
pub fn spans_to_ndjson(backend: AgentTracingBackend, spans: &[TraceSpan]) -> String {
    let mut out = String::new();
    for span in spans {
        let line = match backend {
            AgentTracingBackend::Otel => serde_json::to_string(span),
            AgentTracingBackend::Langfuse => serde_json::to_string(&serde_json::json!({
                "type": "span-create",
                "body": span,
            })),
        };
        if let Ok(line) = line {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// Export finished spans per the [`AgentTracingConfig`]: append NDJSON to the
/// configured file, or emit to the application log when no path is set.
/// Best-effort — a failed write is logged and swallowed so tracing never
/// breaks an agent run. A no-op when tracing is disabled or there are no spans.
pub fn export_spans(config: &AgentTracingConfig, spans: &[TraceSpan]) {
    if !config.enabled || spans.is_empty() {
        return;
    }
    let payload = spans_to_ndjson(config.backend, spans);
    match &config.export_path {
        Some(path) => {
            use std::io::Write as _;
            let opened = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path);
            match opened {
                Ok(mut file) => {
                    if let Err(err) = file.write_all(payload.as_bytes()) {
                        log::warn!(
                            "[agent-tracing] failed to append {} spans to {path}: {err}",
                            spans.len()
                        );
                    } else {
                        log::debug!("[agent-tracing] exported {} spans to {path}", spans.len());
                    }
                }
                Err(err) => log::warn!("[agent-tracing] failed to open {path}: {err}"),
            }
        }
        None => {
            // No path configured — surface to the log so the export still works
            // on read-only / sandboxed deployments.
            log::info!(
                "[agent-tracing] {} spans (trace_id={}):\n{}",
                spans.len(),
                spans.first().map(|s| s.trace_id.as_str()).unwrap_or(""),
                payload.trim_end()
            );
        }
    }
}

#[cfg(test)]
mod tests;
