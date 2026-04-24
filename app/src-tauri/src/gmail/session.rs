//! Helpers for attaching a short-lived CDP session to the logged-in
//! Gmail webview. Every op calls [`attach`], runs a small protocol
//! sequence, then [`detach`]s.
//!
//! Matching strategy: reuse the per-account URL fragment
//! (`#openhuman-account-<id>`) that `cdp::session::spawn_session`
//! appends to the webview's real URL. This is the same mechanism the
//! Discord / Slack / WhatsApp scanners use — see
//! `app/src-tauri/src/cdp/session.rs` `target_url_fragment`.

use crate::cdp::{connect_and_attach_matching, detach_session, target_url_fragment, CdpConn};

/// URL host prefix every Gmail webview navigates to. Guards against
/// matching the Tauri main window or other external tabs that could
/// share a CDP `#openhuman-account-…` fragment by accident.
const GMAIL_URL_PREFIX: &str = "https://mail.google.com/";

/// Attach a CDP session to the Gmail page for `account_id`. Caller must
/// [`detach`] when done (or drop the [`CdpConn`] entirely) so we don't
/// leak sessions in the browser.
pub async fn attach(account_id: &str) -> Result<(CdpConn, String), String> {
    let fragment = target_url_fragment(account_id);
    log::debug!(
        "[gmail][{}] attaching CDP session fragment={}",
        account_id,
        fragment
    );
    let (cdp, session) = connect_and_attach_matching(|t| {
        t.url.starts_with(GMAIL_URL_PREFIX) && t.url.contains(&fragment)
    })
    .await
    .map_err(|e| format!("gmail[{account_id}]: attach failed: {e}"))?;
    log::debug!("[gmail][{}] attached session={}", account_id, session);
    Ok((cdp, session))
}

pub async fn detach(cdp: &mut CdpConn, session: &str) {
    detach_session(cdp, session).await;
}
