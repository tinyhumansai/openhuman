//! RPC handler functions for WhatsApp data domain.
//!
//! Each function:
//!   1. Acquires the global `WhatsAppDataStore`.
//!   2. Delegates to `ops::*` for business logic.
//!   3. Returns an `RpcOutcome<T>`.
//!
//! When no WhatsApp session is active (store not yet initialised), the
//! handlers return an actionable "not connected" error so the agent can
//! surface a useful message instead of a crash.

use anyhow::Result;

use crate::openhuman::whatsapp_data::{
    global, ops,
    types::{
        IngestRequest, IngestResult, ListChatsRequest, ListMessagesRequest, SearchMessagesRequest,
        WhatsAppChat, WhatsAppMessage,
    },
};
use crate::rpc::RpcOutcome;

/// Ensure the global store is initialised.
///
/// On first call after core startup this may lazily initialise using the
/// default workspace path. For the scanner-side ingest path the store is
/// already warm from the `core_server` startup sequence.
fn require_store() -> Result<global::WhatsAppDataStoreRef, String> {
    global::store()
}

/// Ingest a WhatsApp scanner snapshot.
///
/// Called by the Tauri whatsapp_scanner after each full CDP scan tick.
pub async fn whatsapp_data_ingest(req: IngestRequest) -> Result<RpcOutcome<IngestResult>, String> {
    log::debug!(
        "[whatsapp_data][rpc] ingest enter chats={} messages={} (account redacted)",
        req.chats.len(),
        req.messages.len()
    );
    let store = require_store()?;
    let result = ops::ingest(&store, req).map_err(|e| {
        log::warn!("[whatsapp_data][rpc] ingest error: {e}");
        format!("[whatsapp_data] ingest failed: {e}")
    })?;
    log::debug!(
        "[whatsapp_data][rpc] ingest ok chats={} messages={} pruned={}",
        result.chats_upserted,
        result.messages_upserted,
        result.messages_pruned
    );
    Ok(RpcOutcome::single_log(
        result,
        "whatsapp_data ingest complete",
    ))
}

/// List WhatsApp chats, optionally filtered by account.
pub async fn whatsapp_data_list_chats(
    req: ListChatsRequest,
) -> Result<RpcOutcome<Vec<WhatsAppChat>>, String> {
    log::debug!(
        "[whatsapp_data][rpc] list_chats enter has_account={} limit={:?} offset={:?}",
        req.account_id.is_some(),
        req.limit,
        req.offset
    );
    let store = require_store()?;
    let chats = ops::list_chats(&store, req).map_err(|e| {
        log::warn!("[whatsapp_data][rpc] list_chats error: {e}");
        format!("[whatsapp_data] list_chats failed: {e}")
    })?;
    log::debug!("[whatsapp_data][rpc] list_chats ok count={}", chats.len());
    Ok(RpcOutcome::single_log(
        chats,
        "whatsapp_data list_chats complete",
    ))
}

/// List messages for a chat, with optional time range and pagination.
pub async fn whatsapp_data_list_messages(
    req: ListMessagesRequest,
) -> Result<RpcOutcome<Vec<WhatsAppMessage>>, String> {
    log::debug!(
        "[whatsapp_data][rpc] list_messages enter has_account={} (chat redacted)",
        req.account_id.is_some()
    );
    let store = require_store()?;
    let msgs = ops::list_messages(&store, req).map_err(|e| {
        log::warn!("[whatsapp_data][rpc] list_messages error: {e}");
        format!("[whatsapp_data] list_messages failed: {e}")
    })?;
    log::debug!("[whatsapp_data][rpc] list_messages ok count={}", msgs.len());
    Ok(RpcOutcome::single_log(
        msgs,
        "whatsapp_data list_messages complete",
    ))
}

/// Full-text search over message bodies.
pub async fn whatsapp_data_search_messages(
    req: SearchMessagesRequest,
) -> Result<RpcOutcome<Vec<WhatsAppMessage>>, String> {
    log::debug!(
        "[whatsapp_data][rpc] search_messages enter has_account={} has_chat={} (query/identifiers redacted)",
        req.account_id.is_some(),
        req.chat_id.is_some()
    );
    let store = require_store()?;
    let results = ops::search_messages(&store, req).map_err(|e| {
        log::warn!("[whatsapp_data][rpc] search_messages error: {e}");
        format!("[whatsapp_data] search_messages failed: {e}")
    })?;
    log::debug!(
        "[whatsapp_data][rpc] search_messages ok count={}",
        results.len()
    );
    Ok(RpcOutcome::single_log(
        results,
        "whatsapp_data search_messages complete",
    ))
}
