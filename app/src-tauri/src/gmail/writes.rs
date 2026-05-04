//! Write ops: send, trash, add_label.
//!
//! CDP-only writes are driven via `Input.dispatchKeyEvent` /
//! `Input.dispatchMouseEvent` against the live Gmail UI — no JS
//! injection. Each op is intentionally stubbed for the first cut so
//! the standardized API surface is visible end-to-end; filling them
//! in requires careful UI automation that stays stable across Gmail
//! chrome churn. See plan §deferred.

use super::types::{Ack, GmailSendRequest, SendAck};

pub async fn send(account_id: &str, _req: GmailSendRequest) -> Result<SendAck, String> {
    log::debug!("[gmail][{account_id}] send (not implemented)");
    Err(format!(
        "gmail[{account_id}]: send not implemented — follow-up work per plan §deferred \
         (CDP Input-event automation of the compose dialog)"
    ))
}

pub async fn trash(account_id: &str, _message_id: String) -> Result<Ack, String> {
    log::debug!("[gmail][{account_id}] trash (not implemented)");
    Err(format!(
        "gmail[{account_id}]: trash not implemented — follow-up work"
    ))
}

pub async fn add_label(
    account_id: &str,
    _message_id: String,
    _label: String,
) -> Result<Ack, String> {
    log::debug!("[gmail][{account_id}] add_label (not implemented)");
    Err(format!(
        "gmail[{account_id}]: add_label not implemented — follow-up work"
    ))
}
