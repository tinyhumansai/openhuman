//! Per-account CDP session opener. One long-lived task per webview account
//! that keeps a session attached to the target for the lifetime of the
//! webview so the UA override (and any future per-target overrides) stays
//! applied.
//!
//! Why long-lived: `Emulation.setUserAgentOverride` reverts when the
//! session detaches. If we attached just once and dropped, subsequent HTTP
//! requests + navigator reads would revert to WKWebView defaults.
//!
//! Pairs with the `data:` placeholder URL the webview is created with —
//! the opener finds the target by its unique `openhuman:{account_id}`
//! marker in the initial URL, applies the UA override, then navigates the
//! target to the real provider URL with a `#openhuman-account-{id}`
//! fragment appended so other scanners (discord/telegram/slack/whatsapp)
//! can disambiguate multi-account setups without title-marker injection.

use std::time::Duration;

use serde_json::json;
use tauri::{AppHandle, Runtime};
use tokio::time::sleep;

use super::{browser_ws_url, find_page_target_where, set_user_agent_override, CdpConn, UaSpec};

/// Backoff between failed attach attempts / reconnects. Intentionally
/// short — once the webview is open, the target usually shows up within
/// 500ms.
const ATTACH_BACKOFF: Duration = Duration::from_secs(2);

/// Returns the unique marker substring that the account's initial
/// placeholder URL contains so `Target.getTargets` can identify it. Same
/// marker is embedded into the document title of the placeholder so
/// `TargetInfo.title` can also be used as a fallback match key.
pub fn placeholder_marker(account_id: &str) -> String {
    format!("openhuman-acct-{account_id}")
}

/// Fragment appended to the real provider URL so scanners can match this
/// account uniquely even when several accounts share an origin.
pub fn target_url_fragment(account_id: &str) -> String {
    format!("#openhuman-account-{account_id}")
}

/// Build the `data:` URL used as the webview's initial location. Holding
/// here for the ~hundreds of ms we need to attach CDP + apply overrides
/// before the first real HTTP request. URL-encoded by hand (the payload
/// is tiny, no external dep).
pub fn placeholder_data_url(account_id: &str) -> String {
    let marker = placeholder_marker(account_id);
    format!(
        "data:text/html;charset=utf-8,%3C%21DOCTYPE%20html%3E%3Ctitle%3E{marker}%3C%2Ftitle%3E%3Cbody%20style%3D%22background%3A%23111%22%3E%3C%2Fbody%3E"
    )
}

/// Spawn the per-account CDP session. Returns immediately; the background
/// task keeps the session alive and retries on disconnect. Idempotent at
/// the call site — the caller is expected to only call this once per
/// `webview_account_open`.
pub fn spawn_session<R: Runtime>(_app: AppHandle<R>, account_id: String, real_url: String) {
    tokio::spawn(async move {
        log::info!(
            "[cdp-session][{}] up real_url={} marker={}",
            account_id,
            real_url,
            placeholder_marker(&account_id)
        );
        // Let the webview's target appear in CDP before we start hammering
        // `/json/version`. The placeholder URL is tiny so this is quick.
        sleep(Duration::from_millis(500)).await;
        loop {
            match run_session_cycle(&account_id, &real_url).await {
                Ok(()) => {
                    log::info!(
                        "[cdp-session][{}] session ended cleanly, reconnecting",
                        account_id
                    );
                }
                Err(e) => {
                    log::debug!("[cdp-session][{}] cycle failed: {}", account_id, e);
                }
            }
            sleep(ATTACH_BACKOFF).await;
        }
    });
}

async fn run_session_cycle(account_id: &str, real_url: &str) -> Result<(), String> {
    let browser_ws = browser_ws_url().await?;
    let mut cdp = CdpConn::open(&browser_ws).await?;

    // The placeholder URL embeds the account id in both the URL payload AND
    // the <title>. Match on either — the title field is populated as soon
    // as the data URL document parses, which is effectively immediate.
    let marker = placeholder_marker(account_id);
    let fragment = target_url_fragment(account_id);
    let target = find_page_target_where(&mut cdp, |t| {
        t.url.contains(&marker) || t.title.contains(&marker) || t.url.contains(&fragment)
    })
    .await?;
    log::info!(
        "[cdp-session][{}] attaching to target {} url={}",
        account_id,
        target.id,
        target.url
    );

    let attach = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": target.id, "flatten": true }),
            None,
        )
        .await?;
    let session_id = attach
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "attach missing sessionId".to_string())?
        .to_string();

    // UA override BEFORE navigate so the first real HTTP request carries
    // the Chrome UA at the network layer AND navigator.* readouts return
    // the spoofed values from the very first page script.
    let ua = UaSpec::chrome_mac();
    set_user_agent_override(&mut cdp, &session_id, &ua).await?;
    log::info!(
        "[cdp-session][{}] ua override applied session={}",
        account_id,
        session_id
    );

    // Drive the webview from the placeholder to the real provider URL.
    // Fragment survives same-origin navigations so scanners can match on
    // it indefinitely. Skip navigation if the target is already on the
    // real URL (e.g. we reconnected after a ws drop).
    if !target.url.starts_with(real_url) {
        let dest = if real_url.contains('#') {
            real_url.to_string()
        } else {
            format!("{real_url}{fragment}")
        };
        log::info!("[cdp-session][{}] navigating to {}", account_id, dest);
        cdp.call("Page.navigate", json!({ "url": dest }), Some(&session_id))
            .await?;
    }

    // Hold the session open for the lifetime of the webview. The UA
    // override reverts when we detach, so we intentionally block here.
    // pump_events returns when the CDP ws closes (browser process exits
    // or `Target.detachFromTarget` is called from elsewhere).
    cdp.pump_events(&session_id, |_method, _params| {
        // We don't subscribe to any domain here — the session exists
        // purely to keep the UA override resident. Per-provider scanners
        // attach their own sessions for Network / IndexedDB / DOMSnapshot.
    })
    .await
}
