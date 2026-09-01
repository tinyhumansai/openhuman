use tempfile::TempDir;

use super::*;

#[test]
fn subscriber_reads_rebound_workspace_from_shared_handle() {
    let tmp = tempfile::TempDir::new().unwrap();
    let first = tmp.path().join("first");
    let second = tmp.path().join("second");
    let shared = Arc::new(RwLock::new(first.clone()));
    let subscriber = ConversationPersistenceSubscriber::new_shared(Arc::clone(&shared));

    assert_eq!(subscriber.workspace_dir_snapshot().unwrap(), first);
    *shared.write().unwrap() = second.clone();
    assert_eq!(subscriber.workspace_dir_snapshot().unwrap(), second);
}

#[tokio::test]
async fn persists_inbound_and_processed_turns_into_workspace_thread() {
    let temp = TempDir::new().expect("tempdir");
    let subscriber = ConversationPersistenceSubscriber::new(temp.path().to_path_buf());
    let mut inbound_envelope =
        tinychannels_bus::inbound_envelope_from_legacy_message(&ChannelMessage {
            channel: "slack".into(),
            id: "m1".into(),
            sender: "alice".into(),
            reply_target: "general".into(),
            content: "hello".into(),
            thread_ts: Some("thread-1".into()),
            timestamp: 0,
        });
    inbound_envelope.conversation.kind = tinychannels_bus::channel::ConversationKind::Channel;
    inbound_envelope.conversation.scope_id = Some("T123".into());

    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "slack".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "general".into(),
            content: "hello".into(),
            thread_ts: Some("thread-1".into()),
            inbound_envelope: Some(inbound_envelope),
            workspace_dir: temp.path().to_path_buf(),
        })
        .await;
    subscriber
        .handle(&DomainEvent::ChannelMessageProcessed {
            channel: "slack".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "general".into(),
            content: "hello".into(),
            thread_ts: Some("thread-1".into()),
            response: "hi there".into(),
            provider: "test-provider".into(),
            model: "test-model".into(),
            elapsed_ms: 42,
            success: true,
            workspace_dir: temp.path().to_path_buf(),
        })
        .await;

    let threads = crate::openhuman::memory::conversations::list_threads(temp.path().to_path_buf())
        .expect("threads");
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].id, "channel:slack_alice_general_thread:thread-1");

    let messages = crate::openhuman::memory::conversations::get_messages(
        temp.path().to_path_buf(),
        &threads[0].id,
    )
    .expect("messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].id, "user:m1");
    assert_eq!(messages[0].sender, "user");
    assert_eq!(
        messages[0].extra_metadata["tinychannelsSessionKey"],
        "main:slack:default:channel:T123:general:thread-1"
    );
    assert_eq!(messages[1].id, "assistant:m1");
    assert_eq!(messages[1].sender, "assistant");
    assert_eq!(messages[1].extra_metadata["elapsedMs"], 42);
    assert_eq!(messages[1].extra_metadata["success"], true);
    assert_eq!(messages[1].extra_metadata["modelProvider"], "test-provider");
    assert_eq!(messages[1].extra_metadata["model"], "test-model");
}

#[tokio::test]
async fn telegram_thread_ts_does_not_split_persisted_thread() {
    let temp = TempDir::new().expect("tempdir");
    let subscriber = ConversationPersistenceSubscriber::new(temp.path().to_path_buf());

    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "telegram".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-1".into(),
            content: "hello".into(),
            thread_ts: Some("100".into()),
            inbound_envelope: None,
            workspace_dir: temp.path().to_path_buf(),
        })
        .await;
    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "telegram".into(),
            message_id: "m2".into(),
            sender: "alice".into(),
            reply_target: "chat-1".into(),
            content: "follow-up".into(),
            thread_ts: Some("200".into()),
            inbound_envelope: None,
            workspace_dir: temp.path().to_path_buf(),
        })
        .await;

    let threads = crate::openhuman::memory::conversations::list_threads(temp.path().to_path_buf())
        .expect("threads");
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].id, "channel:telegram_alice_chat-1");
}

#[tokio::test]
async fn duplicate_events_do_not_append_duplicate_messages() {
    let temp = TempDir::new().expect("tempdir");
    let subscriber = ConversationPersistenceSubscriber::new(temp.path().to_path_buf());

    let event = DomainEvent::ChannelMessageReceived {
        channel: "discord".into(),
        message_id: "m1".into(),
        sender: "alice".into(),
        reply_target: "room-1".into(),
        content: "hello".into(),
        thread_ts: None,
        inbound_envelope: None,
        workspace_dir: temp.path().to_path_buf(),
    };

    subscriber.handle(&event).await;
    subscriber.handle(&event).await;

    let messages = crate::openhuman::memory::conversations::get_messages(
        temp.path().to_path_buf(),
        "channel:discord_alice_room-1",
    )
    .expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "user:m1");
}

