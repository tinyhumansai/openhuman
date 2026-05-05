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
    SubagentActivity, SubagentToolCall, ToolTimelineEntry, ToolTimelineStatus, TurnLifecycle,
    TurnPhase, TurnState,
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
                ..
            } => {
                self.state.lifecycle = TurnLifecycle::Streaming;
                self.state.phase = Some(TurnPhase::ToolUse);
                self.state.active_tool = Some(tool_name.clone());
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
                } else {
                    self.state.tool_timeline.push(ToolTimelineEntry {
                        id: call_id.clone(),
                        name: tool_name.clone(),
                        round: *iteration,
                        status: ToolTimelineStatus::Running,
                        args_buffer: None,
                        display_name: None,
                        detail: None,
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
                    display_name: Some(agent_id.clone()),
                    detail: None,
                    source_tool_name: Some("spawn_subagent".to_string()),
                    subagent: Some(SubagentActivity {
                        task_id: task_id.clone(),
                        agent_id: agent_id.clone(),
                        mode: Some(mode.clone()),
                        dedicated_thread: Some(*dedicated_thread),
                        child_iteration: None,
                        child_max_iterations: None,
                        iterations: None,
                        elapsed_ms: None,
                        output_chars: None,
                        tool_calls: Vec::new(),
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
                        });
                    }
                }
                false
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
                        if let Some(call) = activity
                            .tool_calls
                            .iter_mut()
                            .rev()
                            .find(|c| c.call_id == *call_id)
                        {
                            call.status = if *success {
                                ToolTimelineStatus::Success
                            } else {
                                ToolTimelineStatus::Error
                            };
                            call.elapsed_ms = Some(*elapsed_ms);
                            call.output_chars = Some(*output_chars);
                        }
                    }
                }
                false
            }
            AgentProgress::TextDelta { delta, .. } => {
                self.state.streaming_text.push_str(delta);
                false
            }
            AgentProgress::ThinkingDelta { delta, .. } => {
                self.state.thinking.push_str(delta);
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
                if let Err(err) = self.store.delete(&self.state.thread_id) {
                    log::warn!(
                        "{MIRROR_LOG_PREFIX} failed to delete snapshot for thread={}: {err}",
                        self.state.thread_id
                    );
                }
                true
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

    fn find_subagent_entry_mut(&mut self, task_id: &str) -> Option<&mut ToolTimelineEntry> {
        let needle = format!("subagent:{task_id}");
        self.state
            .tool_timeline
            .iter_mut()
            .rev()
            .find(|entry| entry.id == needle)
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> &TurnState {
        &self.state
    }
}

#[cfg(test)]
#[path = "mirror_tests.rs"]
mod tests;
