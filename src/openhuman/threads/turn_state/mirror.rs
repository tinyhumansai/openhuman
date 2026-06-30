//! Translate [`AgentProgress`] events into [`TurnState`] mutations and
//! flush snapshots to disk at iteration / tool boundaries.
//!
//! Used by the web-channel progress bridge to keep an authoritative,
//! restart-survivable record of the in-flight turn alongside the live
//! socket emissions. High-frequency deltas (text, thinking, tool args)
//! mutate the in-memory snapshot but do not trigger a disk flush —
//! anything more granular than an iteration / tool boundary would
//! thrash the filesystem under streaming load.
//!
//! On terminal completion the snapshot file is deleted. If the bridge
//! exits without ever observing [`AgentProgress::TurnCompleted`] (for
//! example because the agent loop returned an error), the snapshot is
//! flagged [`TurnLifecycle::Interrupted`] and persisted so the UI can
//! surface a retry affordance.

use crate::openhuman::agent::progress::AgentProgress;

use super::store::TurnStateStore;
use super::types::{
    SubagentActivity, SubagentToolCall, SubagentTranscriptItem, ToolTimelineEntry,
    ToolTimelineStatus, TranscriptItem, TurnLifecycle, TurnPhase, TurnState,
};

const MIRROR_LOG_PREFIX: &str = "[threads:turn_state:mirror]";

/// In-process cursor that keeps the authoritative [`TurnState`] in sync
/// with the agent loop and writes it through to a [`TurnStateStore`].
pub struct TurnStateMirror {
    store: TurnStateStore,
    state: TurnState,
    /// Set to `true` once we observe `TurnCompleted` so `finish` knows
    /// to delete the snapshot rather than mark it interrupted.
    turn_completed: bool,
    /// Monotonic ordering key for [`TranscriptItem`]s. Round alone can't
    /// order narration vs thinking vs tool calls *within* one iteration, so
    /// every transcript push stamps and increments this.
    next_seq: u32,
}

impl TurnStateMirror {
    /// Build a mirror primed with a `Started` snapshot and immediately
    /// flush so a crash before the first agent event still leaves a
    /// recoverable record.
    pub fn new(
        store: TurnStateStore,
        thread_id: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        let state = TurnState::started(thread_id, request_id, 0, now);
        let mut mirror = Self {
            store,
            state,
            turn_completed: false,
            next_seq: 0,
        };
        mirror.flush();
        mirror
    }

