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

    // Pass 1 — account-specific anchors. The fragment survives
    // same-origin navigations on the placeholder and on the real URL
    // up until Gmail's own client rewrites the hash to `#inbox` /
    // `#search/…`; the placeholder title is only set before the first
    // `Page.navigate` completes. If either matches, the attach is
    // unambiguously for this account.
    let fragment_clone = fragment.clone();
    let marker_clone = marker.clone();
    if let Ok((cdp, session)) = connect_and_attach_matching(move |t| {
        t.url.contains(&fragment_clone) || t.title == marker_clone
    })
    .await
    {
        log::debug!(
            "[gmail][{}] attached (account-anchored) session={}",
            account_id,
            session
        );
        return Ok((cdp, session));
    }

    // Pass 2 — fallback. Gmail has navigated away from our fragment,
    // so no anchor remains. We accept any `mail.google.com/*` target,
    // which is safe provided the user has at most one Gmail account
    // open at a time (every `webview_account_open("gmail", …)` gets
    // the fragment-anchor back on its next `Page.navigate`, so two
    // concurrent Gmail webviews will only share the broad predicate
    // for a short window). Log loudly so this case is easy to spot.
    log::warn!(
        "[gmail][{}] account-anchored attach failed; falling back to any mail.google.com/* target",
        account_id
    );
    let (cdp, session) =
        connect_and_attach_matching(|t| t.url.starts_with("https://mail.google.com/"))
            .await
            .map_err(|e| format!("gmail[{account_id}]: attach failed: {e}"))?;
    log::debug!(
        "[gmail][{}] attached (fallback) session={}",
        account_id,
        session
    );
    Ok((cdp, session))
}

pub async fn detach(cdp: &mut CdpConn, session: &str) {
    detach_session(cdp, session).await;
}
