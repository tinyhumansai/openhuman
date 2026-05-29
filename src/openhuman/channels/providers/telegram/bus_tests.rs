use super::bus::TelegramRemoteSubscriber;
use crate::core::event_bus::{DomainEvent, EventHandler};
use tempfile::tempdir;

#[tokio::test]
async fn subscriber_marks_busy_on_received_and_clears_on_processed() {
    let dir = tempdir().expect("tempdir");
    let subscriber = TelegramRemoteSubscriber::new(dir.path().to_path_buf());
    assert_eq!(subscriber.name(), "telegram::remote_control");
    assert_eq!(subscriber.domains(), Some(&["channel"][..]));

    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "telegram".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-99".into(),
            content: "hi".into(),
            thread_ts: Some("1".into()),
            workspace_dir: dir.path().to_path_buf(),
        })
        .await;

    let busy = super::session_store::with_store(dir.path(), |store| Ok(store.is_busy("chat-99")))
        .expect("store");
    assert!(busy);

    subscriber
        .handle(&DomainEvent::ChannelMessageProcessed {
            channel: "telegram".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-99".into(),
            content: "hi".into(),
            thread_ts: Some("1".into()),
            response: "ok".into(),
            elapsed_ms: 10,
            success: true,
            workspace_dir: dir.path().to_path_buf(),
        })
        .await;

    let busy = super::session_store::with_store(dir.path(), |store| Ok(store.is_busy("chat-99")))
        .expect("store");
    assert!(!busy);
}

#[tokio::test]
async fn subscriber_ignores_non_telegram_channel_events() {
    let dir = tempdir().expect("tempdir");
    let subscriber = TelegramRemoteSubscriber::new(dir.path().to_path_buf());

    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "discord".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-99".into(),
            content: "hi".into(),
            thread_ts: None,
            workspace_dir: dir.path().to_path_buf(),
        })
        .await;

    let busy = super::session_store::with_store(dir.path(), |store| Ok(store.is_busy("chat-99")))
        .expect("store");
    assert!(!busy);
}

// ── Workspace-identity guard tests ───────────────────────────────────────────

/// Positive control: matching workspace sets busy state as expected.
#[tokio::test]
async fn telegram_received_matching_workspace_sets_busy() {
    let dir = tempdir().expect("tempdir");
    let subscriber = TelegramRemoteSubscriber::new(dir.path().to_path_buf());

    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "telegram".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-10".into(),
            content: "hi".into(),
            thread_ts: None,
            workspace_dir: dir.path().to_path_buf(),
        })
        .await;

    let busy = super::session_store::with_store(dir.path(), |store| Ok(store.is_busy("chat-10")))
        .expect("store");
    assert!(busy, "matching workspace should mark busy");
}

/// Stale workspace on `ChannelMessageReceived` — busy must NOT be set.
#[tokio::test]
async fn telegram_received_stale_workspace_does_not_set_busy() {
    let dir = tempdir().expect("tempdir");
    let stale = tempdir().expect("stale tempdir");
    let subscriber = TelegramRemoteSubscriber::new(dir.path().to_path_buf());

    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "telegram".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-20".into(),
            content: "hi".into(),
            thread_ts: None,
            workspace_dir: stale.path().to_path_buf(),
        })
        .await;

    let busy = super::session_store::with_store(dir.path(), |store| Ok(store.is_busy("chat-20")))
        .expect("store");
    assert!(!busy, "stale workspace should not set busy");
}

/// Matching workspace on `ChannelMessageProcessed` clears busy state correctly.
#[tokio::test]
async fn telegram_processed_matching_workspace_clears_busy() {
    let dir = tempdir().expect("tempdir");
    let subscriber = TelegramRemoteSubscriber::new(dir.path().to_path_buf());

    // First mark as busy via a matching received event.
    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "telegram".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-30".into(),
            content: "hi".into(),
            thread_ts: None,
            workspace_dir: dir.path().to_path_buf(),
        })
        .await;

    let busy = super::session_store::with_store(dir.path(), |store| Ok(store.is_busy("chat-30")))
        .expect("store");
    assert!(busy, "should be busy after received");

    // Now clear with a matching processed event.
    subscriber
        .handle(&DomainEvent::ChannelMessageProcessed {
            channel: "telegram".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-30".into(),
            content: "hi".into(),
            thread_ts: None,
            response: "done".into(),
            elapsed_ms: 50,
            success: true,
            workspace_dir: dir.path().to_path_buf(),
        })
        .await;

    let busy = super::session_store::with_store(dir.path(), |store| Ok(store.is_busy("chat-30")))
        .expect("store");
    assert!(!busy, "matching processed should clear busy");
}

/// Stale workspace on `ChannelMessageProcessed` — busy must NOT be cleared.
#[tokio::test]
async fn telegram_processed_stale_workspace_does_not_clear_busy() {
    let dir = tempdir().expect("tempdir");
    let stale = tempdir().expect("stale tempdir");
    let subscriber = TelegramRemoteSubscriber::new(dir.path().to_path_buf());

    // Mark as busy via a matching received event.
    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "telegram".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-40".into(),
            content: "hi".into(),
            thread_ts: None,
            workspace_dir: dir.path().to_path_buf(),
        })
        .await;

    let busy = super::session_store::with_store(dir.path(), |store| Ok(store.is_busy("chat-40")))
        .expect("store");
    assert!(busy, "should be busy after matching received");

    // Attempt to clear with a stale-workspace processed event — must be ignored.
    subscriber
        .handle(&DomainEvent::ChannelMessageProcessed {
            channel: "telegram".into(),
            message_id: "m1".into(),
            sender: "alice".into(),
            reply_target: "chat-40".into(),
            content: "hi".into(),
            thread_ts: None,
            response: "done".into(),
            elapsed_ms: 50,
            success: true,
            workspace_dir: stale.path().to_path_buf(),
        })
        .await;

    let busy = super::session_store::with_store(dir.path(), |store| Ok(store.is_busy("chat-40")))
        .expect("store");
    assert!(busy, "stale processed must not clear busy state");
}
