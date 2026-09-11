use super::*;
use tinychannels::controllers::ChannelThreadListResult;

#[test]
fn send_message_payload_preserves_thread_and_subject() {
    let message = SendMessage::with_subject("hello", "alice", "subject")
        .in_thread(Some("thread-1".to_string()));
    let payload = send_message_payload(message);
    assert_eq!(payload["content"], "hello");
    assert_eq!(payload["recipient"], "alice");
    assert_eq!(payload["subject"], "subject");
    assert_eq!(payload["thread_ts"], "thread-1");
}

#[test]
fn parse_or_raw_keeps_backend_payload_when_shape_is_unknown() {
    let payload = json!({"unexpected": true});
    let result: ChannelThreadListResult = parse_or_raw(payload.clone());
    assert_eq!(result.threads.len(), 0);
    assert_eq!(result.raw, Some(payload));
}

#[test]
fn disconnect_result_projects_restart_flag() {
    let payload =
        json!({"channel": "telegram", "restart_required": false, "memory_chunks_deleted": 2});
    let result = disconnect_result("telegram", ChannelAuthMode::BotToken, payload.clone());
    assert_eq!(result.channel, "telegram");
    assert_eq!(result.auth_mode, ChannelAuthMode::BotToken);
    assert!(result.disconnected);
    assert!(!result.restart_required);
    assert_eq!(result.memory_chunks_deleted, Some(2));
    assert_eq!(result.raw, Some(payload));
}
