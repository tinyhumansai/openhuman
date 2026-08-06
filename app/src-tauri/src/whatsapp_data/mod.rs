//! Shell-side structured WhatsApp Web data store (relocated from the core).
//!
//! Owns the SQLite persistence (`store`), the ingest + list/search business
//! logic (`ops`), the busy/corrupt retry layer (`sqlite_retry`), and a
//! process-global store singleton (`global`). The DB lives at
//! `<workspace_dir>/whatsapp_data/whatsapp_data.db` — the same workspace the
//! core resolves via [`Config`], so the file stays on the internal-path denylist
//! (`security::policy` `WORKSPACE_INTERNAL_DIRS`) and agent tools cannot write it.
//!
//! Two callers reach this store:
//!
//! - **The scanner** (`whatsapp_scanner`) writes via the `whatsapp_data.ingest`
//!   native request (see [`register_native_handlers`]).
//! - **The core agent tools** (`openhuman::channels::whatsapp_data::tools`) query via the
//!   `whatsapp_data.{list_chats,list_messages,search_messages}` native requests.
//! - **The frontend** reads via the Tauri commands
//!   [`whatsapp_data_list_chats`] / [`whatsapp_data_list_messages`] /
//!   [`whatsapp_data_search_messages`].
//!
//! The shared DTOs (request/response/row types) are defined once in the core
//! crate (`openhuman_core::openhuman::channels::whatsapp_data::types`) so both sides agree
//! on a single definition and the native-request `TypeId` checks line up.

mod global;
mod ops;
mod sqlite_retry;
mod store;

use std::sync::Arc;

use openhuman_core::openhuman::channels::whatsapp_data::methods;
use openhuman_core::openhuman::channels::whatsapp_data::types::{
    IngestRequest, IngestResult, ListChatsRequest, ListMessagesRequest, SearchMessagesRequest,
    WhatsAppChat, WhatsAppMessage,
};
use store::WhatsAppDataStore;

/// Lazily open (or return) the shell's whatsapp_data store, bound to the active
/// OpenHuman workspace dir.
///
/// The store is process-global so the scanner, native query handlers, and Tauri
/// commands share one write lock and recovery lifecycle. Every call resolves
/// the active workspace; [`global::init`] reuses the store only when that path
/// still matches, and atomically reopens it after user/reset workspace changes.
pub async fn ensure_store() -> Result<Arc<WhatsAppDataStore>, String> {
    let cfg = openhuman_core::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("[whatsapp_data] config load failed: {e:#}"))?;
    log::debug!(
        "[whatsapp_data] ensuring shell store (workspace={})",
        cfg.workspace_dir.display()
    );
    global::init(cfg.workspace_dir.clone())
}

/// Register the in-process native request handlers that bridge the core (agent
/// tools + scanner) to this shell store. Call once during Tauri `setup`.
///
/// Keyed by the method-name constants the core owns
/// (`openhuman_core::openhuman::channels::whatsapp_data::methods`) so the two sides never
/// drift on the string key.
pub fn register_native_handlers() {
    use openhuman_core::core::event_bus::register_native_global;

    register_native_global::<ListChatsRequest, Vec<WhatsAppChat>, _, _>(
        methods::LIST_CHATS,
        |req| async move {
            let store = ensure_store().await?;
            ops::list_chats(&store, req).map_err(|e| format!("{e:#}"))
        },
    );
    register_native_global::<ListMessagesRequest, Vec<WhatsAppMessage>, _, _>(
        methods::LIST_MESSAGES,
        |req| async move {
            let store = ensure_store().await?;
            ops::list_messages(&store, req).map_err(|e| format!("{e:#}"))
        },
    );
    register_native_global::<SearchMessagesRequest, Vec<WhatsAppMessage>, _, _>(
        methods::SEARCH_MESSAGES,
        |req| async move {
            let store = ensure_store().await?;
            ops::search_messages(&store, req).map_err(|e| format!("{e:#}"))
        },
    );
    register_native_global::<IngestRequest, IngestResult, _, _>(
        methods::INGEST,
        |req| async move {
            let store = ensure_store().await?;
            // `{e:#}` renders the full anyhow chain so the underlying SQLite
            // cause (locked / malformed / FK) survives to the scanner's log.
            ops::ingest(&store, req).map_err(|e| format!("[whatsapp_data] ingest failed: {e:#}"))
        },
    );
    log::info!(
        "[whatsapp_data] registered shell native handlers (list_chats / list_messages / search_messages / ingest)"
    );
}

// ── Tauri commands (frontend read surface) ───────────────────────────────────

/// List locally-stored WhatsApp chats. Frontend replacement for the former
/// `openhuman.whatsapp_data_list_chats` core RPC.
#[tauri::command]
pub async fn whatsapp_data_list_chats(req: ListChatsRequest) -> Result<Vec<WhatsAppChat>, String> {
    log::debug!(
        "[whatsapp_data][cmd] list_chats has_account={} limit={:?} offset={:?}",
        req.account_id.is_some(),
        req.limit,
        req.offset
    );
    let store = ensure_store().await?;
    ops::list_chats(&store, req).map_err(|e| format!("[whatsapp_data] list_chats failed: {e:#}"))
}

/// List messages for a chat. Frontend replacement for the former
/// `openhuman.whatsapp_data_list_messages` core RPC.
#[tauri::command]
pub async fn whatsapp_data_list_messages(
    req: ListMessagesRequest,
) -> Result<Vec<WhatsAppMessage>, String> {
    log::debug!(
        "[whatsapp_data][cmd] list_messages has_account={} (chat redacted)",
        req.account_id.is_some()
    );
    let store = ensure_store().await?;
    ops::list_messages(&store, req)
        .map_err(|e| format!("[whatsapp_data] list_messages failed: {e:#}"))
}

/// Full-text search over stored WhatsApp messages. Frontend-facing companion to
/// the agent's `whatsapp_data_search_messages` tool.
#[tauri::command]
pub async fn whatsapp_data_search_messages(
    req: SearchMessagesRequest,
) -> Result<Vec<WhatsAppMessage>, String> {
    log::debug!(
        "[whatsapp_data][cmd] search_messages has_account={} has_chat={}",
        req.account_id.is_some(),
        req.chat_id.is_some()
    );
    let store = ensure_store().await?;
    ops::search_messages(&store, req)
        .map_err(|e| format!("[whatsapp_data] search_messages failed: {e:#}"))
}
