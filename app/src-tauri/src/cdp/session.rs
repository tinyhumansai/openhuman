//! Per-account CDP session opener. One long-lived task per webview account
//! that keeps a session attached to the target for the lifetime of the
//! webview.
//!
//! Why long-lived: the session subscribes to `Page.loadEventFired` (used as
//! a belt-and-braces signal for `webview-account:load`). If we attached
//! once and dropped, the load signal would never reach the frontend.
//!
//! Pairs with the placeholder URL the webview is created with — the opener
//! finds the target by its unique `openhuman:{account_id}` marker in the
//! initial URL, injects the notification-permission shim before the page's
//! own JS runs, then navigates the target to the real provider URL with a
//! `#openhuman-account-{id}` fragment appended so other scanners
//! (discord/telegram/slack/whatsapp) can disambiguate multi-account setups
//! without title-marker injection.

use std::time::Duration;

use serde_json::json;
use tauri::{AppHandle, Runtime};
use tokio::task::JoinHandle;
use tokio::time::sleep;

use super::{browser_ws_url, find_page_target_where, CdpConn};
use crate::webview_accounts::emit_load_finished;

/// Backoff between failed attach attempts / reconnects. Intentionally
/// short — once the webview is open, the target usually shows up within
/// 500ms.
const ATTACH_BACKOFF: Duration = Duration::from_secs(2);

/// Watchdog budget before we synthesise a `webview-account:load` event with
/// `state: "timeout"` so the frontend can switch from an empty loading state
/// to explicit retry/help UI on flaky networks. Matches issue #867.
const LOAD_TIMEOUT: Duration = Duration::from_secs(15);

/// Returns the unique marker substring that the account's initial
/// placeholder URL contains so `Target.getTargets` can identify it.
pub fn placeholder_marker(account_id: &str) -> String {
    format!("openhuman-acct-{account_id}")
}

/// Fragment appended to the real provider URL so scanners can match this
/// account uniquely even when several accounts share an origin.
pub fn target_url_fragment(account_id: &str) -> String {
    format!("#openhuman-account-{account_id}")
}

/// Build the placeholder URL used as the webview's initial location.
/// `about:blank` is sufficient for the short holding page we need while CDP
/// attaches and applies overrides before the first real HTTP request.
///
/// We store the account marker in the fragment so `TargetInfo.url` stays
/// unique per account without depending on Tauri's optional `data:` support.
pub fn placeholder_url(account_id: &str) -> String {
    format!("about:blank#{}", placeholder_marker(account_id))
}

/// Extract the origin (`scheme://host[:port]`) from an absolute URL string.
/// Used to scope `Browser.grantPermissions` — the CDP method requires an
/// origin (no path / no fragment / no query) and rejects malformed input.
///
/// Returns `None` for non-`http(s)://` schemes (e.g. `about:blank`,
/// `data:`, `blob:`) where the grant has no meaningful target, and for
/// any input that fails to parse as an absolute URL.
///
/// Implementation note: uses Tauri's re-exported `url::Url` so query
/// strings, fragments, userinfo, and IPv6 hosts are handled correctly
/// instead of relying on raw byte counting.
fn origin_of(url: &str) -> Option<String> {
    let parsed = tauri::Url::parse(url).ok()?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    // `Url::host_str` is the canonical lowercased host. We only emit a
    // bare `scheme://host[:port]` triple — no userinfo, no path, no
    // query, no fragment — since `Browser.grantPermissions` rejects
    // anything else as a malformed origin.
    let host = parsed.host_str()?;
    if let Some(port) = parsed.port() {
        Some(format!("{scheme}://{host}:{port}"))
    } else {
        Some(format!("{scheme}://{host}"))
    }
}

fn target_matches_account_url(target_url: &str, account_id: &str) -> bool {
    let marker = placeholder_marker(account_id);
    let marker_fragment = format!("#{marker}");
    let fragment = target_url_fragment(account_id);
    target_url.ends_with(&marker_fragment) || target_url.ends_with(&fragment)
}

/// Per-account spawn result. Both handles are owned by `WebviewAccountsState`
/// (see `cdp_sessions` and `load_watchdogs`) so close/purge can abort each one
/// without leaking tasks across reopen cycles.
pub struct SpawnedSession {
    pub session: JoinHandle<()>,
    pub watchdog: JoinHandle<()>,
}

