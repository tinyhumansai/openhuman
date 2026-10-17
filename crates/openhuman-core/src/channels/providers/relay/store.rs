//! The relayed conversation on disk: the thread, the inbound message, the
//! reply, and the history a turn is seeded from.
//!
//! Everything is written to the workspace of the config the caller's context
//! loads, so a SaaS user's relayed thread lives with their web threads. The
//! process-wide channel persistence subscriber is told to leave these turns
//! alone (`threads::store::claim_channel_turn`).

use crate::core::runtime::DispatchCtx;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

type ThreadLocks = Mutex<HashMap<(PathBuf, String), Arc<tokio::sync::Mutex<()>>>>;

use chrono::Utc;
use serde_json::json;
use tinychannels_bus::context::ChatMessage;

use super::params::RelayInboundParams;
use crate::threads::store::blocking as conversations;
use crate::threads::store::{ConversationMessage, CreateConversationThread};

/// Whether an inbound message was new to its thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    New,
    /// Already on the thread: a gateway retry. No turn runs for it.
    Duplicate,
}

/// The stored id of the inbound message.
pub fn inbound_message_id(message_id: &str) -> String {
    format!("user:{message_id}")
}

/// The stored id of the reply to it.
pub fn reply_message_id(message_id: &str) -> String {
    format!("assistant:{message_id}")
}

/// One async lock per `(workspace, thread)`, so the duplicate check and the
/// append in [`record_inbound`] are atomic against a concurrent gateway retry.
fn thread_lock(workspace_dir: &Path, thread_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<ThreadLocks> = OnceLock::new();
    let mut map = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(
        map.entry((workspace_dir.to_path_buf(), thread_id.to_string()))
            .or_default(),
    )
}

/// Create the thread if needed and append the inbound message, unless it is
/// already there.
pub async fn record_inbound(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
    params: &RelayInboundParams,
    thread_id: &str,
) -> Result<Recorded, String> {
    let now = Utc::now().to_rfc3339();
    conversations::ensure_thread(
        ctx.clone(),
        workspace_dir.to_path_buf(),
        CreateConversationThread {
            id: thread_id.to_string(),
            title: params.thread_title(),
            created_at: now.clone(),
            parent_thread_id: None,
            labels: Some(vec!["general".to_string()]),
            personality_id: None,
            working_dir: None,
        },
    )
    .await?;

    let id = inbound_message_id(&params.message_id);
    let lock = thread_lock(workspace_dir, thread_id);
    let _guard = lock.lock().await;
    let existing = conversations::get_messages(
        ctx.clone(),
        workspace_dir.to_path_buf(),
        thread_id.to_string(),
    )
    .await?;
    if existing.iter().any(|message| message.id == id) {
        return Ok(Recorded::Duplicate);
    }
    conversations::append_message(
        ctx.clone(),
        workspace_dir.to_path_buf(),
        thread_id.to_string(),
        ConversationMessage {
            id,
            content: params.text.clone(),
            message_type: "text".to_string(),
            extra_metadata: json!({
                "scope": "channel",
                "channel": params.channel,
                "channelSender": params.sender_id,
                "channelSenderName": params.sender_name,
                "replyTarget": params.chat_id,
                "sourceEvent": "channel_relay_inbound",
                "sourceMessageId": params.message_id,
            }),
            sender: "user".to_string(),
            created_at: now,
        },
    )
    .await?;
    Ok(Recorded::New)
}

/// Append the turn's reply: every text the relay sink sent, in order.
pub async fn record_reply(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
    params: &RelayInboundParams,
    thread_id: &str,
    replies: &[String],
) -> Result<(), String> {
    if replies.is_empty() {
        return Ok(());
    }
    conversations::append_message(
        ctx.clone(),
        workspace_dir.to_path_buf(),
        thread_id.to_string(),
        ConversationMessage {
            id: reply_message_id(&params.message_id),
            content: replies.join("\n\n"),
            message_type: "text".to_string(),
            extra_metadata: json!({
                "scope": "channel",
                "channel": params.channel,
                "replyTarget": params.chat_id,
                "sourceEvent": "channel_relay_reply",
                "sourceMessageId": params.message_id,
            }),
            sender: "assistant".to_string(),
            created_at: Utc::now().to_rfc3339(),
        },
    )
    .await?;
    Ok(())
}

/// The thread's earlier turns as the channel pipeline's per-chat history:
/// user and assistant rows before `message_id`, the most recent `limit`.
pub async fn prior_turns(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
    thread_id: &str,
    message_id: &str,
    limit: usize,
) -> Result<Vec<ChatMessage>, String> {
    let current = inbound_message_id(message_id);
    let messages = conversations::get_messages(
        ctx.clone(),
        workspace_dir.to_path_buf(),
        thread_id.to_string(),
    )
    .await?;
    let rows = to_history(&messages, &current);
    let skip = rows.len().saturating_sub(limit);
    Ok(rows.into_iter().skip(skip).collect())
}

/// `messages` up to (not including) the one with id `current`, as history.
fn to_history(messages: &[ConversationMessage], current: &str) -> Vec<ChatMessage> {
    messages
        .iter()
        .take_while(|message| message.id != current)
        .filter_map(|message| match message.sender.as_str() {
            "user" => Some(ChatMessage::user(&message.content)),
            "assistant" => Some(ChatMessage::assistant(&message.content)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
