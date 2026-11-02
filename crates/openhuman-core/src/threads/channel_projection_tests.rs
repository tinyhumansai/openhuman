use super::*;
use crate::storage::{Scope, StorageBackend};
use std::sync::Arc;
use tinychannels_bus::ConversationKind;

fn repo(backend: &Arc<dyn StorageBackend>) -> Repo {
    Repo::on(backend, &Scope::local(), DOMAIN, collections).unwrap()
}

fn key(
    tenant_key: &str,
    account_id: Option<&str>,
    conversation_id: &str,
    topic_id: Option<&str>,
    provider_message_id: &str,
) -> MessageBindingKey {
    MessageBindingKey {
        tenant_key: tenant_key.to_owned(),
        channel: ChannelRef {
            id: "telegram".into(),
            account_id: account_id.map(str::to_owned),
        },
        conversation: ConversationRef {
            kind: ConversationKind::Thread,
            id: conversation_id.into(),
            scope_id: Some("workspace-1".into()),
            parent_id: Some("group-1".into()),
            thread_id: Some("thread-address-1".into()),
            topic_id: topic_id.map(str::to_owned),
        },
        provider_message_id: provider_message_id.into(),
    }
}

#[test]
fn provider_receipt_maps_to_the_exact_thread_and_row_after_restart() {
    let backend: Arc<dyn StorageBackend> =
        Arc::new(tinystoragedrivers_testkit::MemoryBackend::new());
    let first = repo(&backend);
    let binding = key(
        "tenant-agent-a",
        Some("account-1"),
        "conversation-1",
        Some("topic-1"),
        "provider-17",
    );

    bind_in(&first, binding.clone(), "channel:thread-a").unwrap();
    assert_eq!(
        resolve_in(&first, &binding).unwrap(),
        None,
        "a provider receipt is not a persisted UI row"
    );
    bind_row_in(&first, binding.clone(), "channel:thread-a", "user:row-9").unwrap();

    // A fresh Repository simulates process restart against the same backend.
    let restarted = repo(&backend);
    assert_eq!(
        resolve_in(&restarted, &binding).unwrap(),
        Some(ProjectedMessageTarget {
            thread_id: "channel:thread-a".into(),
            row_id: "user:row-9".into(),
        })
    );
}

#[test]
fn provider_receipts_are_scoped_by_tenant_account_and_full_conversation() {
    let backend: Arc<dyn StorageBackend> =
        Arc::new(tinystoragedrivers_testkit::MemoryBackend::new());
    let repo = repo(&backend);
    let original = key(
        "tenant-agent-a",
        Some("account-1"),
        "conversation-1",
        Some("topic-1"),
        "provider-17",
    );
    bind_in(&repo, original.clone(), "channel:thread-a").unwrap();
    bind_row_in(&repo, original.clone(), "channel:thread-a", "row-a").unwrap();

    for other in [
        key(
            "tenant-agent-b",
            Some("account-1"),
            "conversation-1",
            Some("topic-1"),
            "provider-17",
        ),
        key(
            "tenant-agent-a",
            Some("account-2"),
            "conversation-1",
            Some("topic-1"),
            "provider-17",
        ),
        key(
            "tenant-agent-a",
            Some("account-1"),
            "conversation-1",
            Some("topic-2"),
            "provider-17",
        ),
        key(
            "tenant-agent-a",
            Some("account-1"),
            "conversation-2",
            Some("topic-1"),
            "provider-17",
        ),
        key(
            "tenant-agent-a",
            Some("account-1"),
            "conversation-1",
            Some("topic-1"),
            "provider-18",
        ),
    ] {
        assert_eq!(resolve_in(&repo, &other).unwrap(), None);
    }
}

#[test]
fn sender_scoped_threads_can_share_a_conversation_without_overwriting() {
    let backend: Arc<dyn StorageBackend> =
        Arc::new(tinystoragedrivers_testkit::MemoryBackend::new());
    let repo = repo(&backend);
    let alice = key(
        "tenant",
        Some("account"),
        "group",
        Some("topic"),
        "alice-message",
    );
    let bob = key(
        "tenant",
        Some("account"),
        "group",
        Some("topic"),
        "bob-message",
    );

    bind_in(&repo, alice.clone(), "channel:alice").unwrap();
    bind_in(&repo, bob.clone(), "channel:bob").unwrap();
    bind_row_in(&repo, alice.clone(), "channel:alice", "row-alice").unwrap();
    bind_row_in(&repo, bob.clone(), "channel:bob", "row-bob").unwrap();
    assert_eq!(
        resolve_in(&repo, &alice).unwrap().unwrap().thread_id,
        "channel:alice"
    );
    assert_eq!(
        resolve_in(&repo, &bob).unwrap().unwrap().thread_id,
        "channel:bob"
    );
    assert!(bind_in(&repo, alice, "channel:other").is_err());
}

