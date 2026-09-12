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
//! `managed.chat-v1`). While `observability.agent_tracing.capture_content` is
//! on, content is additionally recorded as span `input`/`output` — the turn's
//! prompt/reply, each generation's **truncated** request messages (system
//! prompt included) + completion, **truncated** tool arguments/results, and
//! each subagent's delegated prompt + final output. With the flag off (the
//! default — #4454), none of that content ever reaches the in-memory span, so
//! no exporter (NDJSON file, app log, or Langfuse) can leak it.
//! Streamed text/thinking deltas (`TextDelta`, `ThinkingDelta`,
//! `ToolCallArgsDelta`), raw error strings, and filesystem paths are **never**
//! recorded regardless of the flag, honoring the project's "never log secrets
//! or full PII" rule for logs.
//!
//! The one exception is the turn's prompt/reply, delivered via
//! `AgentProgress::TurnContent`. It is attached to the turn span **only** when
//! the operator opts in via `observability.agent_tracing.capture_content`
//! (default `false`). That gate is enforced at storage time in
//! [`SpanCollector`] — the single choke point — so with the default off, no
//! exporter (NDJSON file, app log, or Langfuse push) can ever serialize it.
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
use crate::openhuman::config::Config;

/// Journal-backed projection from durable tinyagents observations.
pub(crate) mod journal_projection;
/// Langfuse ingestion exporter (remote push to the co-hosted staging server).
pub(crate) mod langfuse;

#[cfg(test)]
mod journal_projection_tests;

/// Kind of run a trace belongs to, rendered as stable snake_case strings for
/// Langfuse trace tags (`run:<type>`) and metadata (`run_type`) so runs can be
/// filtered in the UI.
///
/// Only kinds actually observable at the collector installation point (the
/// web progress bridge) exist here: orchestration passes, subconscious runs,
/// cron turns, and meeting agents run their turns WITHOUT a progress bridge
/// today, so they never reach the span collector and get no variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunType {
    /// Interactive user chat turn (desktop UI / socket / PTT / dictation).
    #[default]
    InteractiveChat,
    /// Autonomous background run from the task dispatcher.
    AutonomousTask,
    /// Inbound message relayed from an external channel (Telegram, Discord,
    /// Slack, …) through the channel bus.
    ChannelInbound,
}

impl RunType {
    /// Stable snake_case identifier used in tags/metadata.
    pub fn as_str(self) -> &'static str {
        match self {
            RunType::InteractiveChat => "interactive_chat",
            RunType::AutonomousTask => "autonomous_task",
            RunType::ChannelInbound => "channel_inbound",
        }
    }

    /// Classify from the chat-request `source` tag. Known background sources
    /// map to their kinds; everything else (`ptt`/`dictation`/`type`/absent)
    /// is an interactive chat turn.
    pub fn from_source(source: Option<&str>) -> Self {
        match source {
            Some("autonomous") => RunType::AutonomousTask,
            Some("channel_inbound") => RunType::ChannelInbound,
            _ => RunType::InteractiveChat,
        }
    }
}

/// Trace-level correlation context, stamped onto the root span.
#[derive(Debug, Clone)]
pub struct TraceContext {
    /// Trace id — unique per turn. Every span of a single turn shares it, so
    /// each turn becomes its own Langfuse trace.
    pub session_id: String,
    /// Real authenticated user attribution (the backend user id, or email as
    /// fallback) — exported as the Langfuse `userId`. `None` when the caller
    /// is anonymous. Transport identifiers (socket client id / "system")
    /// belong in [`Self::client_id`], not here.
    pub user_id: Option<String>,
    /// Transport client id (the broadcast socket client, or `"system"` for
    /// autonomous runs). Exported as the `client.id` metadata attribute so it
    /// stays inspectable without polluting user attribution.
    pub client_id: Option<String>,
    /// Agent definition id driving the turn (e.g. `"orchestrator"`,
    /// `"researcher"`). Stamped as the `agent.id` attribute and folded into
    /// the root span/trace name (`agent.turn:<agent_id>`).
    pub agent_id: Option<String>,
    /// Where the run originated (`"chat"`, `"ptt"`, `"autonomous"`, …).
    /// Exported as the `channel.source` metadata attribute.
    pub channel_source: Option<String>,
    /// Grouping key (the thread/conversation id) exported as the Langfuse
    /// `sessionId` so per-turn traces still group under one session. When
    /// `None`, the collector falls back to the trace id so every trace still
    /// carries a session id.
    pub session_group: Option<String>,
    /// Whether content capture (`observability.agent_tracing.capture_content`)
    /// is on. Gates recording tool arguments/results onto spans at collection
    /// time — when off, tool I/O never even reaches the in-memory span.
    pub capture_content: bool,
    /// Kind of run — exported as Langfuse trace tags (`run:<type>`) and the
    /// `run_type` metadata key. Defaults to interactive chat.
    pub run_type: RunType,
    /// This run's own id (the tinyagents `RunContext` run id), exported as the
    /// `run_id` metadata key. `None` until the run's observations are known.
    pub run_id: Option<String>,
    /// The spawning run's id when this run is a sub-agent/graph node, exported
    /// as the `parent_run_id` metadata key. `None` for top-level turns. This is
    /// what links a spawned sub-agent's trace back to its parent turn (#4657).
    pub parent_run_id: Option<String>,
    /// The root ancestor run id (equal to [`Self::run_id`] for top-level runs),
    /// exported as the `root_run_id` metadata key so Langfuse can thread a whole
    /// spawn tree under one root.
    pub root_run_id: Option<String>,
}