/// Spawn the per-account CDP session. Returns immediately; the background
/// task keeps the session alive and retries on disconnect. Also spawns a
/// 15 s watchdog task that fires a `webview-account:load{state:"timeout"}`
/// event if neither the native `on_page_load` nor CDP `Page.loadEventFired`
/// signals arrive in time.
///
/// Both `JoinHandle`s inside the returned [`SpawnedSession`] must be stored
/// by the caller and aborted on account close/purge to prevent task leaks
/// across reopen cycles.
pub fn spawn_session<R: Runtime>(
    app: AppHandle<R>,
    account_id: String,
    real_url: String,
) -> SpawnedSession {
    // Load-overlay watchdog — independent of the session loop. Emits a
    // `timeout` signal after LOAD_TIMEOUT so the frontend's loading spinner
    // is always released even if neither the native `on_page_load` nor the
    // CDP `Page.loadEventFired` signal arrives (flaky network, provider
    // blocking, CDP socket hiccup).
    //
    // `emit_load_finished` only treats terminal load signals (`finished`) as
    // dedup markers. If the watchdog fires first, frontend sees `timeout`
    // and can show retry UI; a later real `finished` signal can still reveal.
    // Late watchdogs after a terminal load are no-ops. The returned
    // `JoinHandle` is stored in `WebviewAccountsState.load_watchdogs` and
    // aborted on close/purge so a watchdog spawned for a vanished account
    // can't fire a stale timeout against a freshly-reused id.
    let watchdog = {
        let app = app.clone();
        let account_id = account_id.clone();
        let real_url = real_url.clone();
        tokio::spawn(async move {
            sleep(LOAD_TIMEOUT).await;
            emit_load_finished(&app, &account_id, "timeout", &real_url);
        })
    };
    let session = tokio::spawn(async move { run_session_forever(app, account_id, real_url).await });
    SpawnedSession { session, watchdog }
}

