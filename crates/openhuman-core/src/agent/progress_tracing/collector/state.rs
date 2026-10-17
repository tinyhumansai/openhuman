//! [`SpanCollector`] data shape, construction, and small accessors.

use std::collections::BTreeMap;

use tinyagents_harness::observability::trace_export::{SpanStatus, TraceContext, TraceSpan};

/// When the in-flight model call streamed its first delta, for
/// time-to-first-token. Reset when a new iteration (model call) starts and
/// consumed when that call's usage is recorded.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct FirstDeltas {
    /// First delta of any kind: reasoning, text or tool-call arguments.
    pub(super) any_unix_ms: Option<u64>,
    /// First visible text delta.
    pub(super) text_unix_ms: Option<u64>,
    /// Most recent delta of any kind. A call whose every delta lands in the
    /// last instant before completion was not streamed (see
    /// [`super::model_call`]'s unstreamed-call guard).
    pub(super) last_unix_ms: Option<u64>,
}

impl FirstDeltas {
    /// Stamp `now_unix_ms` as the first delta (and first text, when `is_text`)
    /// unless an earlier one is already recorded.
    pub(super) fn observe(&mut self, now_unix_ms: u64, is_text: bool) {
        self.any_unix_ms.get_or_insert(now_unix_ms);
        self.last_unix_ms = Some(now_unix_ms);
        if is_text {
            self.text_unix_ms.get_or_insert(now_unix_ms);
        }
    }
}

/// Per-subagent bookkeeping so child iterations / tool calls nest correctly.
#[derive(Debug)]
pub(super) struct SubagentState {
    /// Index of the subagent span in [`SpanCollector::spans`].
    pub(super) span_index: usize,
    /// Currently-open child iteration span id, if any.
    pub(super) current_iteration_span_id: Option<String>,
    /// Open child tool spans keyed by `call_id` → span index.
    pub(super) open_tools: BTreeMap<String, usize>,
    /// First streamed delta of the child's in-flight model call.
    pub(super) first_deltas: FirstDeltas,
    /// Langfuse-facing label (`{provider}.{model}`) of the child's latest
    /// model call, stamped on the child tool spans it requests.
    pub(super) last_model: Option<String>,
    /// Raw model of the child's most recent call, the fallback name for a call
    /// that arrives without one.
    pub(super) last_raw_model: Option<String>,
    /// Last child iteration span opened, kept after it closes so a model call
    /// or tool event reported after the subagent finished still nests under it.
    pub(super) last_iteration_span_id: Option<String>,
    /// Start-time bookkeeping for the child's model calls (see [`CallClock`]).
    pub(super) call_clock: CallClock,
}

impl SubagentState {
    pub(super) fn new(span_index: usize) -> Self {
        Self {
            span_index,
            current_iteration_span_id: None,
            open_tools: BTreeMap::new(),
            first_deltas: FirstDeltas::default(),
            last_model: None,
            last_raw_model: None,
            last_iteration_span_id: None,
            call_clock: CallClock::default(),
        }
    }
}

/// Per-scope bookkeeping for a model call's start time.
///
/// The progress stream carries no per-call request start: a generation used
/// to start at its enclosing iteration's start, so several calls folded into
/// one iteration (retries, or a child's calls whose iteration events were
/// lost) all shared that instant and each one's latency covered every call
/// before it. A call cannot have started before the previous call in the same
/// iteration ended, so that end is the tighter bound; an explicit start
/// (the journal's `ModelCompleted.started_at_ms`) beats both.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct CallClock {
    /// End of the previous generation in the current iteration.
    pub(super) last_call_end_unix_ms: Option<u64>,
    /// Explicit request start for the next call, set by a source that knows
    /// it ([`SpanCollector::set_next_call_start`]).
    pub(super) explicit_start_unix_ms: Option<u64>,
}

/// How a turn ended, for the root span's status.
///
/// `AgentProgress` has no aborted/failed turn variant, so a host that knows
/// the outcome passes it to [`SpanCollector::finish_with_outcome`]; a turn
/// span still open at [`SpanCollector::finish`] (no `TurnCompleted` arrived)
/// is sealed as [`TurnOutcome::Incomplete`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    /// The turn produced its reply.
    Completed,
    /// The turn failed with an error.
    Failed { message: String },
    /// A deadline or per-call timeout ended the turn.
    TimedOut { message: String },
    /// The user (or the host) cancelled the turn.
    Cancelled { reason: Option<String> },
    /// The stream closed before the turn reported an outcome.
    Incomplete,
    /// The harness stopped the turn early (failure breaker, deadline
    /// wind-down, iteration cap) and it reached the completion path anyway.
    Stopped {
        stop: tinyagents_harness::turn_stop::TurnStop,
    },
}