    /// Apply one progress event to the in-memory snapshot. Returns `true`
    /// if the event triggered a disk flush.
    pub fn observe(&mut self, event: &AgentProgress) -> bool {
        self.state.updated_at = chrono::Utc::now().to_rfc3339();
        match event {
            AgentProgress::TurnStarted => {
                self.state.lifecycle = TurnLifecycle::Started;
                self.flush();
                true
            }
            AgentProgress::IterationStarted {
                iteration,
                max_iterations,
            } => {
                self.state.iteration = *iteration;
                self.state.max_iterations = *max_iterations;
                self.state.phase = Some(TurnPhase::Thinking);
                self.state.lifecycle = TurnLifecycle::Streaming;
                self.state.active_tool = None;
                self.flush();
                true
            }
            AgentProgress::ToolCallStarted {
                call_id,
                tool_name,
                iteration,
                display_label,
                display_detail,
                ..
            } => {
                self.state.lifecycle = TurnLifecycle::Streaming;
                self.state.phase = Some(TurnPhase::ToolUse);
                self.state.active_tool = Some(tool_name.clone());
                // Record the tool row in the ordered transcript so the
                // processing panel can interleave it between narration /
                // thinking at the position it actually occurred.
                self.push_transcript_tool(*iteration, call_id);
                // `ToolCallArgsDelta` may have already created a
                // synthetic placeholder for this `call_id` before the
                // start event arrived. Reuse it (filling in `name` /
                // `round`) so the timeline doesn't end up with two
                // rows for one tool call.
                if let Some(existing) = self
                    .state
                    .tool_timeline
                    .iter_mut()
                    .rev()
                    .find(|e| e.id == *call_id)
                {
                    existing.name = tool_name.clone();
                    existing.round = *iteration;
                    existing.status = ToolTimelineStatus::Running;
                    // Only overwrite with a present server value so an
                    // args-delta placeholder's fields aren't clobbered to None.
                    if display_label.is_some() {
                        existing.display_name = display_label.clone();
                    }
                    if display_detail.is_some() {
                        existing.detail = display_detail.clone();
                    }
                } else {
                    self.state.tool_timeline.push(ToolTimelineEntry {
                        id: call_id.clone(),
                        name: tool_name.clone(),
                        round: *iteration,
                        status: ToolTimelineStatus::Running,
                        args_buffer: None,
                        display_name: display_label.clone(),
                        detail: display_detail.clone(),
                        source_tool_name: None,
                        subagent: None,
                    });
                }
                self.flush();
                true
            }
            AgentProgress::ToolCallCompleted {
                call_id, success, ..
            } => {
                if let Some(entry) = self
                    .state
                    .tool_timeline
                    .iter_mut()
                    .rev()
                    .find(|e| e.id == *call_id)
                {
                    entry.status = if *success {
                        ToolTimelineStatus::Success
                    } else {
                        ToolTimelineStatus::Error
                    };
                }
                if self.state.active_tool.is_some() {
                    self.state.active_tool = None;
                }
                self.state.phase = Some(TurnPhase::Thinking);
                self.flush();
                true
            }
            AgentProgress::SubagentSpawned {
                agent_id,
                task_id,
                mode,
                dedicated_thread,
                worker_thread_id,
                display_name,
                ..
            } => {
                self.state.phase = Some(TurnPhase::Subagent);
                self.state.active_subagent = Some(agent_id.clone());
                self.state.tool_timeline.push(ToolTimelineEntry {
                    id: format!("subagent:{task_id}"),
                    name: format!("subagent:{agent_id}"),
                    round: self.state.iteration,
                    status: ToolTimelineStatus::Running,
                    args_buffer: None,
                    display_name: display_name.clone().or_else(|| Some(agent_id.clone())),
                    detail: None,
                    source_tool_name: Some("spawn_subagent".to_string()),
                    subagent: Some(SubagentActivity {
                        task_id: task_id.clone(),
                        agent_id: agent_id.clone(),
                        status: None,
                        mode: Some(mode.clone()),
                        dedicated_thread: Some(*dedicated_thread),
                        child_iteration: None,
                        child_max_iterations: None,
                        iterations: None,
                        elapsed_ms: None,
                        output_chars: None,
                        worker_thread_id: worker_thread_id.clone(),
                        tool_calls: Vec::new(),
                        transcript: Vec::new(),
                    }),
                });
                self.flush();
                true
            }
            AgentProgress::SubagentCompleted {
                task_id,
                elapsed_ms,
                iterations,
                output_chars,
                ..
            } => {
                if let Some(entry) = self.find_subagent_entry_mut(task_id) {
                    entry.status = ToolTimelineStatus::Success;
                    if let Some(activity) = entry.subagent.as_mut() {
                        activity.elapsed_ms = Some(*elapsed_ms);
                        activity.iterations = Some(*iterations);
                        activity.output_chars = Some(*output_chars);
                    }
                }
                self.state.active_subagent = None;
                self.state.phase = Some(TurnPhase::Thinking);
                self.flush();
                true
            }
            AgentProgress::SubagentFailed { task_id, .. } => {
                if let Some(entry) = self.find_subagent_entry_mut(task_id) {
                    entry.status = ToolTimelineStatus::Error;
                }
                self.state.active_subagent = None;
                self.state.phase = Some(TurnPhase::Thinking);
                self.flush();
                true
            }
            AgentProgress::SubagentAwaitingUser { task_id, .. } => {
                if let Some(entry) = self.find_subagent_entry_mut(task_id) {
                    if let Some(activity) = entry.subagent.as_mut() {
                        activity.status = Some("awaiting_user".to_string());
                    }
                }
                self.flush();
                true
            }
            AgentProgress::SubagentIterationStarted {
                task_id,
                iteration,
                max_iterations,
                ..
            } => {
                if let Some(entry) = self.find_subagent_entry_mut(task_id) {
                    if let Some(activity) = entry.subagent.as_mut() {
                        activity.child_iteration = Some(*iteration);
                        activity.child_max_iterations = Some(*max_iterations);
                    }
                }
                false
            }
            AgentProgress::SubagentToolCallStarted {
                task_id,
                call_id,
                tool_name,
                iteration,
                display_label,
                display_detail,
                ..
            } => {
                if let Some(entry) = self.find_subagent_entry_mut(task_id) {
                    if let Some(activity) = entry.subagent.as_mut() {
                        activity.tool_calls.push(SubagentToolCall {
                            call_id: call_id.clone(),
                            tool_name: tool_name.clone(),
                            status: ToolTimelineStatus::Running,
                            iteration: Some(*iteration),
                            elapsed_ms: None,
                            output_chars: None,
                            display_name: display_label.clone(),
                            detail: display_detail.clone(),
                        });
                        // Mirror the call into the ordered transcript so the
                        // rehydrated thoughts interleave it at the right spot.
                        activity.transcript.push(SubagentTranscriptItem::Tool {
                            iteration: Some(*iteration),
                            call_id: call_id.clone(),
                            tool_name: tool_name.clone(),
                            status: ToolTimelineStatus::Running,
                            elapsed_ms: None,
                            output_chars: None,
                            display_name: display_label.clone(),
                            detail: display_detail.clone(),
                        });
                    }
                }
                // Flush at sub-agent tool boundaries so prose streamed since the
                // last boundary reaches disk (the parent is blocked on the
                // spawn tool, so its own flushes don't fire mid sub-agent run).
                self.flush();
                true
            }
            AgentProgress::SubagentToolCallCompleted {
                task_id,
                call_id,
                success,
                output_chars,
                elapsed_ms,
                ..
            } => {
                if let Some(entry) = self.find_subagent_entry_mut(task_id) {
                    if let Some(activity) = entry.subagent.as_mut() {
                        let status = if *success {
                            ToolTimelineStatus::Success
                        } else {
                            ToolTimelineStatus::Error
                        };
                        if let Some(call) = activity
                            .tool_calls
                            .iter_mut()
                            .rev()
                            .find(|c| c.call_id == *call_id)
                        {
                            call.status = status;
                            call.elapsed_ms = Some(*elapsed_ms);
                            call.output_chars = Some(*output_chars);
                        }
                        // Keep the transcript's Tool item in lockstep so the
                        // rehydrated row shows the terminal status + timing.
                        if let Some(SubagentTranscriptItem::Tool {
                            status: tx_status,
                            elapsed_ms: tx_elapsed,
                            output_chars: tx_output,
                            ..
                        }) = activity
                            .transcript
                            .iter_mut()
                            .rev()
                            .find(|item| matches!(item, SubagentTranscriptItem::Tool { call_id: c, .. } if c == call_id))
                        {
                            *tx_status = status;
                            *tx_elapsed = Some(*elapsed_ms);
                            *tx_output = Some(*output_chars);
                        }
                    }
                }
                self.flush();
                true
            }
            AgentProgress::SubagentTextDelta {
                task_id,
                delta,
                iteration,
                ..
            } => {
                self.push_subagent_prose(task_id, *iteration, delta, false);
                false
            }
            AgentProgress::SubagentThinkingDelta {
                task_id,
                delta,
                iteration,
                ..
            } => {
                self.push_subagent_prose(task_id, *iteration, delta, true);
                false
            }
            AgentProgress::TaskBoardUpdated { board } => {
                self.state.task_board = Some(board.clone());
                self.flush();
                true
            }
            AgentProgress::TextDelta { delta, iteration } => {
                self.state.streaming_text.push_str(delta);
                self.push_transcript_narration(*iteration, delta);
                false
            }
            AgentProgress::ThinkingDelta { delta, iteration } => {
                self.state.thinking.push_str(delta);
                self.push_transcript_thinking(*iteration, delta);
                false
            }
            AgentProgress::ToolCallArgsDelta {
                call_id,
                tool_name,
                delta,
                ..
            } => {
                if let Some(entry) = self
                    .state
                    .tool_timeline
                    .iter_mut()
                    .rev()
                    .find(|e| e.id == *call_id)
                {
                    let buffer = entry.args_buffer.get_or_insert_with(String::new);
                    buffer.push_str(delta);
                } else {
                    // No matching entry yet — `ToolCallArgsDelta` may
                    // arrive before `ToolCallStarted` so synthesise a
                    // placeholder we can update once the start event lands.
                    self.state.tool_timeline.push(ToolTimelineEntry {
                        id: call_id.clone(),
                        name: tool_name.clone(),
                        round: self.state.iteration,
                        status: ToolTimelineStatus::Running,
                        args_buffer: Some(delta.clone()),
                        display_name: None,
                        detail: None,
                        source_tool_name: None,
                        subagent: None,
                    });
                }
                false
            }
            AgentProgress::TurnCompleted { .. } => {
                self.turn_completed = true;
                // Keep the snapshot (don't delete) so a reloaded / cold-booted
                // client can replay this turn's processing transcript via
                // `getTurnState`. Mark it `Completed` and quiesce the live
                // fields so the UI renders it settled (no spinner / retry),
                // and startup interrupted-marking leaves it alone.
                self.state.lifecycle = TurnLifecycle::Completed;
                self.state.phase = None;
                self.state.active_tool = None;
                self.state.active_subagent = None;
                self.flush();
                true
            }
            AgentProgress::TurnCostUpdated { .. } => {
                // Cost updates don't change the turn-state snapshot
                // shape (lifecycle / phase / active tool / etc.), so
                // we just acknowledge them without flushing. Surfacing
                // cost in the persisted snapshot would force a disk
                // flush per LLM call — not worth it for telemetry.
                false
            }
        }
    }

