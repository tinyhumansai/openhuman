use super::*;

#[test]
fn cli_channel_name() {
    assert_eq!(CliChannel::new().name(), "cli");
}

#[tokio::test]
async fn cli_channel_send_does_not_panic() {
    let ch = CliChannel::new();
    let result = ch
        .send(&SendMessage {
            content: "hello".into(),
            recipient: "user".into(),
            subject: None,
            thread_ts: None,
            idempotency_key: None,
        })
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn cli_channel_send_empty_message() {
    let ch = CliChannel::new();
    let result = ch
        .send(&SendMessage {
            content: String::new(),
            recipient: String::new(),
            subject: None,
            thread_ts: None,
            idempotency_key: None,
        })
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn cli_channel_health_check() {
    let ch = CliChannel::new();
    assert!(ch.health_check().await);
}

#[test]
fn channel_message_struct() {
    let msg = ChannelMessage {
        id: "test-id".into(),
        sender: "user".into(),
        reply_target: "user".into(),
        content: "hello".into(),
        channel: "cli".into(),
        timestamp: 1_234_567_890,
        thread_ts: None,
    };
    assert_eq!(msg.id, "test-id");
    assert_eq!(msg.sender, "user");
    assert_eq!(msg.reply_target, "user");
    assert_eq!(msg.content, "hello");
    assert_eq!(msg.channel, "cli");
    assert_eq!(msg.timestamp, 1_234_567_890);
}

#[test]
fn channel_message_clone() {
    let msg = ChannelMessage {
        id: "id".into(),
        sender: "s".into(),
        reply_target: "s".into(),
        content: "c".into(),
        channel: "ch".into(),
        timestamp: 0,
        thread_ts: None,
    };
    let cloned = msg.clone();
    assert_eq!(cloned.id, msg.id);
    assert_eq!(cloned.content, msg.content);
}
