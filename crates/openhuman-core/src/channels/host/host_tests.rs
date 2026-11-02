//! Unit tests for host-owned channel event projection.

use super::*;
use std::sync::Arc;
#[test]
fn typed_channel_projection_uses_the_target_message_id_not_the_event_id() {
    use tinychannels_bus::{
        ChannelEvent, ChannelEventKind, ChannelRef, ConversationKind, ConversationRef,
    };

    let event = ChannelEvent {
        version: 2,
        event_id: "event-99".into(),
        source_event_id: Some("source-99".into()),
        source_timestamp: None,
        channel: ChannelRef {
            id: "telegram".into(),
            account_id: Some("bot-1".into()),
        },
        conversation: Some(ConversationRef {
            kind: ConversationKind::Group,
            id: "group-1".into(),
            ..Default::default()
        }),
        event: ChannelEventKind::ReadReceipt {
            message_id: "provider-target-42".into(),
            actor: None,
            timestamp: None,
        },
    };
    assert_eq!(
        super::adapters::channel_event_target_message_id(&event),
        Some("provider-target-42")
    );
}

#[tokio::test]
async fn typed_channel_projection_uses_scoped_persisted_row_bindings_after_restart() {
    // The default per-workspace memory backend is dropped by `forget`, which
    // models a restart; installing one backend keeps the rows across it, the way
    // a durable store does.
    let _storage = crate::channels::module_fixture::install_memory_storage();
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use tinychannels_bus::{
        ChannelEvent, ChannelEventKind, ChannelReaction, ChannelRef, ConversationKind,
        ConversationRef, SenderRef,
    };

    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().to_path_buf();
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let make_ctx = |profile: &str| {
        let config = crate::config::Config {
            workspace_dir: workspace.clone(),
            action_dir: workspace.clone(),
            config_path: workspace.join("config.toml"),
            ..Default::default()
        };
        DispatchCtx::new(
            CoreContext::for_test(DomainSet::none(), Some(workspace.clone())).derive_with(
                ContextOverlay::new(config, DomainSet::full(), Default::default()).profile(profile),
            ),
        )
    };
    let alice = make_ctx(&format!("projection-alice-{suffix}"));
    let bob = make_ctx(&format!("projection-bob-{suffix}"));
    let channel = ChannelRef {
        id: "telegram".into(),
        account_id: Some("configured-account".into()),
    };
    let conversation = ConversationRef {
        kind: ConversationKind::Thread,
        id: "provider-conversation".into(),
        scope_id: Some("workspace-scope".into()),
        parent_id: Some("provider-parent".into()),
        thread_id: Some("provider-thread".into()),
        topic_id: Some("provider-topic".into()),
    };
    let provider_id = "provider-receipt-42";
    let thread_id = format!("channel:telegram:{suffix}");
    crate::threads::channel_projection::bind_message(
        &alice,
        &channel,
        &conversation,
        provider_id,
        &thread_id,
    )
    .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let runtime = tokio::runtime::Handle::current();
    let writers = (0..8).map(|_| {
        let barrier = Arc::clone(&barrier);
        let runtime = runtime.clone();
        let alice = alice.clone();
        let workspace = workspace.clone();
        let channel = channel.clone();
        let conversation = conversation.clone();
        let thread_id = thread_id.clone();
        tokio::task::spawn_blocking(move || {
            barrier.wait();
            runtime.block_on(crate::threads::store::persist_channel_message(
                &alice,
                &workspace,
                &channel,
                &conversation,
                &thread_id,
                "Provider conversation",
                provider_id,
                "user",
                "hello from Telegram",
                serde_json::json!({"voiceNote": {"opaqueId": "voice-asset-7"}}),
            ))
        })
    });
    let row_ids = futures::future::join_all(writers)
        .await
        .into_iter()
        .map(|result| result.unwrap().unwrap())
        .collect::<Vec<_>>();
    assert!(row_ids.iter().all(|id| id == "user:provider-receipt-42"));
    let row_id = row_ids[0].clone();
    let persisted = crate::threads::store::blocking::get_messages(
        alice.clone(),
        workspace.clone(),
        thread_id.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        persisted
            .iter()
            .filter(|message| message.id == row_id)
            .count(),
        1,
        "concurrent duplicate channel delivery persists one authoritative row"
    );
    assert_eq!(row_id, "user:provider-receipt-42");

    // Bob has a same-named row in another profile, but has no trusted binding
    // for this provider receipt. The outer event ID intentionally differs from
    // the provider target ID, so only the inner target can resolve Alice's row.
    crate::threads::store::blocking::ensure_thread(
        bob.clone(),
        workspace.clone(),
        crate::threads::store::CreateConversationThread {
            id: thread_id.clone(),
            title: "Bob's unrelated conversation".into(),
            created_at: "2026-10-20T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .unwrap();
    crate::threads::store::blocking::append_message(
        bob.clone(),
        workspace.clone(),
        thread_id.clone(),
        crate::threads::store::ConversationMessage {
            id: row_id.clone(),
            content: "Bob's unrelated row".into(),
            message_type: "text".into(),
            extra_metadata: serde_json::json!({"voiceNote": {"opaqueId": "bob-voice"}}),
            sender: "user".into(),
            created_at: "2026-10-20T00:00:00Z".into(),
        },
    )
    .await
    .unwrap();

    let event = ChannelEvent {
        version: 2,
        event_id: "provider-event-outer-99".into(),
        source_event_id: None,
        source_timestamp: None,
        channel: channel.clone(),
        conversation: Some(conversation.clone()),
        event: ChannelEventKind::ReactionAdded {
            message_id: provider_id.into(),
            provider_message_id: None,
            actor: SenderRef {
                id: "reader-alice".into(),
                ..Default::default()
            },
            reaction: ChannelReaction {
                value: "👍".into(),
                custom_emoji_id: Some("emoji:custom:1".into()),
            },
        },
    };

    assert!(super::adapters::project_channel_event(&bob, event.clone())
        .await
        .unwrap()
        .is_none());
    let bob_messages = crate::threads::store::blocking::get_messages(
        bob.clone(),
        workspace.clone(),
        thread_id.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        bob_messages[0].extra_metadata["voiceNote"]["opaqueId"],
        "bob-voice"
    );
    assert!(bob_messages[0]
        .extra_metadata
        .get("channelActivity")
        .is_none());

    let projected = super::adapters::project_channel_event(&alice, event.clone())
        .await
        .unwrap()
        .expect("exact trusted receipt should project");
    match projected {
        crate::web_chat::WebChannelEvent::ChannelEvent {
            openhuman_message_id,
            context,
            ..
        } => {
            assert_eq!(openhuman_message_id.as_deref(), Some(row_id.as_str()));
            assert_eq!(context.thread_id, thread_id);
        }
        other => panic!("expected typed channel event, got {other:?}"),
    }

    crate::storage::local::forget(&workspace);
    let restarted = make_ctx(&format!("projection-alice-{suffix}"));
    let read = ChannelEvent {
        event_id: "provider-event-outer-100".into(),
        event: ChannelEventKind::ReadReceipt {
            message_id: provider_id.into(),
            actor: Some(SenderRef {
                id: "reader-bob".into(),
                ..Default::default()
            }),
            timestamp: Some(1_792_444_800),
        },
        ..event
    };
    assert!(super::adapters::project_channel_event(&restarted, read)
        .await
        .unwrap()
        .is_some());
    let messages = crate::threads::store::blocking::get_messages(restarted, workspace, thread_id)
        .await
        .unwrap();
    let activity = &messages[0].extra_metadata["channelActivity"];
    assert_eq!(activity["reactions"].as_object().unwrap().len(), 1);
    assert_eq!(activity["readReceipts"].as_object().unwrap().len(), 1);
    assert_eq!(
        messages[0].extra_metadata["voiceNote"]["opaqueId"],
        "voice-asset-7"
    );
}

// --- OpenHumanEventSink --------------------------------------------------

#[tokio::test]
async fn event_sink_accepts_web_channel_event_shape() {
    let sink = OpenHumanEventSink;
    let ok = sink
        .publish_host(
            "web",
            "chat_done",
            serde_json::json!({
                "event": "chat_done",
                "client_id": "c1",
                "thread_id": "t1",
                "request_id": "r1",
                "full_response": "hi"
            }),
        )
        .await;
    assert!(ok.is_ok());
}

#[tokio::test]
async fn event_sink_rejects_non_object_payload() {
    let sink = OpenHumanEventSink;
    let err = sink
        .publish_host("web", "bad", serde_json::json!([1, 2, 3]))
        .await;
    assert!(err.is_err());
}

#[tokio::test]
async fn event_sink_ignores_legacy_reaction_callbacks() {
    let sink = OpenHumanEventSink;
    assert!(sink
        .publish_host(
            "channel",
            "reaction_received",
            serde_json::json!({
                "channel": "telegram", "sender": "u1",
                "target_message_id": "telegram_1_2", "emoji": "👍"
            }),
        )
        .await
        .is_ok());
    assert!(sink
        .publish_host(
            "channel",
            "reaction_sent",
            serde_json::json!({
                "channel": "telegram", "target_message_id": "telegram_1_2",
                "emoji": "👍", "success": true
            }),
        )
        .await
        .is_ok());
    // Unknown domain/kind is a benign no-op.
    assert!(sink
        .publish_host("mystery", "x", serde_json::json!({}))
        .await
        .is_ok());
    assert!(sink
        .publish_host("channel", "unknown", serde_json::json!({}))
        .await
        .is_ok());
}
