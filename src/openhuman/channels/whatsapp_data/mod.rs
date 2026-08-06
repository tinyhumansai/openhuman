//! Structured WhatsApp Web data — shared DTOs + agent query tools.
//!
//! **Storage lives in the Tauri shell, not the core.** The SQLite store, the
//! scanner-side ingest write path, and the list/search business logic were
//! relocated to `app/src-tauri/src/whatsapp_data/` (desktop-only). The core
//! keeps only:
//!
//! - [`types`] — the shared serde DTOs (chat / message rows, list / search /
//!   ingest request + response types). Both the core agent tools and the shell
//!   store reference this single definition so the two sides never drift.
//! - [`tools`] — the three read-only agent tools. Their bodies dispatch over
//!   the in-process native request bus
//!   (`openhuman_core::core::event_bus::request_native_global`) keyed by
//!   `whatsapp_data.list_chats` / `.list_messages` / `.search_messages`. The
//!   shell registers the matching handlers at startup.
//!
//! When no shell handler is registered (headless / CLI / docker — no desktop),
//! the tools degrade gracefully to an empty result with a "WhatsApp data
//! unavailable (desktop only)" note rather than erroring.
//!
//! **Data locality**: all data remains on-device; it is never transmitted to
//! any external service.

pub mod tools;
pub mod types;

/// Native-request method names bridging the core agent tools to the shell store.
///
/// The shell registers a handler for each of these via
/// `register_native_global`; the core tools dispatch to them via
/// `request_native_global`. Kept here as the single source of truth so the two
/// sides never disagree on the string key.
pub mod methods {
    /// List chats — req [`super::types::ListChatsRequest`], resp `Vec<`[`super::types::WhatsAppChat`]`>`.
    pub const LIST_CHATS: &str = "whatsapp_data.list_chats";
    /// List messages — req [`super::types::ListMessagesRequest`], resp `Vec<`[`super::types::WhatsAppMessage`]`>`.
    pub const LIST_MESSAGES: &str = "whatsapp_data.list_messages";
    /// Search messages — req [`super::types::SearchMessagesRequest`], resp `Vec<`[`super::types::WhatsAppMessage`]`>`.
    pub const SEARCH_MESSAGES: &str = "whatsapp_data.search_messages";
    /// Ingest a scanner snapshot — req [`super::types::IngestRequest`], resp [`super::types::IngestResult`].
    pub const INGEST: &str = "whatsapp_data.ingest";
}