#[test]
fn persisted_channel_thread_id_ignores_blank_thread_ts() {
    let without = persisted_channel_thread_id("slack", "alice", "general", None);
    let with_blank = persisted_channel_thread_id("slack", "alice", "general", Some("   "));
    assert_eq!(without, with_blank);
}

#[test]
fn channel_thread_title_uses_thread_suffix_only_for_non_telegram_threads() {
    assert_eq!(
        channel_thread_title("slack", "alice", "general", Some(" 123 ")),
        "slack · alice · general · thread 123"
    );
    assert_eq!(
        channel_thread_title("telegram", "alice", "chat-1", Some("123")),
        "telegram · alice · chat-1"
    );
}

#[test]
fn non_empty_trimmed_rejects_blank_strings() {
    assert_eq!(non_empty_trimmed("  hello  "), Some("hello"));
    assert_eq!(non_empty_trimmed("   "), None);
    assert_eq!(non_empty_trimmed(""), None);
}

// ── Workspace-identity guard tests ───────────────────────────────────────

/// Positive control: a `ChannelMessageReceived` event whose workspace matches
/// the subscriber's workspace IS persisted.
#[tokio::test]
async fn received_matching_workspace_is_persisted() {
    let temp = TempDir::new().expect("tempdir");
    let subscriber = ConversationPersistenceSubscriber::new(temp.path().to_path_buf());

    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "slack".into(),
            message_id: "m1".into(),
            sender: "bob".into(),
            reply_target: "dev".into(),
            content: "hello".into(),
            thread_ts: None,
            inbound_envelope: None,
            workspace_dir: temp.path().to_path_buf(),
        })
        .await;

    let messages = crate::openhuman::memory::conversations::get_messages(
        temp.path().to_path_buf(),
        "channel:slack_bob_dev",
    )
    .expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "user:m1");
}

/// `ChannelMessageReceived` with a mismatched workspace must be silently dropped —
/// nothing persisted in the subscriber's workspace.
#[tokio::test]
async fn received_stale_workspace_is_dropped() {
    let temp = TempDir::new().expect("tempdir");
    let stale = TempDir::new().expect("stale tempdir");
    let subscriber = ConversationPersistenceSubscriber::new(temp.path().to_path_buf());

    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "slack".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "general".into(),
            content: "should not persist".into(),
            thread_ts: None,
            inbound_envelope: None,
            workspace_dir: stale.path().to_path_buf(),
        })
        .await;

    // No thread should have been created in temp (the subscriber's workspace).
    let threads = crate::openhuman::memory::conversations::list_threads(temp.path().to_path_buf())
        .expect("threads");
    assert!(
        threads.is_empty(),
        "stale-workspace event must not create a thread"
    );
}

/// `ChannelMessageProcessed` with matching workspace is appended correctly
/// (positive control for the processed-event guard).
#[tokio::test]
async fn processed_matching_workspace_is_appended() {
    let temp = TempDir::new().expect("tempdir");
    let subscriber = ConversationPersistenceSubscriber::new(temp.path().to_path_buf());

    // Seed the received event first so a thread exists.
    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "slack".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "general".into(),
            content: "hello".into(),
            thread_ts: None,
            inbound_envelope: None,
            workspace_dir: temp.path().to_path_buf(),
        })
        .await;

    subscriber
        .handle(&DomainEvent::ChannelMessageProcessed {
            channel: "slack".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "general".into(),
            content: "hello".into(),
            thread_ts: None,
            response: "hi there".into(),
            provider: "test-provider".into(),
            model: "test-model".into(),
            elapsed_ms: 10,
            success: true,
            workspace_dir: temp.path().to_path_buf(),
        })
        .await;

    let messages = crate::openhuman::memory::conversations::get_messages(
        temp.path().to_path_buf(),
        "channel:slack_alice_general",
    )
    .expect("messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].id, "assistant:m1");
}

/// `ChannelMessageProcessed` with a mismatched workspace must not be appended,
/// even if a prior `ChannelMessageReceived` for the correct workspace was already
/// persisted.
#[tokio::test]
async fn processed_stale_workspace_is_dropped() {
    let temp = TempDir::new().expect("tempdir");
    let stale = TempDir::new().expect("stale tempdir");
    let subscriber = ConversationPersistenceSubscriber::new(temp.path().to_path_buf());

    // Persist the inbound message from the correct workspace.
    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "slack".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "general".into(),
            content: "hello".into(),
            thread_ts: None,
            inbound_envelope: None,
            workspace_dir: temp.path().to_path_buf(),
        })
        .await;

    // Then try to process with a stale workspace — must be dropped.
    subscriber
        .handle(&DomainEvent::ChannelMessageProcessed {
            channel: "slack".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "general".into(),
            content: "hello".into(),
            thread_ts: None,
            response: "should not persist".into(),
            provider: "test-provider".into(),
            model: "test-model".into(),
            elapsed_ms: 10,
            success: true,
            workspace_dir: stale.path().to_path_buf(),
        })
        .await;

    let messages = crate::openhuman::memory::conversations::get_messages(
        temp.path().to_path_buf(),
        "channel:slack_alice_general",
    )
    .expect("messages");
    // Only the user turn should be present; the stale processed event must be dropped.
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "user:m1");
}

