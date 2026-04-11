use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;

use crate::openhuman::channels::context::conversation_history_key;
use crate::openhuman::channels::traits::ChannelMessage;
use crate::openhuman::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};

use super::{
    append_message, ensure_thread, get_messages, ConversationMessage, CreateConversationThread,
};

static CONVERSATION_PERSISTENCE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

const LOG_PREFIX: &str = "[memory:conversations:bus]";

/// Register the long-lived channel conversation persistence subscriber.
///
/// This bridges typed channel events onto the workspace-backed JSONL
/// conversation store so non-web channels persist alongside UI threads.
pub fn register_conversation_persistence_subscriber(workspace_dir: PathBuf) {
    if CONVERSATION_PERSISTENCE_HANDLE.get().is_some() {
        return;
    }

    match crate::openhuman::event_bus::subscribe_global(Arc::new(
        ConversationPersistenceSubscriber::new(workspace_dir),
    )) {
        Some(handle) => {
            let _ = CONVERSATION_PERSISTENCE_HANDLE.set(handle);
        }
        None => {
            log::warn!(
                "{LOG_PREFIX} failed to register conversation persistence subscriber — bus not initialized"
            );
        }
    }
}

pub struct ConversationPersistenceSubscriber {
    workspace_dir: PathBuf,
}

impl ConversationPersistenceSubscriber {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl EventHandler for ConversationPersistenceSubscriber {
    fn name(&self) -> &str {
        "memory::conversations::persistence"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["channel"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::ChannelMessageReceived {
                channel,
                message_id,
                sender,
                reply_target,
                content,
                thread_ts,
            } => {
                if let Err(error) = persist_channel_turn(
                    &self.workspace_dir,
                    ChannelTurnDescriptor {
                        channel,
                        message_id,
                        sender,
                        reply_target,
                        thread_ts: thread_ts.as_deref(),
                        content,
                        role: "user",
                        success: None,
                        elapsed_ms: None,
                        source: "channel_received",
                    },
                ) {
                    log::warn!(
                        "{LOG_PREFIX} failed to persist inbound channel message channel={} message_id={} error={}",
                        channel,
                        message_id,
                        error
                    );
                }
            }
            DomainEvent::ChannelMessageProcessed {
                channel,
                message_id,
                sender,
                reply_target,
                thread_ts,
                response,
                elapsed_ms,
                success,
                ..
            } => {
                if let Err(error) = persist_channel_turn(
                    &self.workspace_dir,
                    ChannelTurnDescriptor {
                        channel,
                        message_id,
                        sender,
                        reply_target,
                        thread_ts: thread_ts.as_deref(),
                        content: response,
                        role: "assistant",
                        success: Some(*success),
                        elapsed_ms: Some(*elapsed_ms),
                        source: "channel_processed",
                    },
                ) {
                    log::warn!(
                        "{LOG_PREFIX} failed to persist processed channel message channel={} message_id={} error={}",
                        channel,
                        message_id,
                        error
                    );
                }
            }
            _ => {}
        }
    }
}

struct ChannelTurnDescriptor<'a> {
    channel: &'a str,
    message_id: &'a str,
    sender: &'a str,
    reply_target: &'a str,
    thread_ts: Option<&'a str>,
    content: &'a str,
    role: &'a str,
    success: Option<bool>,
    elapsed_ms: Option<u64>,
    source: &'a str,
}

