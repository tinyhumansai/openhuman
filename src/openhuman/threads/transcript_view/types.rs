//! Typed display items for the transcript projection RPC
//! (`threads.transcript_get`).
//!
//! These mirror the frontend's existing chat vocabulary (user/assistant
//! bubbles, reasoning drawer, tool timeline rows, sub-agent activity) so the
//! Phase C renderer can map them onto the same components. Serde is camelCase
//! on the wire — the frontend reads `displayContent`, `callId`, `requestId`,
//! etc.

use serde::Serialize;

/// Terminal state of a projected tool call. Mirrors the live timeline's
/// `ToolTimelineStatus` vocabulary (`running` / `success` / `error`) so the
/// settled projection and the live stream render identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    /// Call issued but no result line has been paired yet.
    Running,
    /// A result line was paired to the call.
    Success,
    /// A result line the projection identified as a **failure**: the persisted
    /// tool line carried the additive `failure` flag (stamped at turn-loop
    /// persistence from the tool's `ToolResult::is_error` outcome). Paired with
    /// a [`ToolCallFailure`] payload on the item.
    Error,
}

/// Failure payload attached to an errored [`DisplayItem::ToolCall`]. Minimal by
/// design: the persisted transcript only records that the call failed plus an
/// optional short reason. The frontend mapper expands this into its richer
/// `ToolFailureExplanation` shape (`class` / `category` / `causePlain` /
/// `nextAction`) for the `ToolFailureLines` renderer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallFailure {
    /// Short, single-line reason for the failure, when the writer captured one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One item in a projected transcript, in the frontend's display vocabulary.
///
/// `#[serde(tag = "kind")]` gives each variant a camelCase discriminator
/// (`userMessage`, `assistantMessage`, …) and every field is camelCase.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DisplayItem {
    /// A user prompt. `content` is the raw persisted content (may carry the
    /// injected `Current Date & Time:` scaffolding line); `displayContent` is
    /// the sanitized version to show, present only when it differs from raw.
    UserMessage {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        display_content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
    /// An assistant answer. `interim: true` marks a non-terminal tool-calling
    /// step within a multi-iteration turn (not the final answer bubble).
    AssistantMessage {
        content: String,
        #[serde(default, skip_serializing_if = "is_false")]
        interim: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        iteration: Option<u32>,
    },
    /// The model's reasoning/thinking that preceded an assistant message.
    Reasoning { text: String },
    /// A tool invocation with its paired result, when available.
    ToolCall {
        call_id: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        status: ToolCallStatus,
        /// Present only when `status` is `Error` — the failure payload the
        /// frontend expands for the `ToolFailureLines` renderer.
        #[serde(skip_serializing_if = "Option::is_none")]
        failure: Option<ToolCallFailure>,
    },
    /// A delegated sub-agent run, with its own nested projected items.
    ///
    /// `request_id` anchors the whole sub-agent trail to the parent turn that
    /// spawned it. Sub-agent transcripts are sibling files with no explicit
    /// back-link to the delegating tool call, so the projection derives this by
    /// matching the sub-agent's spawn timestamp (encoded in its file stem)
    /// against the parent turns' timestamp ranges (see
    /// `project::anchor_request_id`). Absent for legacy/CLI transcripts whose
    /// lines carry no `request_id`.
    Subagent {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        items: Vec<DisplayItem>,
    },
    /// A turn boundary — emitted when the `request_id` changes between lines.
    TurnBoundary { request_id: String },
    /// A partial assistant answer captured when a turn was interrupted.
    InterruptedPartial {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking: Option<String>,
    },
    /// A context-compaction marker: the reduced set replaced everything before
    /// it. Counts describe what the record superseded/installed.
    Compaction {
        replaced_count: usize,
        kept_count: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        ts: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// A projected transcript for one thread, before pagination. Chronological
/// (file) order; the RPC layer paginates newest-first.
#[derive(Debug, Clone)]
pub struct ProjectedTranscript {
    pub thread_id: String,
    /// All top-level display items in chronological order.
    pub items: Vec<DisplayItem>,
}
