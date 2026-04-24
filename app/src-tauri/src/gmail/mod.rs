//! Gmail API layer — CDP-driven, zero JS injection.
//!
//! This is the first "data-connect-style" connector for OpenHuman: a
//! typed API surface that reads (and eventually writes) Gmail state out
//! of the logged-in webview. Consumers call the Tauri commands in this
//! module via `invoke('gmail_<fn>', { accountId, … })` — the commands
//! are the standardized API contract.
//!
//! ## Standardized connector shape
//!
//! * Every op is one `#[tauri::command] async fn gmail_<fn>(…)` whose
//!   arguments and return type are the public contract.
//! * All ops take an `account_id: String` to disambiguate multi-account
//!   webview setups. The account must already be open via
//!   `webview_account_open` — the CDP session is discovered by the
//!   `#openhuman-account-<id>` fragment that `cdp::session` appends to
//!   the real URL.
//! * Reads use `DOMSnapshot.captureSnapshot` and/or `Network.*` event
//!   interception.
//! * Writes use `Input.dispatchKeyEvent` / `Input.dispatchMouseEvent`
//!   against the live UI.
//! * Nothing here injects JavaScript into the page.
//!
//! ## Current op coverage
//!
//! | Op                  | Status        |
//! | ------------------- | ------------- |
//! | `gmail_list_labels` | **working**   |
//! | `gmail_list_messages` | stub (Network-MITM follow-up) |
//! | `gmail_search`      | stub          |
//! | `gmail_get_message` | stub          |
//! | `gmail_send`        | stub          |
//! | `gmail_trash`       | stub          |
//! | `gmail_add_label`   | stub          |
//!
//! Stubs return a structured `Err(String)` so the API surface is
//! visible end-to-end — debuggers can see every op registered and
//! immediately tell which are live.
//!
//! ## CEF-only
//!
//! CDP requires a remote-debugging port, which wry doesn't expose.
//! When built without `--features cef` the commands return a clear
//! error instead of being absent, so frontend code doesn't have to
//! branch on the backend.

pub mod types;

#[cfg(feature = "cef")]
mod reads;
#[cfg(feature = "cef")]
mod session;
#[cfg(feature = "cef")]
mod writes;

use types::{Ack, GmailLabel, GmailMessage, GmailSendRequest, SendAck};

#[cfg(not(feature = "cef"))]
const NO_CEF: &str = "gmail API is unavailable without the cef feature (CDP requires remote debugging)";

// ── Tauri commands ──────────────────────────────────────────────────────
//
// Keep these wrappers thin: parse args, delegate, return. The real
// work lives in `reads` / `writes`.

#[tauri::command]
pub async fn gmail_list_labels(account_id: String) -> Result<Vec<GmailLabel>, String> {
    #[cfg(feature = "cef")]
    {
        reads::list_labels(&account_id).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = account_id;
        Err(NO_CEF.into())
    }
}

#[tauri::command]
pub async fn gmail_list_messages(
    account_id: String,
    limit: u32,
    label: Option<String>,
) -> Result<Vec<GmailMessage>, String> {
    #[cfg(feature = "cef")]
    {
        reads::list_messages(&account_id, limit, label).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = (account_id, limit, label);
        Err(NO_CEF.into())
    }
}

#[tauri::command]
pub async fn gmail_search(
    account_id: String,
    query: String,
    limit: u32,
) -> Result<Vec<GmailMessage>, String> {
    #[cfg(feature = "cef")]
    {
        reads::search(&account_id, query, limit).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = (account_id, query, limit);
        Err(NO_CEF.into())
    }
}

#[tauri::command]
pub async fn gmail_get_message(
    account_id: String,
    message_id: String,
) -> Result<GmailMessage, String> {
    #[cfg(feature = "cef")]
    {
        reads::get_message(&account_id, message_id).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = (account_id, message_id);
        Err(NO_CEF.into())
    }
}

#[tauri::command]
pub async fn gmail_send(
    account_id: String,
    request: GmailSendRequest,
) -> Result<SendAck, String> {
    #[cfg(feature = "cef")]
    {
        writes::send(&account_id, request).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = (account_id, request);
        Err(NO_CEF.into())
    }
}

#[tauri::command]
pub async fn gmail_trash(account_id: String, message_id: String) -> Result<Ack, String> {
    #[cfg(feature = "cef")]
    {
        writes::trash(&account_id, message_id).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = (account_id, message_id);
        Err(NO_CEF.into())
    }
}

#[tauri::command]
pub async fn gmail_add_label(
    account_id: String,
    message_id: String,
    label: String,
) -> Result<Ack, String> {
    #[cfg(feature = "cef")]
    {
        writes::add_label(&account_id, message_id, label).await
    }
    #[cfg(not(feature = "cef"))]
    {
        let _ = (account_id, message_id, label);
        Err(NO_CEF.into())
    }
}