fn persist_channel_turn(
    workspace_dir: &PathBuf,
    descriptor: ChannelTurnDescriptor<'_>,
) -> Result<(), String> {
    let thread_id = persisted_channel_thread_id(
        descriptor.channel,
        descriptor.sender,
        descriptor.reply_target,
        descriptor.thread_ts,
    );
    let title = channel_thread_title(
        descriptor.channel,
        descriptor.sender,
        descriptor.reply_target,
        descriptor.thread_ts,
    );
    let created_at = Utc::now().to_rfc3339();

    ensure_thread(
        workspace_dir.clone(),
        CreateConversationThread {
            id: thread_id.clone(),
            title,
            created_at: created_at.clone(),
        },
    )?;

    let persisted_message_id = format!("{}:{}", descriptor.role, descriptor.message_id);
    if get_messages(workspace_dir.clone(), &thread_id)?
        .iter()
        .any(|message| message.id == persisted_message_id)
    {
        log::debug!(
            "{LOG_PREFIX} skipping duplicate persisted turn thread_id={} message_id={}",
            thread_id,
            persisted_message_id
        );
        return Ok(());
    }

    append_message(
        workspace_dir.clone(),
        &thread_id,
        ConversationMessage {
            id: persisted_message_id.clone(),
            content: descriptor.content.to_string(),
            message_type: "text".to_string(),
            extra_metadata: json!({
                "scope": "channel",
                "channel": descriptor.channel,
                "channelSender": descriptor.sender,
                "replyTarget": descriptor.reply_target,
                "threadTs": descriptor.thread_ts,
                "sourceEvent": descriptor.source,
                "success": descriptor.success,
                "elapsedMs": descriptor.elapsed_ms,
                "sourceMessageId": descriptor.message_id,
            }),
            sender: descriptor.role.to_string(),
            created_at,
        },
    )?;

    log::debug!(
        "{LOG_PREFIX} persisted channel turn thread_id={} message_id={} role={}",
        thread_id,
        persisted_message_id,
        descriptor.role
    );
    Ok(())
}

fn persisted_channel_thread_id(
    channel: &str,
    sender: &str,
    reply_target: &str,
    thread_ts: Option<&str>,
) -> String {
    let key = conversation_history_key(&ChannelMessage {
        id: String::new(),
        sender: sender.to_string(),
        reply_target: reply_target.to_string(),
        content: String::new(),
        channel: channel.to_string(),
        timestamp: 0,
        thread_ts: thread_ts.map(ToOwned::to_owned),
    });
    format!("channel:{key}")
}

fn channel_thread_title(
    channel: &str,
    sender: &str,
    reply_target: &str,
    thread_ts: Option<&str>,
) -> String {
    match thread_ts.and_then(non_empty_trimmed) {
        Some(thread_ts) if channel != "telegram" => {
            format!("{channel} · {sender} · {reply_target} · thread {thread_ts}")
        }
        _ => format!("{channel} · {sender} · {reply_target}"),
    }
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn persists_inbound_and_processed_turns_into_workspace_thread() {
        let temp = TempDir::new().expect("tempdir");
        let subscriber = ConversationPersistenceSubscriber::new(temp.path().to_path_buf());

        subscriber
            .handle(&DomainEvent::ChannelMessageReceived {
                channel: "slack".into(),
                message_id: "m1".into(),
                sender: "alice".into(),
                reply_target: "general".into(),
                content: "hello".into(),
                thread_ts: Some("thread-1".into()),
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
                elapsed_ms: 42,
                success: true,
            })
            .await;

        let threads = super::super::list_threads(temp.path().to_path_buf()).expect("threads");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "channel:slack_alice_general_thread:thread-1");

        let messages = super::super::get_messages(temp.path().to_path_buf(), &threads[0].id)
            .expect("messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].id, "user:m1");
        assert_eq!(messages[0].sender, "user");
        assert_eq!(messages[1].id, "assistant:m1");
        assert_eq!(messages[1].sender, "assistant");
        assert_eq!(messages[1].extra_metadata["elapsedMs"], 42);
        assert_eq!(messages[1].extra_metadata["success"], true);
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
            })
            .await;

        let threads = super::super::list_threads(temp.path().to_path_buf()).expect("threads");
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
        };

        subscriber.handle(&event).await;
        subscriber.handle(&event).await;

        let messages =
            super::super::get_messages(temp.path().to_path_buf(), "channel:discord_alice_room-1")
                .expect("messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, "user:m1");
    }
}
