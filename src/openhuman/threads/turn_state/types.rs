//! Wire/storage types for per-thread agent-turn snapshots.
//!
//! A [`TurnState`] mirrors the live state held by the web-channel
//! progress consumer so the UI can rehydrate after a cold boot or
//! after the user navigates away mid-turn. The shape intentionally
//! parallels `app/src/store/chatRuntimeSlice.ts` so a snapshot can
//! be applied directly to that slice.

use serde::{Deserialize, Serialize};

use crate::openhuman::agent::task_board::TaskBoard;

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
    /// The turn finished normally. The snapshot is **kept** (not deleted) so
    /// the chat "View processing" panel can replay the full transcript +
    /// tool timeline after a reload / cold boot — startup interrupted-marking
    /// skips this state, and the next turn on the thread overwrites it.
    Completed,
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
    /// High-level status: `"running"`, `"awaiting_user"`, `"completed"`,
    /// `"failed"`. `None` for legacy snapshots written before this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
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
    /// Persistent worker sub-thread backing this delegation, when one was
    /// created. Lets the UI reopen the full parent↔subagent conversation
    /// from memory after a cold boot / interrupted turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_thread_id: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<SubagentToolCall>,
    /// Ordered reasoning/narration/tool transcript for this sub-agent — what
    /// the inline "Agentic task insights" thoughts render from. Persisted (not
    /// live-only) so the thoughts survive a settled turn / reload.
    /// `#[serde(default)]` so snapshots written before this field load empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transcript: Vec<SubagentTranscriptItem>,
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
    /// Server-computed human label for this child call (e.g. "Reading file"),
    /// or `None` to defer to the client formatter. Mirrors the parent
    /// [`ToolTimelineEntry::display_name`] so the same reusable row renderer
    /// reads the same field for both main-agent and sub-agent calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Server-computed contextual detail (e.g. the path / recipient).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One ordered item in a sub-agent's processing transcript — its streamed
/// reasoning (`thinking`), visible narration (`text`), or a tool call, in the
/// exact order they occurred. Mirrors the frontend `SubagentTranscriptItem`
/// union 1:1 (order = push order; no `seq` is needed because each sub-agent's
/// transcript is built as a single ordered list). Persisting these lets the
/// inline "Agentic task insights" thoughts survive a settled turn / reload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// `rename_all` renames the variant tags; `rename_all_fields` renames the
// fields *inside* the struct variants (call_id → callId, …) — without the
// latter the FE would read `undefined` for camelCase fields.
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum SubagentTranscriptItem {
    /// The sub-agent's hidden reasoning.
    Thinking {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        iteration: Option<u32>,
        text: String,
    },
    /// The sub-agent's visible narration.
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        iteration: Option<u32>,
        text: String,
    },
    /// A child tool call at the point it occurred (self-contained so a
    /// rehydrated row renders without cross-referencing `tool_calls`).
    Tool {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        iteration: Option<u32>,
        call_id: String,
        tool_name: String,
        status: ToolTimelineStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_chars: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

/// One ordered item in the parent turn's processing transcript.
///
/// Unlike [`ToolTimelineEntry`] (a flat list of tool rows), the transcript
/// preserves the **interleaving** of the agent's visible narration, its
/// hidden reasoning, and its tool calls in the exact order they streamed —
/// so the chat "View processing" panel can render prose between tool groups
/// the way Claude / Hermes does. `seq` is a monotonic per-turn ordering key
/// (round alone can't order narration vs thinking within one round). Tool
/// items hold only a `call_id` pointer into [`TurnState::tool_timeline`] so
/// the row's status/label live in exactly one place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// `rename_all_fields` is required so the `ToolCall.call_id` field serializes as
// `callId` (the FE reads camelCase) — `rename_all` alone only renames variants.
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum TranscriptItem {
    /// The agent's visible assistant text between tool calls.
    Narration { round: u32, seq: u32, text: String },
    /// The agent's hidden reasoning (when the model emits it).
    Thinking { round: u32, seq: u32, text: String },
    /// A pointer to a tool row in [`TurnState::tool_timeline`].
    ToolCall {
        round: u32,
        seq: u32,
        call_id: String,
    },
}

/// Persisted snapshot of an in-flight (or just-finished) agent turn for one
/// thread.
///
/// Written to disk by the web-channel progress consumer at iteration
/// boundaries, tool start/complete, and on terminal events. On normal
/// completion it is marked [`TurnLifecycle::Completed`] and **kept** (so the
/// "View processing" panel can replay the finished turn's transcript); a
/// non-terminal snapshot surviving startup is marked
/// [`TurnLifecycle::Interrupted`].
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
    /// Ordered, interleaved record of the agent's narration, reasoning, and
    /// tool calls for the "View processing" panel. `#[serde(default)]` so
    /// snapshots written before this field still load (as empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transcript: Vec<TranscriptItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_board: Option<TaskBoard>,
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
            transcript: Vec::new(),
            task_board: None,
            started_at: now.clone(),
            updated_at: now,
        }
    }
}
