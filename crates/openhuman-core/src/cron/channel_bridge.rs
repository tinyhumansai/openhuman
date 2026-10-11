//! Cron's handle on the channel runtime, for jobs bound to a channel
//! conversation.
//!
//! Cron runs outside the channel runtime, but a channel-origin job needs two
//! things that live in it: the per-chat conversation history (read for the
//! context tail, appended to after a reminder is delivered) and the live
//! channel instances (to send through). The runtime registers both here when
//! it starts; nothing registers in a core with no channels, and every accessor
//! then answers "unavailable" instead of failing.

use crate::channels::traits::{Channel, SendMessage};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tinyagents_session::transcript::TranscriptMessage;

/// Same shape as `channels::context::ConversationHistoryMap`, spelled out so
/// cron does not depend on the feature-gated `channels` module.
pub type ChannelHistories = Arc<Mutex<HashMap<String, Vec<TranscriptMessage>>>>;

/// How many messages a chat's history keeps; matches the channel processor's
/// own trimming so a delivered reminder never grows it past that bound.
pub const MAX_HISTORY_MESSAGES: usize = 40;

#[derive(Clone)]
struct Bridge {
    histories: ChannelHistories,
    channels: Arc<HashMap<String, Arc<dyn Channel>>>,
}

fn slot() -> &'static RwLock<Option<Bridge>> {
    static SLOT: RwLock<Option<Bridge>> = RwLock::new(None);
    &SLOT
}

fn current() -> Option<Bridge> {
    slot().read().ok().and_then(|guard| guard.clone())
}

/// Register the running channel runtime's histories and channels. The latest
/// registration wins (the runtime restarts on logout/login).
pub fn register_channel_bridge(
    histories: ChannelHistories,
    channels: Arc<HashMap<String, Arc<dyn Channel>>>,
    max_history: usize,
) {
    MAX_HISTORY.store(max_history.max(1), std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut guard) = slot().write() {
        *guard = Some(Bridge {
            histories,
            channels,
        });
        tracing::debug!(max_history, "[cron] channel bridge registered");
    }
}

static MAX_HISTORY: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(MAX_HISTORY_MESSAGES);

/// The `(role, text)` messages of one chat's history, oldest first. Empty when
/// no channel runtime is registered or the chat has no history.
pub fn history_messages(history_key: &str) -> Vec<(String, String)> {
    let Some(bridge) = current() else {
        tracing::debug!("[cron] channel history requested but no channel bridge registered");
        return Vec::new();
    };
    let histories = bridge.histories.lock().unwrap_or_else(|e| e.into_inner());
    histories
        .get(history_key)
        .map(|turns| {
            turns
                .iter()
                .map(|m| (m.role.clone(), m.content.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Append an assistant message to one chat's history and trim it the way the
/// channel processor does, so the chat's next turn sees what was delivered.
/// Returns `false` when no channel runtime is registered.
pub fn append_assistant_message(history_key: &str, text: &str) -> bool {
    let Some(bridge) = current() else {
        tracing::debug!("[cron] channel history append skipped: no channel bridge registered");
        return false;
    };
    let max = MAX_HISTORY.load(std::sync::atomic::Ordering::Relaxed);
    let mut histories = bridge.histories.lock().unwrap_or_else(|e| e.into_inner());
    let turns = histories.entry(history_key.to_string()).or_default();
    turns.push(TranscriptMessage::assistant(text));
    while turns.len() > max {
        turns.remove(0);
    }
    tracing::debug!(
        history_len = turns.len(),
        "[cron] appended delivered reply to channel history"
    );
    true
}

/// Send `text` to `reply_target` on the named channel through the registered
/// runtime. Errors when the runtime or channel is not available, or the send
/// fails.
pub async fn send_to_channel(
    channel: &str,
    reply_target: &str,
    thread_id: Option<&str>,
    text: &str,
    idempotency_key: &str,
) -> Result<(), String> {
    let Some(bridge) = current() else {
        return Err("channel runtime is not running".to_string());
    };
    let key = channel.trim().to_ascii_lowercase();
    let Some(ch) = bridge.channels.get(&key) else {
        return Err(format!("channel '{key}' is not available"));
    };
    // Run-specific key: a retry after an ambiguous send must not duplicate the
    // reminder, while identical reminders from different runs must not collide
    // (the content-derived default key would reuse one key for both).
    let mut message = SendMessage::new(text, reply_target).in_thread(thread_id.map(str::to_string));
    message.idempotency_key = Some(idempotency_key.to_string());
    ch.send(&message)
        .await
        .map_err(|e| format!("channel send failed: {e}"))
}

#[cfg(test)]
#[path = "channel_bridge_tests.rs"]
mod tests;