    /// Mark the turn as `Interrupted` on the in-memory snapshot and
    /// flush. Called when the bridge exits without a `TurnCompleted`
    /// event (i.e. the agent loop errored out).
    pub fn finish(mut self) {
        if self.turn_completed {
            return;
        }
        self.state.lifecycle = TurnLifecycle::Interrupted;
        self.state.active_tool = None;
        self.state.active_subagent = None;
        self.state.updated_at = chrono::Utc::now().to_rfc3339();
        self.flush();
    }

    fn flush(&mut self) {
        if let Err(err) = self.store.put(&self.state) {
            log::warn!(
                "{MIRROR_LOG_PREFIX} failed to persist snapshot for thread={}: {err}",
                self.state.thread_id
            );
        }
    }

    /// Append a visible-narration delta to the transcript, coalescing into
    /// the trailing [`TranscriptItem::Narration`] when it's the most recent
    /// item and from the same round — so a streamed paragraph stays one item
    /// instead of one-per-token. A new round (or any intervening thinking /
    /// tool item) starts a fresh narration block.
    fn push_transcript_narration(&mut self, round: u32, delta: &str) {
        if let Some(TranscriptItem::Narration { round: r, text, .. }) =
            self.state.transcript.last_mut()
        {
            if *r == round {
                text.push_str(delta);
                return;
            }
        }
        let seq = self.next_seq();
        self.state.transcript.push(TranscriptItem::Narration {
            round,
            seq,
            text: delta.to_string(),
        });
    }

