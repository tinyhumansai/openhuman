use super::*;

#[test]
#[allow(
    deprecated,
    reason = "The route contract still requires the v21 ChannelMessage projection"
)]
fn route_compat_message_preserves_legacy_addressing_and_sender_metadata() {
    let mut message = inbound(
        "message-1".to_string(),
        "sender-1".to_string(),
        "conversation-1".to_string(),
        "/status".to_string(),
        "telegram".to_string(),
        Some("thread-1".into()),
    );
    message.sender.name = Some("Alice".into());

    let legacy = super::super::route_compat_message(&message);
    assert_eq!(legacy.id, "message-1");
    assert_eq!(legacy.sender, "sender-1");
    assert_eq!(legacy.reply_target, "conversation-1");
    assert_eq!(legacy.content, "/status");
    assert_eq!(legacy.channel, "telegram");
    assert_eq!(legacy.timestamp, 0);
    assert_eq!(legacy.thread_ts.as_deref(), Some("thread-1"));
    assert_eq!(legacy.sender_name.as_deref(), Some("Alice"));
}
