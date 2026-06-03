//! Tests for the Telegram approval-surface subscriber.

use super::*;
use crate::core::event_bus::EventHandler;
use crate::openhuman::channels::traits::{ChannelMessage, SendMessage};
use crate::openhuman::channels::Channel;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;

/// Mock channel that records every outbound `send()` call. Used in place
/// of the real `TelegramChannel` so tests can assert what the subscriber
/// would have sent without needing the Telegram Bot API or a network.
struct RecordingChannel {
    name: String,
    sent: StdMutex<Vec<SendMessage>>,
}

impl RecordingChannel {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            sent: StdMutex::new(Vec::new()),
        }
    }

    fn drain(&self) -> Vec<SendMessage> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }
}

#[async_trait]
impl Channel for RecordingChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push(message.clone());
        Ok(())
    }

    async fn listen(&self, _tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        Ok(())
    }
}

fn subscriber_with_channel(channel: Arc<RecordingChannel>) -> TelegramApprovalSurfaceSubscriber {
    let mut map: HashMap<String, Arc<dyn Channel>> = HashMap::new();
    map.insert("telegram".to_string(), channel);
    TelegramApprovalSurfaceSubscriber::new(Arc::new(map))
}

fn approval_event(thread_id: Option<&str>, client_id: Option<&str>) -> DomainEvent {
    DomainEvent::ApprovalRequested {
        request_id: "req-1".to_string(),
        tool_name: "file_write".to_string(),
        action_summary: "Write notes/today.md (1.2 KiB)".to_string(),
        args_redacted: serde_json::json!({"path": "notes/today.md"}),
        thread_id: thread_id.map(str::to_string),
        client_id: client_id.map(str::to_string),
    }
}

fn inbound_event(sender: &str, reply_target: &str, thread_ts: Option<&str>) -> DomainEvent {
    DomainEvent::ChannelMessageReceived {
        channel: "telegram".to_string(),
        message_id: "m-1".to_string(),
        sender: sender.to_string(),
        reply_target: reply_target.to_string(),
        content: "hi".to_string(),
        thread_ts: thread_ts.map(str::to_string),
        workspace_dir: PathBuf::from("/tmp"),
    }
}

#[tokio::test]
async fn channel_message_received_records_reply_context() {
    let channel = Arc::new(RecordingChannel::new("telegram"));
    let sub = subscriber_with_channel(Arc::clone(&channel));

    sub.handle(&inbound_event("alice", "chat-42", None)).await;

    let ctx = sub
        .reply_context("telegram_alice_chat-42")
        .expect("reply context should be recorded for the inbound message");
    assert_eq!(ctx.reply_target, "chat-42");
    assert_eq!(ctx.thread_ts, None);
}

#[tokio::test]
async fn channel_message_received_ignores_non_telegram_channels() {
    let channel = Arc::new(RecordingChannel::new("telegram"));
    let sub = subscriber_with_channel(Arc::clone(&channel));

    let discord_event = DomainEvent::ChannelMessageReceived {
        channel: "discord".to_string(),
        message_id: "d-1".to_string(),
        sender: "bob".to_string(),
        reply_target: "channel-9".to_string(),
        content: "hi".to_string(),
        thread_ts: None,
        workspace_dir: PathBuf::from("/tmp"),
    };
    sub.handle(&discord_event).await;

    assert!(
        sub.reply_context("discord_bob_channel-9").is_none(),
        "subscriber must scope to telegram and not pollute its index with other channels"
    );
}

#[tokio::test]
async fn approval_request_sends_telegram_message_with_recorded_context() {
    let channel = Arc::new(RecordingChannel::new("telegram"));
    let sub = subscriber_with_channel(Arc::clone(&channel));
    sub.record_reply_context_for_test(
        "telegram_alice_chat-42",
        ReplyContext {
            reply_target: "chat-42".to_string(),
            thread_ts: Some("987654321".to_string()),
        },
    );

    sub.handle(&approval_event(
        Some("telegram_alice_chat-42"),
        Some("telegram"),
    ))
    .await;

    let sent = channel.drain();
    assert_eq!(sent.len(), 1, "exactly one message should have been sent");
    let msg = &sent[0];
    assert_eq!(msg.recipient, "chat-42");
    assert_eq!(msg.thread_ts.as_deref(), Some("987654321"));
    assert!(
        msg.content.contains("Approval needed"),
        "prompt body should advertise itself as an approval ask, got: {}",
        msg.content
    );
    assert!(
        msg.content.contains("file_write"),
        "prompt body should include the tool name"
    );
    assert!(
        msg.content.contains("yes") && msg.content.contains("no"),
        "prompt body must tell the user how to reply, got: {}",
        msg.content
    );
}

#[tokio::test]
async fn approval_request_without_recorded_context_does_not_send_anything() {
    let channel = Arc::new(RecordingChannel::new("telegram"));
    let sub = subscriber_with_channel(Arc::clone(&channel));

    // No record_reply_context_for_test call — subscriber has never seen
    // an inbound message for this thread.
    sub.handle(&approval_event(
        Some("telegram_alice_chat-42"),
        Some("telegram"),
    ))
    .await;

    assert!(
        channel.drain().is_empty(),
        "subscriber must not invent a reply target when no inbound context was recorded"
    );
}

#[tokio::test]
async fn approval_request_for_non_telegram_client_is_ignored() {
    let channel = Arc::new(RecordingChannel::new("telegram"));
    let sub = subscriber_with_channel(Arc::clone(&channel));
    // Seed a context so we'd otherwise be able to send.
    sub.record_reply_context_for_test(
        "telegram_alice_chat-42",
        ReplyContext {
            reply_target: "chat-42".to_string(),
            thread_ts: None,
        },
    );

    // Web-channel approval — must not be handled here.
    sub.handle(&approval_event(Some("thread-web-1"), Some("web")))
        .await;

    assert!(
        channel.drain().is_empty(),
        "subscriber must scope by client_id; web/discord/etc. approvals are handled by their own surfaces"
    );
}

#[tokio::test]
async fn approval_request_without_client_id_is_ignored() {
    let channel = Arc::new(RecordingChannel::new("telegram"));
    let sub = subscriber_with_channel(Arc::clone(&channel));
    sub.record_reply_context_for_test(
        "telegram_alice_chat-42",
        ReplyContext {
            reply_target: "chat-42".to_string(),
            thread_ts: None,
        },
    );

    sub.handle(&approval_event(Some("telegram_alice_chat-42"), None))
        .await;

    assert!(
        channel.drain().is_empty(),
        "background / triage approvals (no client_id) must not be routed to Telegram"
    );
}

#[test]
fn telegram_history_key_matches_format_in_dispatch_helper() {
    // Locking the key shape so the dispatch loop's
    // `conversation_history_key` and this module's `telegram_history_key`
    // stay byte-identical for Telegram — they MUST agree, or the
    // subscriber's lookup will miss every parked approval.
    assert_eq!(
        telegram_history_key("alice", "chat-42"),
        "telegram_alice_chat-42"
    );
}

#[test]
fn format_approval_prompt_includes_tool_action_and_reply_instructions() {
    let body = format_approval_prompt("git_operations", "git commit -m \"fix\"");
    assert!(body.contains("git_operations"));
    assert!(body.contains("git commit"));
    assert!(body.contains("yes"));
    assert!(body.contains("no"));
}
