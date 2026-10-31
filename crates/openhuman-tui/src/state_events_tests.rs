use super::*;

#[test]
fn current_transcript_events_are_reduced() {
    for event in [
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
    ] {
        assert_eq!(disposition(event), Disposition::Reduced, "{event}");
    }
}

#[test]
fn desktop_controls_and_background_events_are_ignored() {
    for event in [
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
    ] {
        assert_eq!(disposition(event), Disposition::Ignored, "{event}");
    }
}

#[test]
fn retired_subagent_stream_tags_are_unknown() {
    assert_eq!(disposition("subagent_spawned"), Disposition::Unknown);
    assert_eq!(disposition("subagent_text_delta"), Disposition::Unknown);
}

#[test]
fn dispositions_cover_the_three_cases() {
    assert_eq!(disposition("chat_done"), Disposition::Reduced);
    assert_eq!(disposition("turn_cost"), Disposition::Unknown);
    assert_eq!(disposition("channel_event"), Disposition::Ignored);
    assert_eq!(disposition("from_the_future"), Disposition::Unknown);
}