/// Simulate the exact workspace-switch race:
/// 1. `ChannelMessageReceived` from workspace A — persisted.
/// 2. `ChannelMessageProcessed` from workspace B — dropped.
/// 3. `ChannelMessageProcessed` from workspace A — persisted.
/// Verify only workspace A's events appear.
#[tokio::test]
async fn workspace_switch_mid_conversation() {
    let workspace_a = TempDir::new().expect("workspace_a");
    let workspace_b = TempDir::new().expect("workspace_b");

    // Subscriber is bound to workspace A.
    let subscriber = ConversationPersistenceSubscriber::new(workspace_a.path().to_path_buf());

    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "telegram".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-1".into(),
            content: "hello".into(),
            thread_ts: None,
            inbound_envelope: None,
            workspace_dir: workspace_a.path().to_path_buf(),
        })
        .await;

    // Stale processed event from workspace B — must be dropped.
    subscriber
        .handle(&DomainEvent::ChannelMessageProcessed {
            channel: "telegram".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-1".into(),
            content: "hello".into(),
            thread_ts: None,
            response: "from workspace B — must be dropped".into(),
            provider: "test-provider".into(),
            model: "test-model".into(),
            elapsed_ms: 5,
            success: true,
            workspace_dir: workspace_b.path().to_path_buf(),
        })
        .await;

    // Correct processed event from workspace A — must be persisted.
    subscriber
        .handle(&DomainEvent::ChannelMessageProcessed {
            channel: "telegram".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-1".into(),
            content: "hello".into(),
            thread_ts: None,
            response: "from workspace A — should persist".into(),
            provider: "test-provider".into(),
            model: "test-model".into(),
            elapsed_ms: 10,
            success: true,
            workspace_dir: workspace_a.path().to_path_buf(),
        })
        .await;

    let messages = crate::openhuman::memory::conversations::get_messages(
        workspace_a.path().to_path_buf(),
        "channel:telegram_alice_chat-1",
    )
    .expect("messages");

    assert_eq!(messages.len(), 2, "only user + correct assistant turn");
    assert_eq!(messages[0].id, "user:m1");
    assert_eq!(messages[1].id, "assistant:m1");
    assert_eq!(
        messages[1].content, "from workspace A — should persist",
        "workspace B response must not have been written"
    );
}

/// Events from 3 different wrong workspaces all get dropped; nothing persists.
#[tokio::test]
async fn multiple_stale_workspaces_all_dropped() {
    let temp = TempDir::new().expect("tempdir");
    let stale_a = TempDir::new().expect("stale_a");
    let stale_b = TempDir::new().expect("stale_b");
    let stale_c = TempDir::new().expect("stale_c");

    let subscriber = ConversationPersistenceSubscriber::new(temp.path().to_path_buf());

    for (i, stale) in [&stale_a, &stale_b, &stale_c].iter().enumerate() {
        subscriber
            .handle(&DomainEvent::ChannelMessageReceived {
                channel: "discord".into(),
                message_id: format!("m{i}"),
                sender: "alice".into(),
                reply_target: "room-1".into(),
                content: format!("msg {i}"),
                thread_ts: None,
                inbound_envelope: None,
                workspace_dir: stale.path().to_path_buf(),
            })
            .await;
    }

    let threads = crate::openhuman::memory::conversations::list_threads(temp.path().to_path_buf())
        .expect("threads");
    assert!(
        threads.is_empty(),
        "no events from wrong workspaces should create a thread"
    );
}

/// After a stale event is dropped, a subsequent matching-workspace event is
/// still persisted correctly.
#[tokio::test]
async fn correct_workspace_after_stale_events() {
    let temp = TempDir::new().expect("tempdir");
    let stale = TempDir::new().expect("stale tempdir");
    let subscriber = ConversationPersistenceSubscriber::new(temp.path().to_path_buf());

    // Stale event first.
    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "slack".into(),
            message_id: "m0".into(),
            sender: "alice".into(),
            reply_target: "general".into(),
            content: "stale".into(),
            thread_ts: None,
            inbound_envelope: None,
            workspace_dir: stale.path().to_path_buf(),
        })
        .await;

    // Now a matching-workspace event.
    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "slack".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "general".into(),
            content: "valid".into(),
            thread_ts: None,
            inbound_envelope: None,
            workspace_dir: temp.path().to_path_buf(),
        })
        .await;

    let messages = crate::openhuman::memory::conversations::get_messages(
        temp.path().to_path_buf(),
        "channel:slack_alice_general",
    )
    .expect("messages");
    assert_eq!(
        messages.len(),
        1,
        "only the valid event should be persisted"
    );
    assert_eq!(messages[0].id, "user:m1");
    assert_eq!(messages[0].content, "valid");
}
