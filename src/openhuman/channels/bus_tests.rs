use super::*;
use crate::api::rest::BackendApiError;
use crate::core::event_bus::DomainEvent;

#[test]
fn subscriber_metadata_is_stable() {
    let subscriber = ChannelInboundSubscriber::new();
    assert_eq!(subscriber.name(), "channel::inbound_handler");
    assert_eq!(subscriber.domains(), Some(&["channel"][..]));
}

#[tokio::test]
async fn unrelated_events_are_ignored() {
    ChannelInboundSubscriber::default()
        .handle(&DomainEvent::SystemStartup {
            component: "test".into(),
        })
        .await;
}

// ── Channel-message edit failures (#5230) ──────────────────────────────────
//
// The backend implements no `PATCH /channels/:channel/messages/:messageId`, so
// every edit 404s. That 404 used to be classified as `MessageNotFound` ("the
// user deleted this message"), and the recovery for *that* condition is to
// forget the message id — which orphaned the streaming draft and left the
// ephemeral "💭 Thinking:" bubble in the chat permanently, because finalization
// then had no id left to delete.

#[test]
fn edit_failure_classification_separates_route_absence_from_message_absence() {
    let route_absent = anyhow::Error::new(BackendApiError::ChannelEditUnsupported {
        provider: "telegram".into(),
        message_id: "1103".into(),
    });
    assert_eq!(
        classify_edit_failure(&route_absent),
        EditFailure::RouteUnsupported
    );

    let message_absent = anyhow::Error::new(BackendApiError::MessageNotFound {
        provider: "telegram".into(),
        message_id: "1103".into(),
    });
    assert_eq!(
        classify_edit_failure(&message_absent),
        EditFailure::MessageGone
    );

    // Anything untyped (5xx, transport, rate limit) keeps the retry budget.
    let transient = anyhow::anyhow!("502 Bad Gateway");
    assert_eq!(classify_edit_failure(&transient), EditFailure::Transient);

    // A different typed variant must not be mistaken for either edit case.
    let unauthorized = anyhow::Error::new(BackendApiError::Unauthorized {
        method: "PATCH".into(),
        path: "/channels/telegram/messages/1103".into(),
    });
    assert_eq!(classify_edit_failure(&unauthorized), EditFailure::Transient);
}

#[test]
fn route_absence_keeps_the_draft_id_so_finalize_can_clean_it_up() {
    let mut state = StreamingState {
        message_id: Some("1103".into()),
        draft_sent: true,
        ..Default::default()
    };

    state.latch_draft_edits_unsupported();

    assert!(state.edit_disabled, "edits must stop being attempted");
    assert_eq!(
        state.message_id.as_deref(),
        Some("1103"),
        "the draft is still on screen and still ours — finalize needs its id to \
         delete it before posting the canonical reply (#5230)"
    );
}

#[test]
fn message_absence_forgets_the_draft_id() {
    let mut state = StreamingState {
        message_id: Some("1103".into()),
        draft_sent: true,
        ..Default::default()
    };

    state.forget_draft();

    assert!(state.edit_disabled);
    assert_eq!(
        state.message_id, None,
        "a message that is genuinely gone leaves nothing to edit or delete"
    );
}

#[test]
fn route_absence_keeps_the_thinking_bubble_id_so_it_still_gets_deleted() {
    let mut state = StreamingState {
        thinking_message_id: Some("2201".into()),
        thinking_sent: true,
        ..Default::default()
    };

    state.latch_thinking_edits_unsupported();

    assert!(state.thinking_edit_disabled);
    assert_eq!(
        state.thinking_message_id.as_deref(),
        Some("2201"),
        "finalization deletes the ephemeral bubble by id — dropping it here is \
         why '💭 Thinking:' stayed in the chat forever (#5230)"
    );
}

#[test]
fn message_absence_forgets_the_thinking_bubble_id() {
    let mut state = StreamingState {
        thinking_message_id: Some("2201".into()),
        thinking_sent: true,
        ..Default::default()
    };

    state.forget_thinking();

    assert!(state.thinking_edit_disabled);
    assert_eq!(state.thinking_message_id, None);
}

#[test]
fn edit_capability_latches_per_provider_not_per_chat() {
    // Provider names unique to this test: the latch is process-wide by design,
    // so sharing `telegram` with another test would couple them.
    assert!(!channel_edits_unsupported("noedit-a:chat-1"));

    mark_channel_edits_unsupported("noedit-a:chat-1");

    assert!(
        channel_edits_unsupported("noedit-a:chat-1"),
        "the channel that taught us must be latched"
    );
    assert!(
        channel_edits_unsupported("noedit-a:chat-2"),
        "route existence is a per-provider backend fact, so a different chat on \
         the same provider must not re-probe it"
    );
    assert!(
        channel_edits_unsupported("noedit-a"),
        "un-prefixed channel ids resolve to the same provider key"
    );
    assert!(
        !channel_edits_unsupported("noedit-b:chat-1"),
        "an unrelated provider must be unaffected"
    );
}

#[test]
fn marking_edit_capability_twice_is_idempotent() {
    mark_channel_edits_unsupported("noedit-c:chat-1");
    mark_channel_edits_unsupported("noedit-c:chat-1");
    assert!(channel_edits_unsupported("noedit-c:chat-1"));
}

#[test]
fn edit_capability_key_strips_the_chat_suffix() {
    assert_eq!(edit_capability_key("telegram:12345"), "telegram");
    assert_eq!(edit_capability_key("discord:guild:chan"), "discord");
    assert_eq!(edit_capability_key("telegram"), "telegram");
    assert_eq!(edit_capability_key(""), "");
}