#[test]
fn channel_activity_emits_atomic_keyed_merge_patches() {
    use tinychannels_bus::{ChannelEvent, ChannelEventKind, ChannelReaction, ChannelRef};

    let event = |kind| ChannelEvent {
        version: 2,
        event_id: "outer-event-id".into(),
        source_event_id: None,
        source_timestamp: None,
        channel: ChannelRef {
            id: "telegram".into(),
            account_id: Some("account".into()),
        },
        conversation: None,
        event: kind,
    };
    let reaction = ChannelReaction {
        value: "🔥".into(),
        custom_emoji_id: None,
    };
    let added = event(ChannelEventKind::ReactionAdded {
        message_id: "provider-target".into(),
        provider_message_id: None,
        actor: tinychannels_bus::SenderRef {
            id: "alice".into(),
            ..Default::default()
        },
        reaction: reaction.clone(),
    });
    let patch = channel_activity_merge_patch(&added).unwrap();
    let reaction_entries = patch["channelActivity"]["reactions"].as_object().unwrap();
    assert_eq!(reaction_entries.len(), 1);
    let (reaction_key, reaction_value) = reaction_entries.iter().next().unwrap();
    assert_eq!(reaction_value["actorId"], "alice");
    assert_eq!(reaction_value["value"], "🔥");

    // The key encodes tuple boundaries, so provider values containing a
    // separator cannot alias another actor/reaction tuple.
    let separator_collision = reaction_digest(
        "a|b",
        &ChannelReaction {
            value: "c".into(),
            custom_emoji_id: None,
        },
    )
    .unwrap();
    let distinct_tuple = reaction_digest(
        "a",
        &ChannelReaction {
            value: "b|c".into(),
            custom_emoji_id: None,
        },
    )
    .unwrap();
    assert_ne!(separator_collision, distinct_tuple);
    let custom = reaction_digest(
        "alice",
        &ChannelReaction {
            value: "🔥".into(),
            custom_emoji_id: Some("provider-custom-1".into()),
        },
    )
    .unwrap();
    assert_ne!(*reaction_key, custom);

    let read = event(ChannelEventKind::ReadReceipt {
        message_id: "provider-target".into(),
        actor: Some(tinychannels_bus::SenderRef {
            id: "bob".into(),
            ..Default::default()
        }),
        timestamp: Some(42),
    });
    let read_patch = channel_activity_merge_patch(&read).unwrap();
    let read_entries = read_patch["channelActivity"]["readReceipts"]
        .as_object()
        .unwrap();
    assert_eq!(read_entries.len(), 1);
    assert_eq!(read_entries.values().next().unwrap()["actorId"], "bob");
    assert_eq!(read_entries.values().next().unwrap()["timestamp"], 42);

    let removed = event(ChannelEventKind::ReactionRemoved {
        message_id: "provider-target".into(),
        provider_message_id: None,
        actor: tinychannels_bus::SenderRef {
            id: "alice".into(),
            ..Default::default()
        },
        reaction,
    });
    let remove_patch = channel_activity_merge_patch(&removed).unwrap();
    assert_eq!(
        remove_patch["channelActivity"]["reactions"][reaction_key],
        Value::Null
    );
}

