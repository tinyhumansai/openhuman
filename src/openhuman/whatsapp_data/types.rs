//! Normalized WhatsApp data structures — local-only, never transmitted externally.
//!
//! These types represent the structured data extracted from WhatsApp Web via CDP and
//! persisted in a local SQLite database. All data remains local; nothing is sent to
//! any remote service.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A WhatsApp chat (conversation) record stored locally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppChat {
    /// JID e.g. "123456@c.us" or "group@g.us"
    pub chat_id: String,
    /// Human-readable display name from WhatsApp contacts/group metadata.
    pub display_name: String,
    /// True if this chat is a group conversation.
    pub is_group: bool,
    /// The connected WhatsApp account identifier.
    pub account_id: String,
    /// Unix timestamp (seconds) of the most recent message stored.
    pub last_message_ts: i64,
    /// Number of messages stored for this chat.
    pub message_count: u32,
    /// Unix timestamp (seconds) when this record was last updated.
    pub updated_at: i64,
}

/// A single WhatsApp message record stored locally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppMessage {
    /// WhatsApp message identifier (compound or bare form).
    pub message_id: String,
    /// JID of the chat this message belongs to.
    pub chat_id: String,
    /// Display name of the sender (stored as-is from WhatsApp).
    pub sender: String,
    /// JID of the sender, when available from IDB metadata.
    pub sender_jid: Option<String>,
    /// True if the message was sent by the account owner.
    pub from_me: bool,
    /// Decrypted message body text.
    pub body: String,
    /// Unix timestamp (seconds) of the message.
    pub timestamp: i64,
    /// WhatsApp message type (e.g. "chat", "image", "sticker").
    pub message_type: Option<String>,
    /// The connected WhatsApp account identifier.
    pub account_id: String,
    /// Data source: "cdp-dom" or "cdp-indexeddb".
    pub source: String,
}

/// Metadata about a single chat in an ingest payload.
#[derive(Debug, Deserialize)]
pub struct ChatMeta {
    /// Display name for the chat, if available.
    pub name: Option<String>,
}

/// A single message entry in an ingest payload.
#[derive(Debug, Deserialize)]
pub struct IngestMessage {
    pub message_id: String,
    pub chat_id: String,
    pub sender: Option<String>,
    pub sender_jid: Option<String>,
    pub from_me: Option<bool>,
    pub body: Option<String>,
    pub timestamp: Option<i64>,
    pub message_type: Option<String>,
    pub source: Option<String>,
}

/// Request payload for `openhuman.whatsapp_data_ingest`.
#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    /// The WhatsApp account identifier (usually the phone JID).
    pub account_id: String,
    /// Map of chat JID → chat metadata (display name, etc.).
    pub chats: HashMap<String, ChatMeta>,
    /// Messages to upsert into the local store.
    pub messages: Vec<IngestMessage>,
}

/// Summary result returned after an ingest operation.
#[derive(Debug, Serialize)]
pub struct IngestResult {
    pub chats_upserted: usize,
    pub messages_upserted: usize,
    pub messages_pruned: u64,
}

/// Request payload for `openhuman.whatsapp_data_list_chats`.
#[derive(Debug, Deserialize)]
pub struct ListChatsRequest {
    /// Optional filter by account. When absent, all accounts are returned.
    pub account_id: Option<String>,
    /// Maximum number of results (default: 50).
    pub limit: Option<u32>,
    /// Pagination offset (default: 0).
    pub offset: Option<u32>,
}

/// Request payload for `openhuman.whatsapp_data_list_messages`.
#[derive(Debug, Deserialize)]
pub struct ListMessagesRequest {
    /// JID of the chat to retrieve messages for.
    pub chat_id: String,
    /// Optional filter by account. When absent, all accounts are searched.
    pub account_id: Option<String>,
    /// Only return messages at or after this Unix timestamp (seconds).
    pub since_ts: Option<i64>,
    /// Only return messages at or before this Unix timestamp (seconds).
    pub until_ts: Option<i64>,
    /// Maximum number of results (default: 100).
    pub limit: Option<u32>,
    /// Pagination offset (default: 0).
    pub offset: Option<u32>,
}

/// Request payload for `openhuman.whatsapp_data_search_messages`.
#[derive(Debug, Deserialize)]
pub struct SearchMessagesRequest {
    /// Full-text search query matched against message bodies (case-insensitive LIKE).
    pub query: String,
    /// Optional filter by chat JID.
    pub chat_id: Option<String>,
    /// Optional filter by account. When absent, all accounts are searched.
    pub account_id: Option<String>,
    /// Maximum number of results (default: 20).
    pub limit: Option<u32>,
}