/// Pure state machine that folds an [`crate::agent::progress::AgentProgress`]
/// stream into spans.
///
/// Call [`record`](Self::record) for each event (with a millisecond
/// timestamp), then [`finish`](Self::finish) once the stream closes to seal
/// any still-open spans. [`spans`](Self::spans) returns the accumulated tree.
#[derive(Debug)]
pub struct SpanCollector {
    pub(super) ctx: TraceContext,
    pub(super) spans: Vec<TraceSpan>,
    pub(super) next_span_seq: u64,
    /// Per-collector (per-turn) random prefix for minted span ids. Langfuse
    /// dedupes observations by id **globally**, so a bare per-turn sequence
    /// (`0000…0001`) collides across turns and silently binds later turns'
    /// observations to whichever trace first claimed the id. Prefixing with a
    /// fresh nonce makes every span id globally unique.
    pub(super) id_prefix: String,

    pub(super) turn_span_id: Option<String>,
    pub(super) turn_span_index: Option<usize>,
    pub(super) current_iteration_span_id: Option<String>,
    pub(super) current_iteration_index: Option<usize>,

    /// Open parent-turn tool spans keyed by `call_id` → span index.
    pub(super) open_tools: BTreeMap<String, usize>,
    /// Live subagents keyed by `task_id`.
    pub(super) subagents: BTreeMap<String, SubagentState>,
    /// First streamed delta of the parent turn's in-flight model call.
    pub(super) first_deltas: FirstDeltas,
    /// Langfuse-facing label (`{provider}.{model}`) of the parent turn's latest
    /// model call. Tool calls run after the model call that requested them
    /// completes, so this names the model behind every tool span opened next.
    pub(super) last_model: Option<String>,
    /// Raw model of the parent turn's most recent call, the fallback name for
    /// a call that arrives without one.
    pub(super) last_raw_model: Option<String>,
    /// Start-time bookkeeping for the parent turn's model calls.
    pub(super) call_clock: CallClock,
    /// Subagents that already reported completion/failure, kept so their late
    /// events (tool completions, model calls) still resolve to their spans
    /// instead of being dropped or misparented under the parent iteration.
    pub(super) finished_subagents: BTreeMap<String, SubagentState>,
    /// Latest event timestamp seen; the end of a span force-closed at
    /// [`SpanCollector::finish`] when nothing tighter is known.
    pub(super) last_activity_unix_ms: u64,
    /// Whether the turn span's input is already set. The first `TurnContent`
    /// input is the originating user message; later ones (the commit path's
    /// last user-role message can be tool results or a harness nudge) must
    /// not overwrite it.
    pub(super) turn_input_recorded: bool,
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
            first_deltas: FirstDeltas::default(),
            last_model: None,
            last_raw_model: None,
            call_clock: CallClock::default(),
            finished_subagents: BTreeMap::new(),
            last_activity_unix_ms: 0,
            turn_input_recorded: false,
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

    /// Record that the next model call in `subagent_task_id`'s scope (`None`
    /// = the parent turn) was dispatched at `start_unix_ms`. Consumed by that
    /// call's generation span. Used by the journal projection, whose
    /// `ModelCompleted` carries the real request start.
    pub(crate) fn set_next_call_start(
        &mut self,
        subagent_task_id: Option<&str>,
        start_unix_ms: u64,
    ) {
        let clock = match subagent_task_id {
            None => Some(&mut self.call_clock),
            Some(id) => self
                .subagents
                .get_mut(id)
                .or_else(|| self.finished_subagents.get_mut(id))
                .map(|state| &mut state.call_clock),
        };
        if let Some(clock) = clock {
            clock.explicit_start_unix_ms = Some(start_unix_ms);
        }
    }

    /// Live or finished subagent state for `task_id`.
    pub(super) fn subagent_state(&self, task_id: &str) -> Option<&SubagentState> {
        self.subagents
            .get(task_id)
            .or_else(|| self.finished_subagents.get(task_id))
    }

    /// Mutable live or finished subagent state for `task_id`.
    pub(super) fn subagent_state_mut(&mut self, task_id: &str) -> Option<&mut SubagentState> {
        match self.subagents.get_mut(task_id) {
            Some(state) => Some(state),
            None => self.finished_subagents.get_mut(task_id),
        }
    }

    /// Index of the span with `span_id`, if any.
    pub(super) fn span_index_by_id(&self, span_id: &str) -> Option<usize> {
        self.spans.iter().position(|sp| sp.span_id == span_id)
    }

    /// Consume the collector and return its spans.
    #[cfg(test)]
    pub fn into_spans(self) -> Vec<TraceSpan> {
        self.spans
    }

    /// OTel-style 16-hex span id derived from a monotonic sequence. Stable
    /// and deterministic within a run, which keeps the tests reproducible.
    pub(super) fn mint_span_id(&mut self) -> String {
        self.next_span_seq += 1;
        // Nonce prefix keeps the id globally unique across turns (Langfuse
        // dedupes observations by id project-wide).
        format!("{}-{:016x}", self.id_prefix, self.next_span_seq)
    }

    pub(super) fn open_span(
        &mut self,
        kind: tinyagents_harness::observability::trace_export::SpanKind,
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
    pub(super) fn close_span(
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
}
