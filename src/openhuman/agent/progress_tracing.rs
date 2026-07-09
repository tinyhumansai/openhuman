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
    /// Programmatic AgentBox `/run` invocation.
    Agentbox,
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
            RunType::Agentbox => "agentbox",
            RunType::ChannelInbound => "channel_inbound",
        }
    }

    /// Classify from the chat-request `source` tag. Known background sources
    /// map to their kinds; everything else (`ptt`/`dictation`/`type`/absent)
    /// is an interactive chat turn.
    pub fn from_source(source: Option<&str>) -> Self {
        match source {
            Some("autonomous") => RunType::AutonomousTask,
            Some("agentbox") => RunType::Agentbox,
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

impl SpanCollector {
    pub fn new(ctx: TraceContext) -> Self {
        Self {
            ctx,
            spans: Vec::new(),
            next_span_seq: 0,
            id_prefix: uuid::Uuid::new_v4().simple().to_string(),
            turn_span_id: None,
            turn_span_index: None,
            current_iteration_span_id: None,
            current_iteration_index: None,
            open_tools: BTreeMap::new(),
            subagents: BTreeMap::new(),
        }
    }

    /// Opt into attaching content to spans (prompt/reply, generation
    /// request/completion, tool + subagent I/O). Wire this to
    /// `observability.agent_tracing.capture_content`. Equivalent to setting
    /// [`TraceContext::with_capture_content`] before construction — there is a
    /// single storage-level gate (`ctx.capture_content`), so content dropped
    /// here can never reach any exporter.
    pub fn with_content_capture(mut self, capture_content: bool) -> Self {
        self.ctx.capture_content = capture_content;
        self
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
    #[cfg(test)]
    pub fn into_spans(self) -> Vec<TraceSpan> {
        self.spans
    }

    /// OTel-style 16-hex span id derived from a monotonic sequence. Stable
    /// and deterministic within a run, which keeps the tests reproducible.
    fn mint_span_id(&mut self) -> String {
        self.next_span_seq += 1;
        // Nonce prefix keeps the id globally unique across turns (Langfuse
        // dedupes observations by id project-wide).
        format!("{}-{:016x}", self.id_prefix, self.next_span_seq)
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
            input: None,
            output: None,
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
        if let Some(client) = &self.ctx.client_id {
            attrs.insert(
                "client.id".to_string(),
                serde_json::Value::String(client.clone()),
            );
        }
        if let Some(agent) = &self.ctx.agent_id {
            attrs.insert(
                "agent.id".to_string(),
                serde_json::Value::String(agent.clone()),
            );
        }
        if let Some(source) = &self.ctx.channel_source {
            attrs.insert(
                "channel.source".to_string(),
                serde_json::Value::String(source.clone()),
            );
        }
        attrs.insert(
            "run.type".to_string(),
            serde_json::Value::String(self.ctx.run_type.as_str().to_string()),
        );
        // Every trace must end up with a Langfuse sessionId: prefer the
        // explicit grouping key (thread/conversation id), else fall back to
        // the trace id itself so the trace is never left session-less.
        let group = self
            .ctx
            .session_group
            .clone()
            .unwrap_or_else(|| self.ctx.session_id.clone());
        attrs.insert("thread.id".to_string(), serde_json::Value::String(group));
        // Trace/root-span name carries agent attribution when known.
        let name = match &self.ctx.agent_id {
            Some(agent) => format!("agent.turn:{agent}"),
            None => "agent.turn".to_string(),
        };
        log::debug!(
            "[agent-tracing] opening turn span trace_id={} name={} user_attributed={} client_attributed={} source={:?}",
            self.ctx.session_id,
            name,
            self.ctx.user_id.is_some(),
            self.ctx.client_id.is_some(),
            self.ctx.channel_source,
        );
        let (id, index) = self.open_span(SpanKind::Turn, name, None, start_unix_ms, attrs);
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

    /// Record a tool call's arguments as the span's `input`, truncated to
    /// [`MAX_TOOL_CONTENT_CHARS`]. A no-op unless content capture is on
    /// (`observability.agent_tracing.capture_content`) — when off, tool I/O
    /// never even reaches the in-memory span. `Null` arguments are skipped.
    fn capture_tool_arguments(&mut self, index: usize, arguments: &serde_json::Value) {
        if !self.ctx.capture_content || arguments.is_null() {
            return;
        }
        let serialized = arguments.to_string();
        let chars = serialized.chars().count();
        if let Some(span) = self.spans.get_mut(index) {
            span.input = Some(serde_json::Value::String(truncate_capture_text(
                &serialized,
            )));
            log::trace!(
                "[agent-tracing] captured tool input span={} chars={chars} truncated={}",
                span.name,
                chars > MAX_TOOL_CONTENT_CHARS,
            );
        }
    }

    /// Record a tool call's result as the span's `output`, truncated to
    /// [`MAX_TOOL_CONTENT_CHARS`]. Same capture gate as
    /// [`Self::capture_tool_arguments`]. Empty output is skipped.
    fn capture_tool_output(&mut self, index: usize, output: &str) {
        if !self.ctx.capture_content || output.is_empty() {
            return;
        }
        let chars = output.chars().count();
        if let Some(span) = self.spans.get_mut(index) {
            span.output = Some(serde_json::Value::String(truncate_capture_text(output)));
            log::trace!(
                "[agent-tracing] captured tool output span={} chars={chars} truncated={}",
                span.name,
                chars > MAX_TOOL_CONTENT_CHARS,
            );
        }
    }

    /// Fold a per-call `ModelCallCompleted` into the tree:
    ///
    /// 1. emit a closed [`SpanKind::Generation`] span (name `llm.<model>`)
    ///    parented under the current iteration — or, for a child call
    ///    (`subagent_task_id` set), under the owning subagent's current
    ///    iteration — carrying exact per-call model/usage/cost plus provenance
    ///    (`gen_ai.provider`) and the pricing basis the local estimator uses;
    /// 2. record the captured request messages (incl. the system prompt) and
    ///    completion as the generation's input/output, gated on
    ///    `capture_content` and truncated to [`MAX_MODEL_CONTENT_CHARS`];
    /// 3. accumulate reasoning / cache-creation tokens onto the root turn
    ///    span, which `TurnCostUpdated` (cumulative rollup) does not carry —
    ///    and, for child calls, roll model + usage onto the subagent span so
    ///    a delegation (e.g. the Context Scout) surfaces its model natively.
    ///
    /// The Langfuse-facing model label is `{provider_id}.{model}` (e.g.
    /// `managed.chat-v1`, `openai.gpt-4o`).
    ///
    /// Generation start is approximated by the enclosing iteration span's
    /// start (the iteration opens on `ModelStarted`); end is the observation
    /// time of the usage record.
    #[allow(clippy::too_many_arguments)]
    fn record_model_call(
        &mut self,
        model: &str,
        provider_id: &str,
        subagent_task_id: Option<&str>,
        input: Option<&serde_json::Value>,
        output: Option<&serde_json::Value>,
        iteration: u32,
        input_tokens: u64,
        output_tokens: u64,
        cached_input_tokens: u64,
        cache_creation_tokens: u64,
        reasoning_tokens: u64,
        cost_usd: f64,
        now_unix_ms: u64,
    ) {
        // Resolve the parent + start basis: a child call nests under its
        // subagent's current iteration (else the subagent span itself); a
        // parent call nests under the turn's current iteration (else root).
        let subagent_state = subagent_task_id.and_then(|id| self.subagents.get(id));
        let (parent, start_basis_index) = match subagent_state {
            Some(state) => match &state.current_iteration_span_id {
                Some(id) => {
                    let idx = self.span_index_by_id(id);
                    (id.clone(), idx)
                }
                None => (
                    self.spans[state.span_index].span_id.clone(),
                    Some(state.span_index),
                ),
            },
            None => {
                let parent = self.active_parent_id(now_unix_ms);
                (parent, self.current_iteration_index)
            }
        };
        let start_unix_ms = start_basis_index
            .and_then(|idx| self.spans.get(idx))
            .map(|span| span.start_unix_ms)
            .unwrap_or(now_unix_ms);

        let labeled_model = format!("{provider_id}.{model}");
        let pricing = crate::openhuman::agent::cost::lookup_pricing(model);

        let mut attrs = BTreeMap::new();
        attrs.insert("gen_ai.request.model".to_string(), json_str(&labeled_model));
        attrs.insert("gen_ai.provider".to_string(), json_str(provider_id));
        attrs.insert("agent.iteration".to_string(), json_u32(iteration));
        attrs.insert(
            "gen_ai.usage.input_tokens".to_string(),
            json_u64(input_tokens),
        );
        attrs.insert(
            "gen_ai.usage.output_tokens".to_string(),
            json_u64(output_tokens),
        );
        // Cache reads always flow (even 0) so usageDetails stay complete.
        attrs.insert(
            "gen_ai.usage.cached_input_tokens".to_string(),
            json_u64(cached_input_tokens),
        );
        if cache_creation_tokens > 0 {
            attrs.insert(
                "gen_ai.usage.cache_creation_tokens".to_string(),
                json_u64(cache_creation_tokens),
            );
        }
        if reasoning_tokens > 0 {
            attrs.insert(
                "gen_ai.usage.reasoning_tokens".to_string(),
                json_u64(reasoning_tokens),
            );
        }
        attrs.insert("gen_ai.usage.cost_usd".to_string(), json_f64(cost_usd));
        // Pricing basis so Langfuse cost figures are auditable against the
        // client-side estimator (USD per million tokens).
        attrs.insert(
            "gen_ai.pricing.input_per_mtok_usd".to_string(),
            json_f64(pricing.input_per_mtok_usd),
        );
        attrs.insert(
            "gen_ai.pricing.cached_input_per_mtok_usd".to_string(),
            json_f64(pricing.cached_input_per_mtok_usd),
        );
        attrs.insert(
            "gen_ai.pricing.output_per_mtok_usd".to_string(),
            json_f64(pricing.output_per_mtok_usd),
        );

        log::debug!(
            "[agent-tracing] generation span model={labeled_model} \
             iteration={iteration} child={} in={input_tokens} out={output_tokens} \
             cost_usd={cost_usd:.6} input_captured={} output_captured={}",
            subagent_task_id.is_some(),
            input.is_some(),
            output.is_some(),
        );
        let (_, index) = self.open_span(
            SpanKind::Generation,
            format!("llm.{model}"),
            Some(parent),
            start_unix_ms,
            attrs,
        );
        // Captured request messages (incl. system prompt) + completion become
        // the generation's input/output — only while content capture is on,
        // truncated so one huge context window can't bloat the trace batch.
        if self.ctx.capture_content {
            if let Some(span) = self.spans.get_mut(index) {
                if let Some(value) = input {
                    span.input = Some(capture_model_content(value));
                }
                if let Some(value) = output {
                    span.output = Some(capture_model_content(value));
                }
            }
        }
        self.close_span(index, now_unix_ms, SpanStatus::Ok, BTreeMap::new());

        // Child call: roll model + usage + cost onto the owning subagent span
        // so the delegation row (e.g. `subagent.Context Scout`) natively shows
        // which model served it and what it cost.
        if let Some(state_index) = subagent_task_id
            .and_then(|id| self.subagents.get(id))
            .map(|state| state.span_index)
        {
            if let Some(span) = self.spans.get_mut(state_index) {
                span.attributes
                    .insert("gen_ai.request.model".to_string(), json_str(&labeled_model));
                span.attributes
                    .insert("gen_ai.provider".to_string(), json_str(provider_id));
                for (key, add) in [
                    ("gen_ai.usage.input_tokens", input_tokens),
                    ("gen_ai.usage.output_tokens", output_tokens),
                    ("gen_ai.usage.cached_input_tokens", cached_input_tokens),
                ] {
                    let prior = span
                        .attributes
                        .get(key)
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    span.attributes
                        .insert(key.to_string(), json_u64(prior.saturating_add(add)));
                }
                let prior_cost = span
                    .attributes
                    .get("gen_ai.usage.cost_usd")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0);
                span.attributes.insert(
                    "gen_ai.usage.cost_usd".to_string(),
                    json_f64(prior_cost + cost_usd),
                );
            }
            return;
        }

        // Root rollup for the usage dimensions the cumulative TurnCostUpdated
        // event does not carry (reasoning / cache-creation), plus provenance
        // and the provider-labeled model (TurnCostUpdated only knows the raw
        // model handle and can fire before the first per-call event).
        let root = match self.turn_span_index {
            Some(idx) => idx,
            None => {
                self.ensure_turn_span(now_unix_ms);
                self.turn_span_index.expect("turn span just created")
            }
        };
        if let Some(span) = self.spans.get_mut(root) {
            span.attributes
                .insert("gen_ai.provider".to_string(), json_str(provider_id));
            span.attributes
                .insert("gen_ai.request.model".to_string(), json_str(&labeled_model));
            for (key, add) in [
                ("gen_ai.usage.reasoning_tokens", reasoning_tokens),
                ("gen_ai.usage.cache_creation_tokens", cache_creation_tokens),
            ] {
                if add == 0 && span.attributes.get(key).is_none() {
                    continue;
                }
                let prior = span
                    .attributes
                    .get(key)
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                span.attributes
                    .insert(key.to_string(), json_u64(prior.saturating_add(add)));
            }
        }
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
                arguments,
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
                self.capture_tool_arguments(index, arguments);
                self.open_tools.insert(call_id.clone(), index);
            }

            AgentProgress::ToolCallCompleted {
                call_id,
                success,
                output_chars,
                output,
                arguments,
                elapsed_ms,
                failure,
                ..
            } => {
                if let Some(index) = self.open_tools.remove(call_id) {
                    // The tinyagents path emits `Null` arguments on the started
                    // event and the real captured arguments on completion —
                    // backfill the span input when it's still empty.
                    if self.spans[index].input.is_none() {
                        if let Some(arguments) = arguments {
                            self.capture_tool_arguments(index, arguments);
                        }
                    }
                    self.capture_tool_output(index, output);
                    let start = self.spans[index].start_unix_ms;
                    let mut extra = BTreeMap::new();
                    extra.insert(
                        "tool.success".to_string(),
                        serde_json::Value::Bool(*success),
                    );
                    extra.insert("tool.output_chars".to_string(), json_usize(*output_chars));
                    extra.insert("tool.elapsed_ms".to_string(), json_u64(*elapsed_ms));
                    // Failed tool calls surface a Langfuse statusMessage: the
                    // classified plain-language cause, truncated, gated on
                    // content capture (it can quote user data / paths).
                    if let Some(failure) = failure {
                        if self.ctx.capture_content {
                            extra.insert(
                                "error.message".to_string(),
                                serde_json::Value::String(truncate_chars(
                                    &failure.cause_plain,
                                    MAX_ERROR_MESSAGE_CHARS,
                                )),
                            );
                        }
                    }
                    self.close_span(index, start + elapsed_ms, status_of(*success), extra);
                }
            }

            AgentProgress::ModelCallCompleted {
                model,
                provider_id,
                subagent_task_id,
                input,
                output,
                iteration,
                input_tokens,
                output_tokens,
                cached_input_tokens,
                cache_creation_tokens,
                reasoning_tokens,
                cost_usd,
            } => {
                self.record_model_call(
                    model,
                    provider_id,
                    subagent_task_id.as_deref(),
                    input.as_ref(),
                    output.as_ref(),
                    *iteration,
                    *input_tokens,
                    *output_tokens,
                    *cached_input_tokens,
                    *cache_creation_tokens,
                    *reasoning_tokens,
                    *cost_usd,
                    now_unix_ms,
                );
            }

            AgentProgress::SubagentSpawned {
                agent_id,
                task_id,
                mode,
                dedicated_thread,
                prompt_chars,
                prompt,
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
                // The delegated prompt is the subagent span's input (gated +
                // truncated like model content — scout prompts run 10k+ chars).
                if self.ctx.capture_content && !prompt.is_empty() {
                    if let Some(span) = self.spans.get_mut(index) {
                        span.input = Some(serde_json::Value::String(truncate_chars(
                            prompt,
                            MAX_MODEL_CONTENT_CHARS,
                        )));
                    }
                }
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
                arguments,
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
                self.capture_tool_arguments(index, arguments);
                if let Some(state) = self.subagents.get_mut(task_id) {
                    state.open_tools.insert(call_id.clone(), index);
                }
            }

            AgentProgress::SubagentToolCallCompleted {
                task_id,
                call_id,
                success,
                output_chars,
                output,
                arguments,
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
                if self.spans[index].input.is_none() {
                    if let Some(arguments) = arguments {
                        self.capture_tool_arguments(index, arguments);
                    }
                }
                self.capture_tool_output(index, output);
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
                output,
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
                // The subagent's final assistant text is the span's output
                // (same gate + cap as its prompt input).
                if self.ctx.capture_content && !output.is_empty() {
                    if let Some(span) = self.spans.get_mut(state.span_index) {
                        span.output = Some(serde_json::Value::String(truncate_chars(
                            output,
                            MAX_MODEL_CONTENT_CHARS,
                        )));
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
                // Always record that an error occurred and its length. The raw
                // error text (may embed paths / payloads) is recorded — truncated
                // — only when content capture is on, and surfaces in Langfuse as
                // the observation statusMessage.
                extra.insert("error".to_string(), serde_json::Value::Bool(true));
                extra.insert("error.length".to_string(), json_usize(error.len()));
                if self.ctx.capture_content {
                    extra.insert(
                        "error.message".to_string(),
                        serde_json::Value::String(truncate_chars(error, MAX_ERROR_MESSAGE_CHARS)),
                    );
                }
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

            AgentProgress::TurnContent { input, output } => {
                // Storage-level privacy gate (#4454): prompt/reply text is
                // attached to the span ONLY when content capture is opted in.
                // With the gate off (default), the content is dropped here so no
                // exporter — NDJSON file, app log, or Langfuse push — can ever
                // serialize it. This is the single choke point; the exporters
                // deliberately do not re-check the flag.
                if !self.ctx.capture_content {
                    log::debug!(
                        target: "agent-tracing",
                        "[agent-tracing] TurnContent dropped at storage (capture_content=false)"
                    );
                    return;
                }
                let index = match self.turn_span_index {
                    Some(idx) => idx,
                    None => {
                        self.ensure_turn_span(now_unix_ms);
                        self.turn_span_index.expect("turn span just created")
                    }
                };
                if let Some(span) = self.spans.get_mut(index) {
                    if let Some(text) = input {
                        span.input = Some(serde_json::Value::String(text.clone()));
                    }
                    if let Some(text) = output {
                        span.output = Some(serde_json::Value::String(text.clone()));
                    }
                    log::debug!(
                        target: "agent-tracing",
                        "[agent-tracing] TurnContent attached to turn span (capture_content=true)"
                    );
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
    observations: &[tinyagents::harness::observability::AgentObservation],
    run_telemetry: Option<&crate::openhuman::session_db::run_ledger::RunTelemetry>,
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
mod tests;
