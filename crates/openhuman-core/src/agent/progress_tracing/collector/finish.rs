//! Sealing the span tree: retiring a finished subagent, recording a turn's
//! outcome on the root span, and force-closing whatever is still open when
//! the progress stream ends.

use std::collections::BTreeMap;

use tinyagents_harness::observability::trace_export::serialize::{
    json_str, truncate_chars, MAX_ERROR_MESSAGE_CHARS,
};
use tinyagents_harness::observability::trace_export::{SpanKind, SpanStatus};

use super::state::{SpanCollector, TurnOutcome};

/// Metadata key marking a span this collector closed without its completion
/// event. Its latency is a bound, not a measurement.
pub(crate) const FORCE_CLOSED_ATTR: &str = "force_closed";
/// Span attribute carrying a Langfuse level override: `WARNING` for spans that
/// need attention without being harness failures (a force-closed tool, a
/// cancelled turn, a command that exited non-zero). The OTLP converter exports
/// it as the observation level.
pub(crate) const LEVEL_ATTR: &str =
    tinyagents_harness::observability::trace_export::otlp::OBSERVATION_LEVEL_ATTR;

impl SpanCollector {
    /// Seal every span still open after the stream closes. Idempotent.
    ///
    /// A turn span still open here never saw `TurnCompleted`, so it is sealed
    /// as [`TurnOutcome::Incomplete`]. Callers that know how the turn ended
    /// use [`Self::finish_with_outcome`].
    pub fn finish(&mut self, now_unix_ms: u64) {
        self.finish_with_outcome(now_unix_ms, None);
    }

    /// [`Self::finish`], recording `outcome` on the root turn span when it is
    /// still open (a `TurnCompleted` already sealed it otherwise).
    ///
    /// Spans other than the turn that are still open lost their completion
    /// event. They are force-closed at the last moment they could have been
    /// running (their parent's end, else the last event seen) and marked
    /// `force_closed` with a `WARNING` level, so their duration is not read as
    /// a real latency.
    pub fn finish_with_outcome(&mut self, now_unix_ms: u64, outcome: Option<TurnOutcome>) {
        let end = self.last_activity_unix_ms.max(now_unix_ms);
        if let Some(index) = self.turn_span_index {
            if self.spans[index].end_unix_ms.is_none() {
                // Close the open iteration first so a tool under it can be
                // bounded by the iteration's end.
                self.close_current_iteration(self.last_activity_unix_ms.min(end));
                self.apply_turn_outcome(outcome.unwrap_or(TurnOutcome::Incomplete), end);
            }
        }
        // A span still open is bounded by its parent's end when the parent
        // already closed (a tool under a finished iteration ran no later than
        // the next model call), else by the last event seen.
        let open: Vec<usize> = (0..self.spans.len())
            .filter(|&idx| self.spans[idx].end_unix_ms.is_none())
            .collect();
        let fallback_end = self.last_activity_unix_ms.min(end);
        for idx in open {
            self.force_close(idx, fallback_end);
        }
        self.current_iteration_span_id = None;
        self.current_iteration_index = None;
        self.open_tools.clear();
        self.subagents.clear();
        self.finished_subagents.clear();
    }

    /// Record how the turn ended on the root span and seal it at
    /// `now_unix_ms`. A no-op without a turn span.
    ///
    /// * completed → `Ok`;
    /// * failed / timed out / incomplete → `Error` with an `error.message`
    ///   (Langfuse statusMessage). The raw failure text can quote user data,
    ///   so it is attached only with content capture on; otherwise a fixed,
    ///   content-free message is used;
    /// * cancelled → status left unset, `observation.level = WARNING`;
    /// * stopped (breaker / wind-down / iteration cap) → status left unset,
    ///   `observation.level = WARNING` plus the content-free stop attributes
    ///   ([`Self::insert_stop_attrs`]).
    pub(crate) fn apply_turn_outcome(&mut self, outcome: TurnOutcome, now_unix_ms: u64) {
        let Some(index) = self.turn_span_index else {
            return;
        };
        let capture = self.ctx.capture_content;
        let gated = |text: &str, generic: &str| -> serde_json::Value {
            if capture && !text.trim().is_empty() {
                serde_json::Value::String(truncate_chars(text, MAX_ERROR_MESSAGE_CHARS))
            } else {
                json_str(generic)
            }
        };
        let mut extra = BTreeMap::new();
        let (status, label) = match &outcome {
            TurnOutcome::Completed => (SpanStatus::Ok, "completed"),
            TurnOutcome::Failed { message } => {
                extra.insert("error.message".to_string(), gated(message, "Turn failed"));
                (SpanStatus::Error, "failed")
            }
            TurnOutcome::TimedOut { message } => {
                extra.insert(
                    "error.message".to_string(),
                    gated(message, "Turn timed out"),
                );
                (SpanStatus::Error, "timed_out")
            }
            TurnOutcome::Incomplete => {
                extra.insert(
                    "error.message".to_string(),
                    json_str("Turn ended without completing"),
                );
                (SpanStatus::Error, "incomplete")
            }
            TurnOutcome::Cancelled { reason } => {
                extra.insert(LEVEL_ATTR.to_string(), json_str("WARNING"));
                extra.insert(
                    "observation.status_message".to_string(),
                    gated(reason.as_deref().unwrap_or(""), "Turn cancelled"),
                );
                (SpanStatus::Unset, "cancelled")
            }
            TurnOutcome::Stopped { stop } => {
                Self::insert_stop_attrs(stop, &mut extra);
                log::debug!(
                    "[agent-tracing] turn stopped early trace_id={} {}",
                    self.ctx.session_id,
                    stop.status_message()
                );
                (SpanStatus::Unset, "stopped")
            }
        };
        if status == SpanStatus::Error {
            extra.insert("error".to_string(), serde_json::Value::Bool(true));
        }
        extra.insert("turn.outcome".to_string(), json_str(label));
        log::debug!(
            "[agent-tracing] turn outcome trace_id={} outcome={label}",
            self.ctx.session_id
        );
        self.close_span(index, now_unix_ms, status, extra);
    }

