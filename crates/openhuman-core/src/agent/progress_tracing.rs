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
//! Spans always carry *metadata* — span names, counts, timings, and
//! token/cost figures (model labels are `{provider_id}.{model}`, e.g.
//! `managed.hint:chat`). While `observability.agent_tracing.capture_content` is
//! on (the default), content is additionally recorded as span `input`/`output` — the turn's
//! prompt/reply, each generation's bounded structured request messages (system
//! prompt included) + completion, **truncated** tool arguments/results, and
//! each subagent's delegated prompt + final output. With the flag off,
//! none of that content ever reaches the in-memory span, so
//! no exporter (NDJSON file, app log, or Langfuse) can leak it.
//! Streamed text/thinking deltas (`TextDelta`, `ThinkingDelta`,
//! `ToolCallArgsDelta`), raw error strings, and filesystem paths are **never**
//! recorded regardless of the flag, honoring the project's "never log secrets
//! or full PII" rule for logs.
//!
//! The one exception is the turn's prompt/reply, delivered via
//! `AgentProgress::TurnContent`. It is attached to the turn span **only** when
//! `observability.agent_tracing.capture_content` is enabled. That gate is
//! enforced at storage time in [`SpanCollector`] — the single choke point — so when off, no
//! exporter (NDJSON file, app log, or Langfuse push) can ever serialize it.
//!
//! ## Wiring
//!
//! [`SpanCollector`] is a pure state machine: feed it the progress events plus
//! a millisecond timestamp and it accumulates finished spans. The consumer
//! side (the web progress bridge) owns the clock and the export — see
//! [`export_spans`]. The collector has no I/O and no async, so the span shape
//! is exhaustively unit-testable.

/// Journal-backed projection from durable tinyagents observations.
pub(crate) mod journal_projection;
/// Langfuse ingestion exporter (remote push through the configured backend).
pub(crate) mod langfuse;
mod otlp;

/// The [`SpanCollector`] state machine that folds progress events into spans.
mod collector;
/// Handing finished spans to the local exporter and the Langfuse push.
mod export;
/// Content truncation caps, JSON-value builders, and NDJSON serialization.
mod serialize;
/// Data model: [`RunType`], [`TraceContext`], [`SpanKind`], [`SpanStatus`],
/// [`TraceSpan`].
mod types;

pub use collector::SpanCollector;
#[cfg(test)]
pub use types::SpanKind;
pub use types::{trace_session_id, RunType, SpanStatus, TraceContext, TraceSpan};

pub(crate) use export::export_subagent_journal_trace;
pub(crate) use export::{export_run_trace, export_run_trace_from_journal};

#[cfg(test)]
mod journal_projection_tests;

#[cfg(test)]
#[path = "progress_tracing/progress_tracing_tests.rs"]
mod tests;
