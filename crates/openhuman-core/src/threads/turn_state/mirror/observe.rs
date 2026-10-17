//! Translating one [`AgentProgress`] event into a [`TurnState`] mutation.
//!
//! The mirror itself (snapshot, flush, caps, transcript bookkeeping,
//! interrupted-turn finalization) is
//! `tinyagents_session::turn_state::TurnStateMirror`; this is the host-side
//! progress projection over it.

use crate::agent::progress::AgentProgress;
use crate::threads::turn_state::rpc_types::persisted_tool_failure;

use tinyagents_session::turn_state::mirror::caps::{cap_persisted_args, cap_persisted_output};
use tinyagents_session::turn_state::types::{
    SubagentActivity, SubagentToolCall, SubagentTranscriptItem, ToolTimelineEntry,
    ToolTimelineStatus, TurnLifecycle, TurnPhase,
};
use tinyagents_session::turn_state::TurnStateMirror;

/// Host extension of the upstream mirror: fold one [`AgentProgress`] event into
/// the snapshot.
pub trait ObserveProgress {
    /// Apply one progress event to the in-memory snapshot. Returns `true`
    /// if the event triggered a disk flush.
    fn observe(&mut self, event: &AgentProgress) -> bool;
}

impl ObserveProgress for TurnStateMirror {
    fn observe(&mut self, event: &AgentProgress) -> bool {
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
                    let seq = self.next_tool_seq();
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
                        failure: None,
                        output: None,
                        seq: Some(seq),
                    });
                }
                self.flush();
                true
            }
            AgentProgress::ToolCallCompleted {
                call_id,
                success,
                failure,
                output,
                ..
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
                    // Persist the plain-language failure so the explanation
                    // survives a thread switch / cold boot (#4459). Clear it on
                    // a (re-)success so a retried row doesn't keep stale copy.
                    entry.failure = failure.as_ref().map(persisted_tool_failure);
                    // Persist the (capped) result text so the rehydrated
                    // timeline can show what the tool returned, matching the
                    // live `tool_result` socket payload.
                    entry.output = cap_persisted_output(output);
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
                parent_call_id,
                ..
            } => {
                self.state.phase = Some(TurnPhase::Subagent);
                self.state.active_subagent = Some(agent_id.clone());
                // Derive the real invoking tool's name from the parent row
                // (`spawn_parallel_agents`, `spawn_async_subagent`,
                // `continue_subagent`, a synthesized `delegate_*`, …) instead
                // of hardcoding `spawn_subagent`, which was wrong for every
                // other delegation path. Falls back to the historical
                // default when there's no `parent_call_id` (e.g.
                // `orchestration::ops`) or no matching row (e.g. it already
                // scrolled out of the timeline).
                let source_tool_name = parent_call_id
                    .as_deref()
                    .and_then(|id| {
                        self.state
                            .tool_timeline
                            .iter()
                            .rev()
                            .find(|entry| entry.id == id)
                    })
                    .map(|entry| entry.name.clone())
                    .unwrap_or_else(|| "spawn_subagent".to_string());
                let seq = self.next_tool_seq();
                self.state.tool_timeline.push(ToolTimelineEntry {
                    id: format!("subagent:{task_id}"),
                    name: format!("subagent:{agent_id}"),
                    round: self.state.iteration,
                    status: ToolTimelineStatus::Running,
                    args_buffer: None,
                    display_name: display_name.clone().or_else(|| Some(agent_id.clone())),
                    detail: None,
                    source_tool_name: Some(source_tool_name),
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
                        parent_call_id: parent_call_id.clone(),
                        output: None,
                        tool_calls: Vec::new(),
                        transcript: Vec::new(),
                    }),
                    failure: None,
                    output: None,
                    seq: Some(seq),
                });
                self.flush();
                true
            }
            AgentProgress::SubagentCompleted {
                task_id,
                elapsed_ms,
                iterations,
                output_chars,
                output,
                ..
            } => {
                if let Some(entry) = self.find_subagent_entry_mut(task_id) {
                    entry.status = ToolTimelineStatus::Success;
                    if let Some(activity) = entry.subagent.as_mut() {
                        activity.elapsed_ms = Some(*elapsed_ms);
                        activity.iterations = Some(*iterations);
                        activity.output_chars = Some(*output_chars);
                        activity.output = cap_persisted_output(output);
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
                arguments,
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
                            // The live socket event already carries these; the
                            // snapshot has to as well or a reloaded child row
                            // comes back without its input (#5987).
                            args: cap_persisted_args(arguments),
                            failure: None,
                            output: None,
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
                output,
                output_chars,
                arguments,
                elapsed_ms,
                failure,
                ..
            } => {
                if let Some(entry) = self.find_subagent_entry_mut(task_id) {
                    if let Some(activity) = entry.subagent.as_mut() {
                        let status = if *success {
                            ToolTimelineStatus::Success
                        } else {
                            ToolTimelineStatus::Error
                        };
                        let persisted_failure = failure.as_ref().map(persisted_tool_failure);
                        if let Some(call) = activity
                            .tool_calls
                            .iter_mut()
                            .rev()
                            .find(|c| c.call_id == *call_id)
                        {
                            call.status = status;
                            call.elapsed_ms = Some(*elapsed_ms);
                            call.output_chars = Some(*output_chars);
                            // Carry the child failure so a failed sub-agent row
                            // keeps its explanation across a round-trip (#4459).
                            call.failure = persisted_failure;
                            call.output = cap_persisted_output(output);
                            // Backfill the input on the tinyagents path, where
                            // the *started* event carries `Value::Null` and the
                            // captured arguments only reach us here (see
                            // `SubagentToolCallCompleted::arguments`). Without
                            // this the ordinary sub-agent tool — the common
                            // case — still rehydrates with no input (#5987).
                            // Only fills a gap: a start event that already
                            // supplied arguments stays authoritative.
                            if call.args.is_none() {
                                if let Some(arguments) = arguments {
                                    call.args = cap_persisted_args(arguments);
                                }
                            }
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
                    let seq = self.next_tool_seq();
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
                        failure: None,
                        output: None,
                        seq: Some(seq),
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
            AgentProgress::TurnCostUpdated { .. } | AgentProgress::ModelCallCompleted { .. } => {
                // Cost/usage updates don't change the turn-state snapshot
                // shape (lifecycle / phase / active tool / etc.), so
                // we just acknowledge them without flushing. Surfacing
                // cost in the persisted snapshot would force a disk
                // flush per LLM call — not worth it for telemetry.
                false
            }
            AgentProgress::TurnContent { .. } => {
                // Prompt/reply content is consumed by the tracing exporter, not
                // the turn-state snapshot; nothing to mirror, no flush.
                false
            }
            AgentProgress::ToolCallProgress { .. } => {
                // A running tool's status line is live-only (`tool_progress`
                // on the socket); the snapshot already shows the call as
                // running, so a flush per beat would buy nothing.
                false
            }
        }
    }
}