#[tokio::test]
async fn persisted_activity_merges_concurrent_events_for_the_exact_scoped_row() {
    // `forget` below models a restart; one installed backend keeps the rows
    // across it, as a durable store does.
    #[cfg(all(feature = "channels", feature = "modules"))]
    let _storage = crate::channels::module_fixture::install_memory_storage();
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::threads::store::{self, ConversationMessage, CreateConversationThread};
    use tinychannels_bus::{
        ChannelEvent, ChannelEventKind, ChannelReaction, ChannelRef, ConversationKind,
        ConversationRef, SenderRef,
    };

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
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
    let alice = make_ctx(&format!("activity-alice-{suffix}"));
    let bob = make_ctx(&format!("activity-bob-{suffix}"));
    let thread_id = format!("channel:activity:{suffix}");
    let row_id = "user:openhuman-row-42".to_owned();
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
    let provider_message_id = "provider-message-17";

    for ctx in [&alice, &bob] {
        store::blocking::ensure_thread(
            ctx.clone(),
            workspace.clone(),
            CreateConversationThread {
                id: thread_id.clone(),
                title: "Activity projection".into(),
                created_at: "2026-10-20T00:00:00Z".into(),
                parent_thread_id: None,
                labels: None,
                personality_id: None,
                working_dir: None,
            },
        )
        .await
        .unwrap();
        store::blocking::append_message(
            ctx.clone(),
            workspace.clone(),
            thread_id.clone(),
            ConversationMessage {
                id: row_id.clone(),
                content: "original provider message".into(),
                message_type: "text".into(),
                extra_metadata: serde_json::json!({
                    "voiceNote": {"opaqueId": "voice-asset-preserved"},
                    "custom": {"keep": true}
                }),
                sender: "user".into(),
                created_at: "2026-10-20T00:00:01Z".into(),
            },
        )
        .await
        .unwrap();
    }

    bind_message(
        &alice,
        &channel,
        &conversation,
        provider_message_id,
        &thread_id,
    )
    .unwrap();
    bind_row(
        &alice,
        &channel,
        &conversation,
        provider_message_id,
        &thread_id,
        &row_id,
    )
    .unwrap();
    let target = ProjectedMessageTarget {
        thread_id: thread_id.clone(),
        row_id: row_id.clone(),
    };
    let event = |event_id: &str, kind| ChannelEvent {
        version: 2,
        event_id: event_id.into(),
        source_event_id: None,
        source_timestamp: None,
        channel: channel.clone(),
        conversation: Some(conversation.clone()),
        event: kind,
    };
    let reaction = event(
        "event-reaction",
        ChannelEventKind::ReactionAdded {
            message_id: provider_message_id.into(),
            provider_message_id: None,
            actor: SenderRef {
                id: "reader-alice".into(),
                ..Default::default()
            },
            reaction: ChannelReaction {
                value: "👍".into(),
                custom_emoji_id: Some("provider-emoji-1".into()),
            },
        },
    );
    let receipt = event(
        "event-read",
        ChannelEventKind::ReadReceipt {
            message_id: provider_message_id.into(),
            actor: Some(SenderRef {
                id: "reader-bob".into(),
                ..Default::default()
            }),
            timestamp: Some(1_792_444_800),
        },
    );

    let (reaction_result, receipt_result) = tokio::join!(
        persist_activity(&alice, &target, &reaction),
        persist_activity(&alice, &target, &receipt),
    );
    reaction_result.unwrap();
    receipt_result.unwrap();

    let alice_rows =
        store::blocking::get_messages(alice.clone(), workspace.clone(), thread_id.clone())
            .await
            .unwrap();
    assert_eq!(alice_rows.len(), 1);
    let activity = &alice_rows[0].extra_metadata["channelActivity"];
    assert_eq!(activity["reactions"].as_object().unwrap().len(), 1);
    assert_eq!(activity["readReceipts"].as_object().unwrap().len(), 1);
    assert_eq!(
        alice_rows[0].extra_metadata["voiceNote"]["opaqueId"],
        "voice-asset-preserved"
    );
    assert_eq!(alice_rows[0].extra_metadata["custom"]["keep"], true);

    // Reopen the workspace backend and resolve the provider target under a
    // different profile. It must not find Alice's binding or mutate Bob's
    // same-named row.
    crate::storage::local::forget(&workspace);
    let restarted_alice = make_ctx(&format!("activity-alice-{suffix}"));
    let restarted_bob = make_ctx(&format!("activity-bob-{suffix}"));
    assert_eq!(
        resolve_message(&restarted_bob, &channel, &conversation, provider_message_id).unwrap(),
        None
    );
    let reloaded_alice =
        store::blocking::get_messages(restarted_alice, workspace.clone(), thread_id.clone())
            .await
            .unwrap();
    assert_eq!(reloaded_alice[0].id, row_id);
    assert_eq!(
        reloaded_alice[0].extra_metadata,
        alice_rows[0].extra_metadata
    );
    let untouched_bob = store::blocking::get_messages(restarted_bob, workspace, thread_id)
        .await
        .unwrap();
    assert_eq!(untouched_bob[0].id, row_id);
    assert_eq!(untouched_bob[0].content, "original provider message");
    assert_eq!(
        untouched_bob[0].extra_metadata["voiceNote"]["opaqueId"],
        "voice-asset-preserved"
    );
    assert!(untouched_bob[0]
        .extra_metadata
        .get("channelActivity")
        .is_none());
}
