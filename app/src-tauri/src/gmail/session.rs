//! Helpers for attaching a short-lived CDP session to the logged-in
//! Gmail webview. Every op calls [`attach`], runs a small protocol
//! sequence, then [`detach`]s.
//!
//! Matching strategy: reuse the per-account URL fragment
//! (`#openhuman-account-<id>`) that `cdp::session::spawn_session`
//! appends to the webview's real URL. This is the same mechanism the
//! Discord / Slack / WhatsApp scanners use — see
//! `app/src-tauri/src/cdp/session.rs` `target_url_fragment`.

use crate::cdp::{
    connect_and_attach_matching, detach_session, placeholder_marker, target_url_fragment, CdpConn,
};

/// Attach a CDP session to the Gmail page for `account_id`. Caller must
/// [`detach`] when done (or drop the [`CdpConn`] entirely) so we don't
/// leak sessions in the browser.
///
/// Match strategy: the fragment `#openhuman-account-<id>` is appended
/// by `cdp::session` to the real URL on first navigation, but Gmail
/// may redirect to `accounts.google.com` for auth and strip fragments
/// along the way. To stay robust we accept either the fragment match
/// OR the original placeholder-title marker (used on very early ticks
/// before the first Page.navigate completes). Same strategy the
/// per-account session opener itself uses — see
/// `app/src-tauri/src/cdp/session.rs`.
pub async fn attach(account_id: &str) -> Result<(CdpConn, String), String> {
    let fragment = target_url_fragment(account_id);
    let marker = placeholder_marker(account_id);
    log::debug!(
        "[gmail][{}] attaching CDP session fragment={} marker={}",
        account_id,
        fragment,
        marker
    );
    // Match any of:
    //  - the account-specific fragment (survives same-origin navs),
    //  - the placeholder title (very early ticks, pre-Page.navigate),
    //  - any mail.google.com/* URL (Gmail rewrites the fragment to
    //    `#inbox`, `#search/…`, etc. as soon as the user routes).
    // The third fallback is safe because we only invoke `attach` on
    // behalf of a caller who already knows which `account_id` they
    // want — there's only one Gmail tab per account, and multi-account
    // setups don't open two concurrent `mail.google.com` tabs at once
    // without their distinct fragments persisting.
    let (cdp, session) = connect_and_attach_matching(|t| {
        t.url.contains(&fragment)
            || t.title == marker
            || t.url.starts_with("https://mail.google.com/")
    })
    .await
    .map_err(|e| format!("gmail[{account_id}]: attach failed: {e}"))?;
    log::debug!("[gmail][{}] attached session={}", account_id, session);
    Ok((cdp, session))
}

pub async fn detach(cdp: &mut CdpConn, session: &str) {
    detach_session(cdp, session).await;
}