impl TraceContext {
    pub fn new(session_id: impl Into<String>, user_id: Option<String>) -> Self {
        Self {
            session_id: session_id.into(),
            user_id,
            client_id: None,
            agent_id: None,
            channel_source: None,
            session_group: None,
            capture_content: false,
            run_type: RunType::default(),
            run_id: None,
            parent_run_id: None,
            root_run_id: None,
        }
    }

    /// Set the grouping key (thread/conversation id) for the Langfuse
    /// `sessionId`, so a conversation's per-turn traces group together.
    pub fn with_session_group(mut self, group: impl Into<String>) -> Self {
        self.session_group = Some(group.into());
        self
    }

    /// Set the transport client id (`client.id` metadata attribute).
    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = Some(client_id.into());
        self
    }

    /// Set the agent definition id (`agent.id` attribute + trace name suffix).
    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    /// Set the run origin (`channel.source` metadata attribute).
    pub fn with_channel_source(mut self, source: impl Into<String>) -> Self {
        self.channel_source = Some(source.into());
        self
    }

    /// Enable/disable content capture (tool arguments/results on spans).
    pub fn with_capture_content(mut self, capture_content: bool) -> Self {
        self.capture_content = capture_content;
        self
    }

    /// Set the run type (Langfuse `run:<type>` tag / `run_type` metadata).
    pub fn with_run_type(mut self, run_type: RunType) -> Self {
        self.run_type = run_type;
        self
    }

    /// Stamp the run lineage (`run_id` / `parent_run_id` / `root_run_id`) so a
    /// spawned sub-agent's trace links back to its parent turn (#4657). The ids
    /// come from the tinyagents `RunContext`, surfaced via the run's journalled
    /// observations at export time.
    pub fn with_run_lineage(
        mut self,
        run_id: Option<String>,
        parent_run_id: Option<String>,
        root_run_id: Option<String>,
    ) -> Self {
        self.run_id = run_id;
        self.parent_run_id = parent_run_id;
        self.root_run_id = root_run_id;
        self
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
    /// A single LLM call (model invocation) with per-call usage/cost.
    Generation,
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
/// conventions (snake_case `trace_id`/`span_id`/`start_unix_ms`/…) so the raw
/// NDJSON file/log export is a self-describing OTel-style span dump for local
/// inspection.
///
/// #4469 item 13: this raw record is **not** directly Langfuse-ingestible — the
/// Langfuse `/api/public/ingestion` API needs each span wrapped in a
/// `{ type, id, timestamp, body }` event envelope. That envelope is produced
/// only by [`langfuse::spans_to_langfuse_batch`] on the remote-push path; the
/// local NDJSON exporter intentionally emits the raw spans, not the batch
/// format.
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
    /// Optional prompt/input content. Populated (via `AgentProgress::TurnContent`)
    /// **only** when `observability.agent_tracing.capture_content` is opted in —
    /// the [`SpanCollector`] drops content at storage time otherwise, so with the
    /// default gate off this is always `None` and no exporter can serialize it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    /// Optional model-reply/output content. Same storage-level gating as
    /// [`Self::input`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
}

impl TraceSpan {
    /// Duration in milliseconds, or `None` while the span is still open.
    #[cfg(test)]
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
    /// Per-collector (per-turn) random prefix for minted span ids. Langfuse
    /// dedupes observations by id **globally**, so a bare per-turn sequence
    /// (`0000…0001`) collides across turns and silently binds later turns'
    /// observations to whichever trace first claimed the id. Prefixing with a
    /// fresh nonce makes every span id globally unique.
    id_prefix: String,

    turn_span_id: Option<String>,
    turn_span_index: Option<usize>,
    current_iteration_span_id: Option<String>,
    current_iteration_index: Option<usize>,

    /// Open parent-turn tool spans keyed by `call_id` → span index.
    open_tools: BTreeMap<String, usize>,
    /// Live subagents keyed by `task_id`.
    subagents: BTreeMap<String, SubagentState>,
}
include!("progress_tracing_impl_01_part_01.rs");
include!("progress_tracing_impl_01_part_02.rs");

/// Cap on tool arguments / tool output recorded onto spans when content
/// capture is on. Keeps a single runaway tool result from bloating the trace
/// batch while still giving Langfuse an actionable preview.
const MAX_TOOL_CONTENT_CHARS: usize = 4_000;

/// Cap on captured error text (Langfuse observation `statusMessage`).
const MAX_ERROR_MESSAGE_CHARS: usize = 500;

/// Cap on captured model request/completion content and subagent
/// prompt/output. Larger than the tool cap because a generation's input is the
/// full message array (system prompt included) — but still bounded so a
/// 100k-token context can't push the ingestion batch past Langfuse's event
/// size limits.
const MAX_MODEL_CONTENT_CHARS: usize = 25_000;