    /// Append a hidden-reasoning delta to the transcript, with the same
    /// coalescing rule as [`Self::push_transcript_narration`].
    fn push_transcript_thinking(&mut self, round: u32, delta: &str) {
        if let Some(TranscriptItem::Thinking { round: r, text, .. }) =
            self.state.transcript.last_mut()
        {
            if *r == round {
                text.push_str(delta);
                return;
            }
        }
        let seq = self.next_seq();
        self.state.transcript.push(TranscriptItem::Thinking {
            round,
            seq,
            text: delta.to_string(),
        });
    }

    /// Record a tool call in the transcript at the point it occurred, as a
    /// pointer into [`TurnState::tool_timeline`] (the row's status/label live
    /// there). Skips a duplicate if the same `call_id` was already recorded
    /// (e.g. a start event after an args-delta placeholder).
    fn push_transcript_tool(&mut self, round: u32, call_id: &str) {
        let already = self.state.transcript.iter().any(
            |item| matches!(item, TranscriptItem::ToolCall { call_id: c, .. } if c == call_id),
        );
        if already {
            return;
        }
        let seq = self.next_seq();
        self.state.transcript.push(TranscriptItem::ToolCall {
            round,
            seq,
            call_id: call_id.to_string(),
        });
    }

    /// Return the next monotonic transcript ordering key and advance it.
    fn next_seq(&mut self) -> u32 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        seq
    }

    fn find_subagent_entry_mut(&mut self, task_id: &str) -> Option<&mut ToolTimelineEntry> {
        let needle = format!("subagent:{task_id}");
        self.state
            .tool_timeline
            .iter_mut()
            .rev()
            .find(|entry| entry.id == needle)
    }

    /// Append a sub-agent prose delta (narration when `is_thinking == false`,
    /// reasoning otherwise) to that sub-agent's transcript, coalescing into the
    /// trailing same-kind, same-iteration item so a streamed paragraph stays
    /// one entry (mirrors the frontend `appendSubagentStreamDelta`). Mutate-
    /// only (no flush) — high-frequency like the parent's `TextDelta`; the
    /// accumulated prose is persisted at the next sub-agent tool boundary.
    fn push_subagent_prose(
        &mut self,
        task_id: &str,
        iteration: u32,
        delta: &str,
        is_thinking: bool,
    ) {
        let Some(entry) = self.find_subagent_entry_mut(task_id) else {
            return;
        };
        let Some(activity) = entry.subagent.as_mut() else {
            return;
        };
        match activity.transcript.last_mut() {
            Some(SubagentTranscriptItem::Thinking {
                iteration: it,
                text,
            }) if is_thinking && *it == Some(iteration) => {
                text.push_str(delta);
                return;
            }
            Some(SubagentTranscriptItem::Text {
                iteration: it,
                text,
            }) if !is_thinking && *it == Some(iteration) => {
                text.push_str(delta);
                return;
            }
            _ => {}
        }
        activity.transcript.push(if is_thinking {
            SubagentTranscriptItem::Thinking {
                iteration: Some(iteration),
                text: delta.to_string(),
            }
        } else {
            SubagentTranscriptItem::Text {
                iteration: Some(iteration),
                text: delta.to_string(),
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> &TurnState {
        &self.state
    }
}

#[cfg(test)]
#[path = "mirror_tests.rs"]
mod tests;
