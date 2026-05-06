//! Business logic for WhatsApp data ingestion and retrieval.
//!
//! All operations take a `&WhatsAppDataStore` so callers control the store
//! lifetime (shared `Arc` at runtime, fresh instance in tests).

use anyhow::Result;

use crate::openhuman::whatsapp_data::{
    store::WhatsAppDataStore,
    types::{
        IngestRequest, IngestResult, ListChatsRequest, ListMessagesRequest, SearchMessagesRequest,
        WhatsAppChat, WhatsAppMessage,
    },
};

/// Number of seconds in 90 days — the auto-prune horizon.
const PRUNE_HORIZON_SECS: i64 = 90 * 24 * 60 * 60;

/// Ingest a scanner snapshot: upsert chats and messages, then prune messages
/// older than 90 days.
///
/// Returns counts for observability / logging at the RPC layer.
pub fn ingest(store: &WhatsAppDataStore, req: IngestRequest) -> Result<IngestResult> {
    log::debug!(
        "[whatsapp_data] ingest start chats={} messages={} (account redacted)",
        req.chats.len(),
        req.messages.len()
    );

    let chats_upserted = store.upsert_chats(&req.account_id, &req.chats)?;
    let messages_upserted = store.upsert_messages(&req.account_id, &req.messages)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let cutoff_ts = now - PRUNE_HORIZON_SECS;
    let messages_pruned = store.prune_old_messages(cutoff_ts)?;

    let result = IngestResult {
        chats_upserted,
        messages_upserted,
        messages_pruned,
    };
    log::debug!(
        "[whatsapp_data] ingest done chats_upserted={} messages_upserted={} pruned={} (account redacted)",
        result.chats_upserted,
        result.messages_upserted,
        result.messages_pruned
    );
    Ok(result)
}

/// Return chats from the local store, optionally filtered by account.
pub fn list_chats(store: &WhatsAppDataStore, req: ListChatsRequest) -> Result<Vec<WhatsAppChat>> {
    log::debug!(
        "[whatsapp_data] list_chats has_account={} limit={:?} offset={:?}",
        req.account_id.is_some(),
        req.limit,
        req.offset
    );
    store.list_chats(&req)
}

/// Return messages for a chat, with optional time range and pagination.
pub fn list_messages(
    store: &WhatsAppDataStore,
    req: ListMessagesRequest,
) -> Result<Vec<WhatsAppMessage>> {
    log::debug!(
        "[whatsapp_data] list_messages has_account={} (chat/account redacted)",
        req.account_id.is_some()
    );
    store.list_messages(&req)
}

/// Full-text search over message bodies.
pub fn search_messages(
    store: &WhatsAppDataStore,
    req: SearchMessagesRequest,
) -> Result<Vec<WhatsAppMessage>> {
    log::debug!(
        "[whatsapp_data] search_messages has_account={} has_chat={} (query/identifiers redacted)",
        req.account_id.is_some(),
        req.chat_id.is_some()
    );
    store.search_messages(&req)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::whatsapp_data::types::{ChatMeta, IngestMessage};
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn make_store() -> (WhatsAppDataStore, tempfile::TempDir) {
        let tmp = tempdir().expect("tempdir");
        let store = WhatsAppDataStore::new(tmp.path()).expect("store");
        (store, tmp)
    }

    fn sample_request() -> IngestRequest {
        // Use a timestamp close to "now" so messages are not pruned by the
        // 90-day auto-prune horizon.  We derive it from the system clock
        // minus one hour so even on slow CI boxes the message is comfortably
        // within the retention window.
        let recent_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64 - 3600)
            .unwrap_or(1_750_000_000);
        let mut chats = HashMap::new();
        chats.insert(
            "alice@c.us".to_string(),
            ChatMeta {
                name: Some("Alice".to_string()),
            },
        );
        IngestRequest {
            account_id: "acct1".to_string(),
            chats,
            messages: vec![IngestMessage {
                message_id: "msg1".to_string(),
                chat_id: "alice@c.us".to_string(),
                sender: Some("Alice".to_string()),
                sender_jid: None,
                from_me: Some(false),
                body: Some("Hello!".to_string()),
                timestamp: Some(recent_ts),
                message_type: Some("chat".to_string()),
                source: Some("cdp-dom".to_string()),
            }],
        }
    }

    #[test]
    fn ingest_returns_correct_counts() {
        let (store, _tmp) = make_store();
        let result = ingest(&store, sample_request()).unwrap();
        assert_eq!(result.chats_upserted, 1);
        assert_eq!(result.messages_upserted, 1);
    }

    #[test]
    fn list_chats_after_ingest() {
        let (store, _tmp) = make_store();
        ingest(&store, sample_request()).unwrap();

        let chats = list_chats(
            &store,
            ListChatsRequest {
                account_id: None,
                limit: None,
                offset: None,
            },
        )
        .unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].chat_id, "alice@c.us");
    }

    #[test]
    fn list_messages_after_ingest() {
        let (store, _tmp) = make_store();
        ingest(&store, sample_request()).unwrap();

        let msgs = list_messages(
            &store,
            ListMessagesRequest {
                chat_id: "alice@c.us".to_string(),
                account_id: None,
                since_ts: None,
                until_ts: None,
                limit: None,
                offset: None,
            },
        )
        .unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "Hello!");
    }

    #[test]
    fn search_messages_after_ingest() {
        let (store, _tmp) = make_store();
        ingest(&store, sample_request()).unwrap();

        let results = search_messages(
            &store,
            SearchMessagesRequest {
                query: "Hello".to_string(),
                chat_id: None,
                account_id: None,
                limit: None,
            },
        )
        .unwrap();
        assert_eq!(results.len(), 1);
    }
}
