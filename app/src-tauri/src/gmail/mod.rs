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
//! ## CEF runtime required
//!
//! CDP requires a remote-debugging port exposed by the CEF runtime.

pub mod types;

mod atom;
mod cdp_fetch;
mod print_view;
mod reads;
mod session;
mod writes;

use types::{Ack, GmailLabel, GmailMessage, GmailSendRequest, SendAck};

// ── Shared helpers (called by both Tauri IPC and the webview_apis bridge) ──

pub async fn cdp_list_labels(account_id: &str) -> Result<Vec<GmailLabel>, String> {
    reads::list_labels(account_id).await
}

pub async fn cdp_list_messages(
    account_id: &str,
    limit: u32,
    label: Option<String>,
) -> Result<Vec<GmailMessage>, String> {
    reads::list_messages(account_id, limit, label).await
}

pub async fn cdp_search(
    account_id: &str,
    query: String,
    limit: u32,
) -> Result<Vec<GmailMessage>, String> {
    reads::search(account_id, query, limit).await
}

pub async fn cdp_get_message(account_id: &str, message_id: String) -> Result<GmailMessage, String> {
    reads::get_message(account_id, message_id).await
}

pub async fn cdp_send(account_id: &str, request: GmailSendRequest) -> Result<SendAck, String> {
    writes::send(account_id, request).await
}

pub async fn cdp_trash(account_id: &str, message_id: String) -> Result<Ack, String> {
    writes::trash(account_id, message_id).await
}

pub async fn cdp_add_label(
    account_id: &str,
    message_id: String,
    label: String,
) -> Result<Ack, String> {
    writes::add_label(account_id, message_id, label).await
}

/// Find the user's own LinkedIn profile URL by searching Gmail for any
/// `from:linkedin.com` mail, clicking live result rows, and scraping the
/// rendered thread DOM for `comm/in/<username>` (LinkedIn notification
/// footer) or `/in/<username>`.
///
/// Search and extraction are driven through the live Gmail UI via CDP
/// input + DOM snapshot calls, with no page-world JS injection. Returns
/// `None` when the search surfaces no parsable profile URL.
///
/// Used by the onboarding LinkedIn-enrichment pipeline as a stand-in
/// for the Composio Gmail OAuth path that no longer ships.
pub async fn cdp_find_linkedin_profile_url(account_id: &str) -> Result<Option<String>, String> {
    reads::find_linkedin_profile_url(account_id).await
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

/// Debug command — surfaces [`cdp_find_linkedin_profile_url`] to the
/// frontend so the LinkedIn-enrichment Tauri pipeline can be exercised
/// from the dev console (`invoke('gmail_find_linkedin_profile_url',
/// { accountId })`) ahead of the full bridge / core wiring.
#[tauri::command]
pub async fn gmail_find_linkedin_profile_url(account_id: String) -> Result<Option<String>, String> {
    log::debug!("[gmail][tauri] gmail_find_linkedin_profile_url account_id={account_id}");
    cdp_find_linkedin_profile_url(&account_id).await
}
