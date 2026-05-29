//! Event-bus subscriber that mirrors inbound channel messages into the
//! workspace-backed conversation store, so non-web channels (Slack, Telegram,
//! etc.) persist alongside UI-driven threads.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;

use crate::core::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};
use crate::openhuman::channels::context::conversation_history_key;
use crate::openhuman::channels::traits::ChannelMessage;

use super::{
    append_message, ensure_thread, get_messages, ConversationMessage, CreateConversationThread,
};

static CONVERSATION_PERSISTENCE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();
static CONVERSATION_PERSISTENCE_WORKSPACE: OnceLock<Arc<RwLock<PathBuf>>> = OnceLock::new();

const LOG_PREFIX: &str = "[memory:conversations:bus]";

/// Register the long-lived channel conversation persistence subscriber.
///
/// This bridges typed channel events onto the workspace-backed JSONL
/// conversation store so non-web channels persist alongside UI threads.
pub fn register_conversation_persistence_subscriber(workspace_dir: PathBuf) {
    let workspace = CONVERSATION_PERSISTENCE_WORKSPACE
        .get_or_init(|| Arc::new(RwLock::new(workspace_dir.clone())));
    match workspace.write() {
        Ok(mut guard) => {
            *guard = workspace_dir;
        }
        Err(error) => {
            log::warn!("{LOG_PREFIX} failed to update workspace binding: {error}");
        }
    }

    if CONVERSATION_PERSISTENCE_HANDLE.get().is_some() {
        return;
    }

    match crate::core::event_bus::subscribe_global(Arc::new(
        ConversationPersistenceSubscriber::new_shared(Arc::clone(workspace)),
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
    workspace_dir: Arc<RwLock<PathBuf>>,
}

impl ConversationPersistenceSubscriber {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            workspace_dir: Arc::new(RwLock::new(workspace_dir)),
        }
    }

    fn new_shared(workspace_dir: Arc<RwLock<PathBuf>>) -> Self {
        Self { workspace_dir }
    }

    fn workspace_dir_snapshot(&self) -> Result<PathBuf, String> {
        self.workspace_dir
            .read()
            .map(|guard| guard.clone())
            .map_err(|error| format!("workspace binding poisoned: {error}"))
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
                workspace_dir,
            } => {
                let my_workspace = match self.workspace_dir_snapshot() {
                    Ok(d) => d,
                    Err(error) => {
                        log::warn!("{LOG_PREFIX} failed to resolve workspace: {error}");
                        return;
                    }
                };
                if *workspace_dir != my_workspace {
                    log::debug!(
                        "{LOG_PREFIX} dropping stale-workspace event \
                         event_ws={} self_ws={}",
                        workspace_dir.display(),
                        my_workspace.display()
                    );
                    return;
                }
                if let Err(error) = persist_channel_turn(
                    &my_workspace,
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
                workspace_dir,
                ..
            } => {
                let my_workspace = match self.workspace_dir_snapshot() {
                    Ok(d) => d,
                    Err(error) => {
                        log::warn!("{LOG_PREFIX} failed to resolve workspace: {error}");
                        return;
                    }
                };
                if *workspace_dir != my_workspace {
                    log::debug!(
                        "{LOG_PREFIX} dropping stale-workspace event \
                         event_ws={} self_ws={}",
                        workspace_dir.display(),
                        my_workspace.display()
                    );
                    return;
                }
                if let Err(error) = persist_channel_turn(
                    &my_workspace,
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
    workspace_dir: &Path,
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
        workspace_dir.to_path_buf(),
        CreateConversationThread {
            id: thread_id.clone(),
            title,
            created_at: created_at.clone(),
            parent_thread_id: None,
            labels: Some(vec!["work".to_string()]),
            personality_id: None,
        },
    )?;

    let persisted_message_id = format!("{}:{}", descriptor.role, descriptor.message_id);
    if get_messages(workspace_dir.to_path_buf(), &thread_id)?
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
        workspace_dir.to_path_buf(),
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

        subscriber
            .handle(&DomainEvent::ChannelMessageReceived {
                channel: "slack".into(),
                message_id: "m1".into(),
                sender: "alice".into(),
                reply_target: "general".into(),
                content: "hello".into(),
                thread_ts: Some("thread-1".into()),
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
                elapsed_ms: 42,
                success: true,
                workspace_dir: temp.path().to_path_buf(),
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
                workspace_dir: temp.path().to_path_buf(),
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
            workspace_dir: temp.path().to_path_buf(),
        };

        subscriber.handle(&event).await;
        subscriber.handle(&event).await;

        let messages =
            super::super::get_messages(temp.path().to_path_buf(), "channel:discord_alice_room-1")
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
                workspace_dir: temp.path().to_path_buf(),
            })
            .await;

        let messages =
            super::super::get_messages(temp.path().to_path_buf(), "channel:slack_bob_dev")
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
                workspace_dir: stale.path().to_path_buf(),
            })
            .await;

        // No thread should have been created in temp (the subscriber's workspace).
        let threads = super::super::list_threads(temp.path().to_path_buf()).expect("threads");
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
                elapsed_ms: 10,
                success: true,
                workspace_dir: temp.path().to_path_buf(),
            })
            .await;

        let messages =
            super::super::get_messages(temp.path().to_path_buf(), "channel:slack_alice_general")
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
                elapsed_ms: 10,
                success: true,
                workspace_dir: stale.path().to_path_buf(),
            })
            .await;

        let messages =
            super::super::get_messages(temp.path().to_path_buf(), "channel:slack_alice_general")
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
                elapsed_ms: 10,
                success: true,
                workspace_dir: workspace_a.path().to_path_buf(),
            })
            .await;

        let messages = super::super::get_messages(
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
                    workspace_dir: stale.path().to_path_buf(),
                })
                .await;
        }

        let threads = super::super::list_threads(temp.path().to_path_buf()).expect("threads");
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
                workspace_dir: temp.path().to_path_buf(),
            })
            .await;

        let messages =
            super::super::get_messages(temp.path().to_path_buf(), "channel:slack_alice_general")
                .expect("messages");
        assert_eq!(
            messages.len(),
            1,
            "only the valid event should be persisted"
        );
        assert_eq!(messages[0].id, "user:m1");
        assert_eq!(messages[0].content, "valid");
    }
}
