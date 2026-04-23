//! Persistence for provider assistive surfaces.
//!
//! Follow-up work will add a SQLite-backed store for normalized provider
//! events, respond queue state, and local drafts.

use std::sync::{Mutex, OnceLock};

use crate::openhuman::provider_surfaces::types::{ProviderEvent, RespondQueueItem};

/// Soft cap on the in-memory respond queue to bound growth under provider
/// firehose volume before the SQLite-backed store lands. The queue is
/// prepend-ordered, so oldest entries are dropped from the tail.
const MAX_QUEUE_ITEMS: usize = 500;

static RESPOND_QUEUE: OnceLock<Mutex<Vec<RespondQueueItem>>> = OnceLock::new();

fn queue() -> &'static Mutex<Vec<RespondQueueItem>> {
    RESPOND_QUEUE.get_or_init(|| Mutex::new(Vec::new()))
}

fn queue_lock() -> std::sync::MutexGuard<'static, Vec<RespondQueueItem>> {
    queue()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn queue_item_id(event: &ProviderEvent) -> String {
    format!(
        "{}:{}:{}:{}",
        event.provider, event.account_id, event.event_kind, event.entity_id
    )
}

pub fn upsert_queue_item(event: ProviderEvent) -> RespondQueueItem {
    let item = RespondQueueItem {
        id: queue_item_id(&event),
        provider: event.provider,
        account_id: event.account_id,
        event_kind: event.event_kind,
        entity_id: event.entity_id,
        thread_id: event.thread_id,
        title: event.title,
        snippet: event.snippet,
        sender_name: event.sender_name,
        sender_handle: event.sender_handle,
        timestamp: event.timestamp,
        deep_link: event.deep_link,
        requires_attention: event.requires_attention,
        status: "pending".to_string(),
    };

    let mut queue = queue_lock();
    if let Some(existing_idx) = queue.iter().position(|entry| entry.id == item.id) {
        queue.remove(existing_idx);
    }
    queue.insert(0, item.clone());
    if queue.len() > MAX_QUEUE_ITEMS {
        queue.truncate(MAX_QUEUE_ITEMS);
    }
    item
}

pub fn list_queue_items() -> Vec<RespondQueueItem> {
    queue_lock().clone()
}

#[cfg(test)]
pub fn clear_queue() {
    queue_lock().clear();
}