async fn run_session_forever<R: Runtime>(app: AppHandle<R>, account_id: String, real_url: String) {
    log::info!(
        "[cdp-session][{}] up real_url={} marker={}",
        account_id,
        real_url,
        placeholder_marker(&account_id)
    );
    // Let the webview's target appear in CDP before we start hammering
    // `/json/version`. The placeholder URL is trivial so this is quick.
    sleep(Duration::from_millis(500)).await;
    loop {
        match run_session_cycle(&app, &account_id, &real_url).await {
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
}

async fn run_session_cycle<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    real_url: &str,
) -> Result<(), String> {
    let browser_ws = browser_ws_url().await?;
    let mut cdp = CdpConn::open(&browser_ws).await?;

    // Account-unique match. The placeholder URL and the real provider URL
    // both carry account-specific fragments, so we can use ends_with and
    // avoid substring collisions like `…account-abc` vs `…account-abcdef`.
    let fragment = target_url_fragment(account_id);
    let target =
        find_page_target_where(&mut cdp, |t| target_matches_account_url(&t.url, account_id))
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

    // Stub the Web Notifications permission API before any provider JS
    // runs. Without this, providers like Slack and Gmail show in-app
    // "please enable notifications" banners because Notification.permission
    // returns "default" in the CEF context. The real notification path runs
    // through the CEF IPC hook registered in webview_accounts — this just
    // makes the page's permission check pass.
    cdp.call(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({
            "source": "(function(){\
                function ensureNotificationGranted(){\
                    try {\
                        var NativeNotification = window.Notification;\
                        if (typeof NativeNotification === 'function') {\
                            var OpenHumanNotification = function(title, options){\
                                try { return new NativeNotification(title, options); }\
                                catch (_) { return {}; }\
                            };\
                            OpenHumanNotification.prototype = NativeNotification.prototype;\
                            try {\
                                Object.defineProperty(OpenHumanNotification, 'permission', {\
                                    get: function(){ return 'granted'; },\
                                    configurable: true\
                                });\
                            } catch (_) {}\
                            OpenHumanNotification.requestPermission = function(){\
                                return Promise.resolve('granted');\
                            };\
                            window.Notification = OpenHumanNotification;\
                        }\
                    } catch (_) {}\
                    try {\
                        var p = navigator && navigator.permissions;\
                        if (p && typeof p.query === 'function') {\
                            var q = p.query.bind(p);\
                            var fp = {\
                                query: function(d){\
                                    if (d && d.name === 'notifications') {\
                                        return Promise.resolve({ state: 'granted', onchange: null });\
                                    }\
                                    return q(d);\
                                }\
                            };\
                            Object.defineProperty(navigator, 'permissions', {\
                                get: function(){ return fp; },\
                                configurable: true\
                            });\
                        }\
                    } catch (_) {}\
                }\
                ensureNotificationGranted();\
                try { setInterval(ensureNotificationGranted, 1000); } catch (_) {}\
            })();"
        }),
        Some(&session_id),
    )
    .await?;
    log::debug!(
        "[cdp-session][{}] notification permission stub injected",
        account_id
    );

    // The JS shim above masks `Notification.permission` so providers stop
    // showing "enable notifications" banners, but it does NOT cause CEF's
    // real native-toast pipeline to fire. For that we have to actually grant
    // `notifications` for the provider's origin via the browser-level
    // `Browser.grantPermissions` CDP method (sessionId = None routes to the
    // browser target). With this grant, `new Notification(...)` from the
    // page reaches the CEF helper's notify-IPC, which posts back to
    // `forward_native_notification` in `webview_accounts`. Without it,
    // the constructor silently no-ops and no toast ever fires (#1016).
    if let Some(origin) = origin_of(&real_url) {
        if let Err(e) = cdp
            .call(
                "Browser.grantPermissions",
                json!({
                    "origin": origin,
                    "permissions": ["notifications"],
                }),
                None,
            )
            .await
        {
            log::warn!(
                "[cdp-session][{}] Browser.grantPermissions(notifications) for {} failed: {}",
                account_id,
                origin,
                e
            );
        } else {
            log::info!(
                "[cdp-session][{}] granted notifications for origin={}",
                account_id,
                origin
            );
        }
    }

    // Enable the Page domain so `Page.loadEventFired` reaches our
    // `pump_events` callback below. Must happen BEFORE `Page.navigate` so
    // the first top-level load event for the real provider URL isn't missed.
    cdp.call("Page.enable", json!({}), Some(&session_id))
        .await?;

    // Drive the webview from the placeholder to the real provider URL.
    // Fragment survives same-origin navigations so scanners can match on
    // it indefinitely. Skip navigation if the target is already on the
    // real URL (e.g. we reconnected after a ws drop). Boundary-check
    // the prefix so `https://discord.com` doesn't spuriously match
    // `https://discord.com.evil/…`.
    let at_real_url = target.url.starts_with(real_url)
        && target.url[real_url.len()..]
            .chars()
            .next()
            .is_none_or(|c| matches!(c, '/' | '?' | '#'));
    if !at_real_url {
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
    //
    // The callback emits `webview-account:load{state:"finished"}` on the
    // first `Page.loadEventFired` as a belt-and-braces fallback to the
    // native `WebviewBuilder::on_page_load` handler wired in
    // `webview_account_open`. `emit_load_finished` dedups across both paths
    // so the frontend only sees one signal per cold open.
    let cb_app = app.clone();
    let cb_account_id = account_id.to_string();
    let cb_real_url = real_url.to_string();
    cdp.pump_events(&session_id, move |method, _params| {
        if method == "Page.loadEventFired" {
            emit_load_finished(&cb_app, &cb_account_id, "finished", &cb_real_url);
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_url_uses_about_blank_fragment_marker() {
        assert_eq!(
            placeholder_url("acct-42"),
            "about:blank#openhuman-acct-acct-42"
        );
    }

    #[test]
    fn origin_of_strips_path_query_and_fragment() {
        assert_eq!(
            origin_of("https://app.slack.com/client/T123/C456?foo=bar#frag"),
            Some("https://app.slack.com".to_string())
        );
    }

    #[test]
    fn origin_of_preserves_explicit_port() {
        assert_eq!(
            origin_of("http://localhost:7788/health"),
            Some("http://localhost:7788".to_string())
        );
    }

    #[test]
    fn origin_of_returns_none_for_non_http_schemes() {
        assert_eq!(origin_of("about:blank"), None);
        assert_eq!(origin_of("data:text/plain,hello"), None);
        assert_eq!(origin_of("blob:https://app.slack.com/abc"), None);
        assert_eq!(origin_of("file:///etc/hosts"), None);
    }

    #[test]
    fn origin_of_returns_none_for_malformed_input() {
        assert_eq!(origin_of(""), None);
        assert_eq!(origin_of("not-a-url"), None);
        assert_eq!(origin_of("http://"), None);
    }

    #[test]
    fn origin_of_lowercases_host() {
        // tauri::Url normalises to lowercase host so we never grant
        // permissions twice for `Slack.com` vs `slack.com`.
        assert_eq!(
            origin_of("https://APP.SLACK.COM/client"),
            Some("https://app.slack.com".to_string())
        );
    }

    #[test]
    fn target_match_accepts_placeholder_and_real_provider_fragments_only_for_same_account() {
        assert!(target_matches_account_url(
            "about:blank#openhuman-acct-acct-42",
            "acct-42"
        ));
        assert!(target_matches_account_url(
            "https://discord.com/channels/@me#openhuman-account-acct-42",
            "acct-42"
        ));

        assert!(!target_matches_account_url(
            "about:blank#openhuman-acct-acct-420",
            "acct-42"
        ));
        assert!(!target_matches_account_url(
            "https://example.com/openhuman-acct-acct-42",
            "acct-42"
        ));
        assert!(!target_matches_account_url(
            "https://discord.com/channels/@me#openhuman-account-acct-420",
            "acct-42"
        ));
    }
}
