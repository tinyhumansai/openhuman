//! Wire/storage types for per-thread agent-turn snapshots.
//!
//! A [`TurnState`] mirrors the live state held by the web-channel
//! progress consumer so the UI can rehydrate after a cold boot or
//! after the user navigates away mid-turn. The shape intentionally
//! parallels `app/src/store/chatRuntimeSlice.ts` so a snapshot can
//! be applied directly to that slice.

use serde::{Deserialize, Serialize};

/// Lifecycle of an in-flight (or formerly in-flight) turn.
///
/// `Started` is set when the user sends and the agent loop is about
/// to enter the iteration loop. `Streaming` is set after the first
/// progress signal arrives. `Interrupted` is stamped at startup on
/// any snapshot that survived a process restart — there is no live
/// driver to resume it, so the UI should surface a retry affordance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnLifecycle {
    Started,
    Streaming,
    Interrupted,
}

/// High-level phase the agent is in within an iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnPhase {
    Thinking,
    ToolUse,
    Subagent,
}

/// Per-tool entry shown in the live timeline UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTimelineStatus {
    Running,
    Success,
    Error,
}

/// One row in the per-turn tool timeline.
///
/// Field names use camelCase on the wire so a snapshot can be applied
/// directly to `chatRuntimeSlice.toolTimelineByThread` without a
/// translation layer in the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolTimelineEntry {
    pub id: String,
    pub name: String,
    pub round: u32,
    pub status: ToolTimelineStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_buffer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent: Option<SubagentActivity>,
}

/// Live sub-agent activity nested under a `subagent:*` timeline row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentActivity {
    pub task_id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedicated_thread: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_iteration: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_max_iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_chars: Option<usize>,
    #[serde(default)]
    pub tool_calls: Vec<SubagentToolCall>,
}

/// One child tool call performed by a running sub-agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentToolCall {
    pub call_id: String,
    pub tool_name: String,
    pub status: ToolTimelineStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_chars: Option<usize>,
}

/// Persisted snapshot of an in-flight agent turn for one thread.
///
/// Written to disk by the web-channel progress consumer at iteration
/// boundaries, tool start/complete, and on terminal events. Deleted
/// on successful turn completion. A surviving snapshot at startup
/// indicates an interrupted turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnState {
    pub thread_id: String,
    pub request_id: String,
    pub lifecycle: TurnLifecycle,
    pub iteration: u32,
    pub max_iterations: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<TurnPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_subagent: Option<String>,
    #[serde(default)]
    pub streaming_text: String,
    #[serde(default)]
    pub thinking: String,
    #[serde(default)]
    pub tool_timeline: Vec<ToolTimelineEntry>,
    pub started_at: String,
    pub updated_at: String,
}

/// Request payload for `openhuman.threads_turn_state_get`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetTurnStateRequest {
    pub thread_id: String,
}

/// Response payload for `openhuman.threads_turn_state_get`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTurnStateResponse {
    /// `None` when no snapshot exists for the thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_state: Option<TurnState>,
}

/// Response payload for `openhuman.threads_turn_state_list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTurnStatesResponse {
    pub turn_states: Vec<TurnState>,
    pub count: usize,
}

/// Request payload for `openhuman.threads_turn_state_clear`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearTurnStateRequest {
    pub thread_id: String,
}

/// Response payload for `openhuman.threads_turn_state_clear`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearTurnStateResponse {
    pub cleared: bool,
}

impl TurnState {
    /// Build a fresh `Started` snapshot for a new turn.
    pub fn started(
        thread_id: impl Into<String>,
        request_id: impl Into<String>,
        max_iterations: u32,
        now_rfc3339: impl Into<String>,
    ) -> Self {
        let now = now_rfc3339.into();
        Self {
            thread_id: thread_id.into(),
            request_id: request_id.into(),
            lifecycle: TurnLifecycle::Started,
            iteration: 0,
            max_iterations,
            phase: None,
            active_tool: None,
            active_subagent: None,
            streaming_text: String::new(),
            thinking: String::new(),
            tool_timeline: Vec::new(),
            started_at: now.clone(),
            updated_at: now,
        }
    }
}
