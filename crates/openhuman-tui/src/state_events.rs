//! Which web-channel events the Chat tab folds into the transcript, and which
//! it leaves alone on purpose.
//!
//! The event vocabulary is the tinychannels `WebChannelEvent` contract (the
//! same one the desktop app's generated TypeScript types come from). Every
//! tag in it is classified here, so a tag the contract gains cannot be dropped
//! silently: `state_events_tests.rs` checks the classification against the
//! contract's JSON schema.

/// What the Chat tab does with one event tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Disposition {
    /// Folded into the transcript by `TranscriptState::apply_event`.
    Reduced,
    /// Part of the contract, deliberately not shown in the terminal.
    Ignored,
    /// Not in the contract (a newer core); logged and skipped.
    Unknown,
}

/// Tags `apply_event` reduces. Every `subagent_*` tag is reduced too (see
/// [`disposition`]).
const REDUCED: &[&str] = &[
    "text_delta",
    "thinking_delta",
    "tool_call",
    "tool_result",
    "tool_args_delta",
    "artifact_pending",
    "artifact_ready",
    "artifact_failed",
    "chat_done",
    "channel_outbound",
    "chat_error",
];

/// Tags the terminal ignores, with the reason each is safe to skip.
const IGNORED: &[&str] = &[
    // Turn bookkeeping with nothing to show; the stream itself is the signal.
    "inference_start",
    "inference_heartbeat",
    "iteration_start",
    // The web UI shows a summarizing indicator; the terminal has no such row.
    "context_compacting",
    "turn_cost",
    "memory_activity",
    // `chat_error` already ends a cancelled turn (the core sends both for a
    // release), and `chat_interim`/`chat_segment`/`chat_message` repeat text
    // the deltas and `chat_done.full_response` already delivered.
    "chat_cancelled",
    "chat_interim",
    "chat_segment",
    "chat_message",
    // Live tool progress text; the call/result rows carry the outcome.
    "tool_progress",
    // Follow-up chips, run queue and plan/goal/todo panels are desktop UI.
    "chat_suggestions",
    "queue_item_queued",
    "queue_item_delivered",
    "queue_item_removed",
    "thread_goal_updated",
    "thread_goal_cleared",
    "thread_todos_changed",
    "run_mode_changed",
    // The terminal has its own approval prompt; these are the web cards.
    "approval_request",
    "approval_decided",
    "plan_review_request",
    "plan_review_decided",
    "external_transfer_pending",
    // Not tied to this client's turn (system client id) or UI-only.
    "proactive_message",
    "voice_speak",
    "user_error",
    // Canonical provider read/reaction events are not transcript rows in the TUI.
    "channel_event",
];

/// Classify one event tag.
pub(crate) fn disposition(event: &str) -> Disposition {
    if event.starts_with("subagent_") || REDUCED.contains(&event) {
        Disposition::Reduced
    } else if IGNORED.contains(&event) {
        Disposition::Ignored
    } else {
        Disposition::Unknown
    }
}

#[cfg(test)]
#[path = "state_events_tests.rs"]
mod tests;
