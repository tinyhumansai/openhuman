//! Gmail API layer — CDP-driven, zero JS injection.
//!
//! This is the first "data-connect-style" connector for OpenHuman: a
//! typed API surface that reads (and eventually writes) Gmail state out
//! of the logged-in webview. Consumers are:
//!
//! * **Frontend** via `invoke('gmail_<fn>', …)` — the
//!   `#[tauri::command]` wrappers in this module.
//! * **Core sidecar** via the webview_apis WebSocket bridge — the
//!   router in `crate::webview_apis::router` calls the `cdp_*` helpers
//!   below. Core-side JSON-RPC handlers in
//!   `src/openhuman/webview_apis/` proxy through that bridge so curl
//!   against `openhuman.gmail_*` reaches the live webview session.
//!
//! ## Standardized connector shape
//!
//! * Every op has one typed `cdp_<fn>` helper (the public surface both
//!   callers share), plus one thin `#[tauri::command] gmail_<fn>`
//!   wrapper for the frontend path.
//! * All ops take `account_id` to disambiguate multi-account webviews.
//!   The account must already be open via `webview_account_open` — the
//!   CDP session is discovered by the `#openhuman-account-<id>`
//!   fragment that `cdp::session` appends to the real URL.
//! * Reads use `DOMSnapshot.captureSnapshot` and/or `Network.*` events.
//! * Writes use `Input.dispatchKeyEvent` / `Input.dispatchMouseEvent`.
//! * Nothing here injects JavaScript into the page.
//!
//! ## Current op coverage
//!
//! | Op           | Status        |
//! | ------------ | ------------- |
//! | `list_labels` | **working**  |
//! | `list_messages` | stub       |
//! | `search`     | stub          |
//! | `get_message` | stub         |
//! | `send`       | stub          |
//! | `trash`      | stub          |
//! | `add_label`  | stub          |
//!
//! ## CEF-only
//!
//! CDP requires a remote-debugging port, which wry doesn't expose.
//! Without `--features cef` the helpers return a structured error so
//! callers see a clear message instead of a missing symbol.

pub mod types;

#[cfg(feature = "cef")]
mod atom;
#[cfg(feature = "cef")]
mod cdp_fetch;
#[cfg(feature = "cef")]
mod print_view;
#[cfg(feature = "cef")]
mod reads;
#[cfg(feature = "cef")]
mod session;
#[cfg(feature = "cef")]
mod writes;

use types::{Ack, GmailLabel, GmailMessage, GmailSendRequest, SendAck};

#[cfg(not(feature = "cef"))]
const NO_CEF: &str =
    "gmail API is unavailable without the cef feature (CDP requires remote debugging)";

// ── Shared helpers (called by both Tauri IPC and the webview_apis bridge) ──

pub async fn cdp_list_labels(account_id: &str) -> Result<Vec<GmailLabel>, String> {
    #[cfg(feature = "cef")]
    {
        reads::list_labels(account_id).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = account_id;
        Err(NO_CEF.into())
    }
}

pub async fn cdp_list_messages(
    account_id: &str,
    limit: u32,
    label: Option<String>,
) -> Result<Vec<GmailMessage>, String> {
    #[cfg(feature = "cef")]
    {
        reads::list_messages(account_id, limit, label).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = (account_id, limit, label);
        Err(NO_CEF.into())
    }
}

pub async fn cdp_search(
    account_id: &str,
    query: String,
    limit: u32,
) -> Result<Vec<GmailMessage>, String> {
    #[cfg(feature = "cef")]
    {
        reads::search(account_id, query, limit).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = (account_id, query, limit);
        Err(NO_CEF.into())
    }
}

pub async fn cdp_get_message(account_id: &str, message_id: String) -> Result<GmailMessage, String> {
    #[cfg(feature = "cef")]
    {
        reads::get_message(account_id, message_id).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = (account_id, message_id);
        Err(NO_CEF.into())
    }
}

pub async fn cdp_send(account_id: &str, request: GmailSendRequest) -> Result<SendAck, String> {
    #[cfg(feature = "cef")]
    {
        writes::send(account_id, request).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = (account_id, request);
        Err(NO_CEF.into())
    }
}

pub async fn cdp_trash(account_id: &str, message_id: String) -> Result<Ack, String> {
    #[cfg(feature = "cef")]
    {
        writes::trash(account_id, message_id).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = (account_id, message_id);
        Err(NO_CEF.into())
    }
}

pub async fn cdp_add_label(
    account_id: &str,
    message_id: String,
    label: String,
) -> Result<Ack, String> {
    #[cfg(feature = "cef")]
    {
        writes::add_label(account_id, message_id, label).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = (account_id, message_id, label);
        Err(NO_CEF.into())
    }
}

// ── Tauri commands (frontend path) ──────────────────────────────────────

// Entry-point logging at the Tauri command layer distinguishes
// frontend `invoke` paths from the webview_apis bridge path — both
// ultimately call the same `cdp_*` helpers in `reads.rs` / `writes.rs`,
// but the upstream origin matters when tracing a failing flow.

#[tauri::command]
pub async fn gmail_list_labels(account_id: String) -> Result<Vec<GmailLabel>, String> {
    log::debug!("[gmail][tauri] gmail_list_labels account_id={account_id}");
    cdp_list_labels(&account_id).await
}

#[tauri::command]
pub async fn gmail_list_messages(
    account_id: String,
    limit: u32,
    label: Option<String>,
) -> Result<Vec<GmailMessage>, String> {
    log::debug!(
        "[gmail][tauri] gmail_list_messages account_id={account_id} limit={limit} label={label:?}"
    );
    cdp_list_messages(&account_id, limit, label).await
}

#[tauri::command]
pub async fn gmail_search(
    account_id: String,
    query: String,
    limit: u32,
) -> Result<Vec<GmailMessage>, String> {
    log::debug!(
        "[gmail][tauri] gmail_search account_id={account_id} query_len={} limit={limit}",
        query.len()
    );
    cdp_search(&account_id, query, limit).await
}

#[tauri::command]
pub async fn gmail_get_message(
    account_id: String,
    message_id: String,
) -> Result<GmailMessage, String> {
    log::debug!("[gmail][tauri] gmail_get_message account_id={account_id} message_id={message_id}");
    cdp_get_message(&account_id, message_id).await
}

#[tauri::command]
pub async fn gmail_send(account_id: String, request: GmailSendRequest) -> Result<SendAck, String> {
    log::debug!(
        "[gmail][tauri] gmail_send account_id={account_id} to={} cc={} bcc={} body_len={}",
        request.to.len(),
        request.cc.len(),
        request.bcc.len(),
        request.body.len()
    );
    cdp_send(&account_id, request).await
}

#[tauri::command]
pub async fn gmail_trash(account_id: String, message_id: String) -> Result<Ack, String> {
    log::debug!("[gmail][tauri] gmail_trash account_id={account_id} message_id={message_id}");
    cdp_trash(&account_id, message_id).await
}

#[tauri::command]
pub async fn gmail_add_label(
    account_id: String,
    message_id: String,
    label: String,
) -> Result<Ack, String> {
    log::debug!(
        "[gmail][tauri] gmail_add_label account_id={account_id} message_id={message_id} label={label}"
    );
    cdp_add_label(&account_id, message_id, label).await
}
