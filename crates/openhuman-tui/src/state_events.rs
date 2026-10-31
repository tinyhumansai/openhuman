//! Which web-channel events the Chat tab folds into the transcript, and which
//! it leaves alone on purpose.
//!
//! The event vocabulary is TinyChannels' `WebChannelEvent` contract. The TUI
//! reduces transcript content and ignores desktop-only controls.

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

const REDUCED: &[&str] = &[
    "text_delta",
    "thinking_delta",
    "tool_call",
    "tool_result",
    "artifact_pending",
    "artifact_ready",
    "artifact_failed",
    "chat_done",
    "channel_outbound",
    "chat_error",
];

const IGNORED: &[&str] = &[
    "chat_cancelled",
    "chat_interim",
    "chat_suggestions",
    "user_error",
    "tool_progress",
    "proactive_message",
    "queue_item_removed",
    "queue_item_queued",
    "queue_item_delivered",
    "external_transfer_pending",
    "memory_activity",
    "thread_goal_updated",
    "thread_goal_cleared",
    "thread_todos_changed",
    "run_mode_changed",
    "approval_request",
    "plan_review_request",
    "approval_decided",
    "plan_review_decided",
    "voice_speak",
    "channel_event",
];

/// Classify one event tag.
pub(crate) fn disposition(event: &str) -> Disposition {
    if REDUCED.contains(&event) {
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
