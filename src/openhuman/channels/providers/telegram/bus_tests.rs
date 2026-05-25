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
        })
        .await;

    let busy = super::session_store::with_store(dir.path(), |store| Ok(store.is_busy("chat-99")))
        .expect("store");
    assert!(!busy);
}