/// Capture a model-payload JSON value for a span: kept structured when it
/// serializes within [`MAX_MODEL_CONTENT_CHARS`], else degraded to a truncated
/// string (readable in Langfuse, bounded in size).
fn capture_model_content(value: &serde_json::Value) -> serde_json::Value {
    let serialized = value.to_string();
    if serialized.chars().count() <= MAX_MODEL_CONTENT_CHARS {
        value.clone()
    } else {
        serde_json::Value::String(truncate_chars(&serialized, MAX_MODEL_CONTENT_CHARS))
    }
}

/// Truncate `text` to `max` characters, appending an explicit truncation
/// marker (with the omitted char count) when content was dropped. Returns the
/// input unchanged when it already fits. Slices on char boundaries, so it
/// never panics on multi-byte content.
fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_string(),
        Some((byte_end, _)) => {
            let omitted = text.chars().count() - max;
            format!("{}…[truncated {omitted} chars]", &text[..byte_end])
        }
    }
}

/// Truncate tool-content text to [`MAX_TOOL_CONTENT_CHARS`].
fn truncate_capture_text(text: &str) -> String {
    truncate_chars(text, MAX_TOOL_CONTENT_CHARS)
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
pub(crate) fn spans_to_ndjson(backend: AgentTracingBackend, spans: &[TraceSpan]) -> String {
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
pub(crate) fn export_spans(config: &AgentTracingConfig, spans: &[TraceSpan]) {
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
            // No path configured. Surface only metadata (count + trace id) at
            // `info` so the export is visible on read-only / sandboxed
            // deployments WITHOUT ever printing span content at `info` (#4454).
            // The NDJSON body — which may carry prompt/reply text when
            // `capture_content` is opted in — goes to `debug` only. With the
            // default gate off, the storage layer already strips content, so
            // `payload` is metadata-only regardless.
            log::info!(
                "[agent-tracing] {} spans (trace_id={}) — set observability.agent_tracing.export_path to persist",
                spans.len(),
                spans.first().map(|s| s.trace_id.as_str()).unwrap_or(""),
            );
            log::debug!(
                target: "agent-tracing",
                "[agent-tracing] span NDJSON ({} spans):\n{}",
                spans.len(),
                payload.trim_end()
            );
        }
    }
}

/// Hand a completed run's spans to the configured tracing sink(s).
///
/// Two independent paths, both best-effort and never fatal to a turn:
///
/// 1. **Usage-data sharing** (`observability.share_usage_data`, on by default):
///    push the run's spans to the backend Langfuse proxy — endpoint derived from
///    the current backend host, authed with the session bearer (see
///    [`langfuse::push_spans`]). A failure (no live session, network, rejected
///    batch) just logs; there is no local fallback, since sharing and local
///    export are distinct opt-ins. Web-channel turns that successfully read a
///    durable tinyagents journal should call [`export_run_trace_from_journal`]
///    instead, so the remote push uses the crate-owned observation exporter.
/// 2. **Local exporter** (`observability.agent_tracing.enabled`, opt-in): append
///    OTel/Langfuse-format NDJSON to the configured file or the app log via
///    [`export_spans`].
///
/// A no-op when there are no spans or both paths are off.
pub(crate) async fn export_run_trace(config: &Config, spans: &[TraceSpan]) {
    if spans.is_empty() {
        return;
    }
    let observability = &config.observability;

    if observability.share_usage_data {
        if let Err(err) = langfuse::push_spans(config, spans).await {
            log::warn!("[agent-tracing] Langfuse usage-data push failed ({err})");
        }
    }

    if observability.agent_tracing.enabled {
        export_spans(&observability.agent_tracing, spans);
    }
}

/// Export a completed run when durable tinyagents observations are available.
/// Remote usage-data sharing uses the crate Langfuse exporter over the journal;
/// local tracing still writes the live spans until the migration deletes the
/// legacy span collector/exporter path.
pub(crate) async fn export_run_trace_from_journal(
    config: &Config,
    trace_ctx: &TraceContext,
    observations: &[tinyagents_harness::observability::AgentObservation],
    run_telemetry: Option<&tinyagents_session::run_ledger::RunTelemetry>,
    live_spans: &[TraceSpan],
) {
    if observations.is_empty() && live_spans.is_empty() {
        return;
    }
    let observability = &config.observability;

    if observability.share_usage_data && !observations.is_empty() {
        if let Err(err) =
            langfuse::push_observations(config, trace_ctx, observations, run_telemetry).await
        {
            log::warn!("[agent-tracing] Langfuse journal usage-data push failed ({err})");
        }
    } else if observability.share_usage_data {
        log::debug!("[agent-tracing] no journal observations for Langfuse usage-data push");
    }

    if observability.agent_tracing.enabled && !live_spans.is_empty() {
        export_spans(&observability.agent_tracing, live_spans);
    }
}

#[cfg(test)]
#[path = "progress_tracing/progress_tracing_tests.rs"]
mod tests;