    /// Mark a span (turn or subagent) as stopped early: `WARNING` level, the
    /// content-free `stopped: …` summary as its status message, and the
    /// `turn.stop_*` attributes. The summary is also written as
    /// `error.message`, which is what the OTLP converter exports as the
    /// Langfuse statusMessage; it is safe ungated because it holds only the
    /// stop kind, a failure class and a tool name.
    pub(super) fn insert_stop_attrs(
        stop: &tinyagents_harness::turn_stop::TurnStop,
        extra: &mut BTreeMap<String, serde_json::Value>,
    ) {
        let message = stop.status_message();
        extra.insert(LEVEL_ATTR.to_string(), json_str("WARNING"));
        extra.insert("observation.status_message".to_string(), json_str(&message));
        extra.insert("error.message".to_string(), json_str(&message));
        extra.insert("turn.outcome".to_string(), json_str("stopped"));
        extra.insert("turn.stop_kind".to_string(), json_str(stop.kind.as_str()));
        if let Some(class) = &stop.failure_class {
            extra.insert("turn.stop_class".to_string(), json_str(class));
        }
        if let Some(operation) = &stop.operation {
            extra.insert("turn.stop_operation".to_string(), json_str(operation));
        }
    }

    /// Move a completed/failed subagent to `finished_subagents`, closing its
    /// open iteration with `status`. Its still-open tool spans stay open so a
    /// late completion can close them precisely. Returns the subagent span's
    /// index, or `None` for an unknown (or already finished) task.
    pub(super) fn retire_subagent(
        &mut self,
        task_id: &str,
        now_unix_ms: u64,
        status: SpanStatus,
    ) -> Option<usize> {
        let mut state = self.subagents.remove(task_id)?;
        if let Some(id) = state.current_iteration_span_id.take() {
            if let Some(idx) = self.span_index_by_id(&id) {
                self.close_span(idx, now_unix_ms, status, BTreeMap::new());
            }
        }
        if !state.open_tools.is_empty() {
            log::debug!(
                "[agent-tracing] subagent task_id={task_id} finished with {} tool span(s) still \
                 open; awaiting late completions",
                state.open_tools.len()
            );
        }
        let span_index = state.span_index;
        self.finished_subagents.insert(task_id.to_string(), state);
        Some(span_index)
    }

    /// Close one span that never got its completion event.
    fn force_close(&mut self, idx: usize, fallback_end: u64) {
        let (kind, start, parent_end) = {
            let span = &self.spans[idx];
            let parent_end = span
                .parent_span_id
                .as_deref()
                .and_then(|id| self.span_index_by_id(id))
                .and_then(|parent| self.spans[parent].end_unix_ms);
            (span.kind, span.start_unix_ms, parent_end)
        };
        let end = parent_end
            .filter(|end| *end >= start)
            .unwrap_or(fallback_end);
        let mut extra = BTreeMap::new();
        if matches!(kind, SpanKind::Tool | SpanKind::Subagent) {
            extra.insert(FORCE_CLOSED_ATTR.to_string(), serde_json::Value::Bool(true));
            extra.insert(LEVEL_ATTR.to_string(), json_str("WARNING"));
            log::debug!(
                "[agent-tracing] force-closed span name={} start={start} end={end} \
                 (completion event never arrived)",
                self.spans[idx].name
            );
        }
        self.close_span(idx, end, SpanStatus::Unset, extra);
    }
}
