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
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
// `tokio::time::Instant` (not `std::time::Instant`) so the hard-ceiling
// elapsed check honours `tokio::time::pause()` / `advance()` in unit tests.
use tokio::time::{sleep, Instant};

use super::{browser_ws_url, find_page_target_where, CdpConn};
use crate::webview_accounts::{emit_load_finished, RevealTrigger};

/// Backoff between failed attach attempts / reconnects. Intentionally
/// short — once the webview is open, the target usually shows up within
/// 500ms.
const ATTACH_BACKOFF: Duration = Duration::from_secs(2);

/// Retry schedule used on the very first attach pass after the webview is
/// spawned. The target usually appears almost immediately, but the CEF
/// browser host can take a few hundred ms on cold start. We try at t=0
/// (in case the target is already up — common after the CEF prewarm), then
/// escalate quickly so the worst case before the [`ATTACH_BACKOFF`] kicks
/// in is ~600ms — saving ~500ms on the warm path versus the previous fixed
/// `sleep(500ms)`. Issue #1233.
const INITIAL_ATTACH_SCHEDULE: [Duration; 4] = [
    Duration::from_millis(0),
    Duration::from_millis(50),
    Duration::from_millis(150),
    Duration::from_millis(400),
];

/// How long the page must be **idle** (no CDP progress signal) before the
/// watchdog gives up and synthesises a `webview-account:load{state:"timeout"}`
/// event so the frontend can switch from an empty loading state to explicit
/// retry/help UI on flaky networks. See issue #1213.
///
/// Replaces the previous wall-clock `LOAD_TIMEOUT` (15 s after spawn): a
/// fast initial paint followed by slow subresources would needlessly fire
/// timeout, while a genuinely stuck page would not get more than 15 s of
/// runway. The idle watchdog resets on every `Page.frameStartedLoading` /
/// `Page.frameStoppedLoading` / `Page.lifecycleEvent` /
/// `Page.frameNavigated` / `Page.loadEventFired` so it only fires after a
/// true silence — letting providers like Google Meet take 20–30 s to fully
/// hydrate without spurious timeouts, while still surfacing genuine stalls
/// quickly.
const IDLE_BUDGET: Duration = Duration::from_secs(8);

/// Hard ceiling on total watchdog runtime. If the page is *continuously*
/// emitting progress signals (e.g. an infinite redirect loop, a busy
/// long-poll, a streaming load that never settles) the watchdog must still
/// release the loading spinner so the frontend doesn't hang forever.
/// Picked roughly 2× the slowest provider's observed cold-load tail.
const HARD_CEILING: Duration = Duration::from_secs(60);

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

/// Does `origin` (a `scheme://host[:port]` string from [`origin_of`]) match
/// a specific host? Tolerates an explicit port suffix on `origin` so the
/// callers can pass canonical hosts without hard-coding default ports.
fn origin_host_is(origin: &str, host: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
    else {
        return false;
    };
    let host_part = rest.split(':').next().unwrap_or(rest);
    host_part.eq_ignore_ascii_case(host)
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
/// task keeps the session alive and retries on disconnect. Also spawns an
/// idle-watchdog task that fires a `webview-account:load{state:"timeout"}`
/// event when the page has been silent (no CDP progress signal) for
/// [`IDLE_BUDGET`] OR has been continuously loading for [`HARD_CEILING`].
///
/// The session task and the watchdog communicate over a small mpsc channel:
/// the `pump_events` callback inside `run_session_cycle` sends a `()` ping on
/// every progress-relevant CDP method, which resets the watchdog's idle
/// timer. When the session task exits cleanly the sender drops, the
/// watchdog's `recv()` returns `None`, and it terminates without emitting
/// a stale timeout.
///
/// Both `JoinHandle`s inside the returned [`SpawnedSession`] must be stored
/// by the caller and aborted on account close/purge to prevent task leaks
/// across reopen cycles.
pub fn spawn_session<R: Runtime>(
    app: AppHandle<R>,
    account_id: String,
    real_url: String,
) -> SpawnedSession {
    // 64 is generous — pump_events processes events one at a time, so a
    // backlog only builds if the watchdog itself is starved. We use
    // `try_send` on the producer side so a hypothetical full channel never
    // blocks the CDP event loop.
    let (progress_tx, progress_rx) = mpsc::channel::<()>(64);

    let watchdog = {
        let app = app.clone();
        let account_id = account_id.clone();
        let real_url = real_url.clone();
        tokio::spawn(async move {
            log::debug!(
                "[cdp-session][{}][watchdog] start idle_budget={:?} hard_ceiling={:?} url={}",
                account_id,
                IDLE_BUDGET,
                HARD_CEILING,
                real_url
            );
            let outcome = run_idle_watchdog(progress_rx, IDLE_BUDGET, HARD_CEILING).await;
            match outcome {
                WatchdogOutcome::Idle | WatchdogOutcome::HardCeiling => {
                    log::info!(
                        "[cdp-session][{}][watchdog] firing timeout reason={} url={}",
                        account_id,
                        outcome.reason_str(),
                        real_url
                    );
                    // `emit_load_finished` dedups timeouts that arrive after a
                    // terminal `finished` event — see `loaded_accounts` in
                    // `webview_accounts/mod.rs`. So it is safe to call
                    // unconditionally even if the page actually loaded fine.
                    emit_load_finished(
                        &app,
                        &account_id,
                        "timeout",
                        &real_url,
                        RevealTrigger::Watchdog,
                    );
                }
                WatchdogOutcome::SenderDropped => {
                    log::debug!(
                        "[cdp-session][{}][watchdog] clean exit reason=sender_dropped url={}",
                        account_id,
                        real_url
                    );
                }
            }
        })
    };

    let session =
        tokio::spawn(
            async move { run_session_forever(app, account_id, real_url, progress_tx).await },
        );

    SpawnedSession { session, watchdog }
}

/// Returns `true` for CDP method names we treat as "the page is still
/// making progress" — i.e. a signal that the watchdog's idle timer should
/// be reset. Restricted to Page-domain methods so we do not need to enable
/// `Network.enable` in this session (which would be a behaviour change for
/// every existing webview account).
///
/// Whether a method counts as progress is a *behavioural* decision, so it
/// lives in this dedicated helper that the unit tests can exercise without
/// standing up a real CDP connection.
pub(crate) fn is_progress_signal(method: &str) -> bool {
    matches!(
        method,
        "Page.frameStartedLoading"
            | "Page.frameStoppedLoading"
            | "Page.frameNavigated"
            | "Page.lifecycleEvent"
            | "Page.loadEventFired"
            | "Page.domContentEventFired"
    )
}

/// Outcome of [`run_idle_watchdog`]. Returned (instead of an inline
/// `FnOnce` callback) so the caller can log the *reason* for a timeout
/// — `idle_silence` vs `hard_ceiling` — and distinguish either from a
/// clean sender-dropped exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WatchdogOutcome {
    /// `IDLE_BUDGET` of true silence elapsed without a progress ping.
    Idle,
    /// Total runtime exceeded `HARD_CEILING` even though pings kept arriving.
    HardCeiling,
    /// The session task dropped its sender — clean exit, no timeout fired.
    SenderDropped,
}

impl WatchdogOutcome {
    pub(crate) fn reason_str(self) -> &'static str {
        match self {
            WatchdogOutcome::Idle => "idle_silence",
            WatchdogOutcome::HardCeiling => "hard_ceiling",
            WatchdogOutcome::SenderDropped => "sender_dropped",
        }
    }
}

/// Drives the idle-watchdog state machine. Public-in-crate so the unit
/// tests can exercise it with a mock channel.
///
/// Behaviour:
///
/// 1. On every `()` received from `progress_rx`, restart the
///    [`IDLE_BUDGET`] sleep. The page is still progressing.
/// 2. If the [`IDLE_BUDGET`] sleep elapses with no ping, return
///    [`WatchdogOutcome::Idle`] — the page has gone silent without
///    finishing.
/// 3. If total runtime since spawn exceeds [`HARD_CEILING`] regardless of
///    progress, return [`WatchdogOutcome::HardCeiling`] — prevents an
///    infinite-redirect or chatty long-poll from keeping the spinner up
///    forever.
/// 4. If the sender side drops (`recv()` returns `None`) without a timeout
///    having fired, return [`WatchdogOutcome::SenderDropped`] — the
///    session task ended on its own and the watchdog should NOT emit a
///    stale timeout.
///
/// The `tokio::select!` is `biased;` so the recv arm is polled first
/// each iteration. This prevents a false-positive timeout when both the
/// `IDLE_BUDGET` sleep and a progress ping become ready in the same
/// poll cycle (without `biased`, select picks pseudo-randomly).
pub(crate) async fn run_idle_watchdog(
    mut progress_rx: mpsc::Receiver<()>,
    idle_budget: Duration,
    hard_ceiling: Duration,
) -> WatchdogOutcome {
    let started = Instant::now();
    loop {
        let elapsed = started.elapsed();
        let remaining_ceiling = hard_ceiling.saturating_sub(elapsed);
        if remaining_ceiling.is_zero() {
            return WatchdogOutcome::HardCeiling;
        }
        let wake_after = idle_budget.min(remaining_ceiling);
        tokio::select! {
            biased;
            recv = progress_rx.recv() => {
                match recv {
                    // Progress ping — reset by looping back into select.
                    Some(()) => continue,
                    // Sender dropped (session task ended) — exit clean.
                    None => return WatchdogOutcome::SenderDropped,
                }
            }
            _ = sleep(wake_after) => {
                // No ping inside the wake budget. If we hit the cap because
                // of `hard_ceiling.min(idle_budget)`, classify as hard
                // ceiling so the caller log line is accurate; else idle.
                if wake_after >= remaining_ceiling {
                    return WatchdogOutcome::HardCeiling;
                }
                return WatchdogOutcome::Idle;
            }
        }
    }
}

async fn run_session_forever<R: Runtime>(
    app: AppHandle<R>,
    account_id: String,
    real_url: String,
    progress_tx: mpsc::Sender<()>,
) {
    log::info!(
        "[cdp-session][{}] up real_url={} marker={}",
        account_id,
        real_url,
        placeholder_marker(&account_id)
    );
    // Issue #1233 — first-pass retry schedule replaces the previous fixed
    // `sleep(500ms)` warmup. Try at t=0 (often succeeds when the target was
    // already up via CEF prewarm), then escalate quickly. Each schedule slot
    // sleeps THEN tries, so a target up at t≈0ms attaches without waiting
    // for the old 500ms grace.
    //
    // The steady-state reconnect loop below sleeps `ATTACH_BACKOFF` BEFORE
    // each attempt. That ordering matters: it means an exhausted initial
    // schedule (all four attach attempts failed) gets a proper 2s backoff
    // before the fifth attempt, instead of the original "drop straight in
    // and try immediately" bug that effectively fired five back-to-back
    // attaches in <1s and then waited 2s. After a successful session that
    // ends cleanly we also wait the backoff before reconnecting so we
    // don't tight-loop against a target that just torched its renderer.
    for (idx, delay) in INITIAL_ATTACH_SCHEDULE.iter().enumerate() {
        sleep(*delay).await;
        match run_session_cycle(&app, &account_id, &real_url, &progress_tx).await {
            Ok(()) => {
                log::info!(
                    "[cdp-session][{}] initial session ended cleanly attempt={} reconnecting",
                    account_id,
                    idx
                );
                break;
            }
            Err(e) => {
                log::debug!(
                    "[cdp-session][{}] initial attach attempt={} delay={:?} failed: {}",
                    account_id,
                    idx,
                    delay,
                    e
                );
            }
        }
    }
    loop {
        sleep(ATTACH_BACKOFF).await;
        match run_session_cycle(&app, &account_id, &real_url, &progress_tx).await {
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
    }
}

async fn run_session_cycle<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    real_url: &str,
    progress_tx: &mpsc::Sender<()>,
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
        // Default permission set every embedded provider needs. Origin-scoped
        // so we don't leak grants across providers running in the same CEF
        // browser process.
        let mut perms: Vec<&str> = vec!["notifications"];

        // Google Meet additionally needs:
        //   - audioCapture / videoCapture: getUserMedia for cam/mic so the
        //     pre-call greenroom auto-grants instead of falling back to
        //     Meet's "Use microphone and camera" consent dialog
        //   - displayCapture: getDisplayMedia for screen-share present
        //   - clipboardReadWrite: copy meeting link / paste join code
        // Without these, Meet sits on the consent dialog forever and cam/mic
        // never enumerate (verified during #1022 smoke).
        if origin_host_is(&origin, "meet.google.com") {
            perms.extend_from_slice(&[
                "audioCapture",
                "videoCapture",
                "displayCapture",
                "clipboardReadWrite",
            ]);
        }

        // Slack Huddles need the same media-capture set as Meet:
        //   - audioCapture / videoCapture: getUserMedia for huddle voice +
        //     optional camera tile. Without these, the huddle pre-flight
        //     enumerateDevices returns empty and the join button silently
        //     no-ops.
        //   - displayCapture: getDisplayMedia for in-huddle screen share.
        //   - clipboardReadWrite: huddle invite-link copy + slash-command
        //     paste flows.
        // Mirrors the gmeet pattern from #1054. The huddle popup paint
        // lifecycle bug is tracked separately under #1074 / the CEF
        // tracking issue — granting these perms now means once the paint
        // bug clears, the huddle is functional immediately rather than
        // requiring a follow-up perms wire-up.
        if origin_host_is(&origin, "app.slack.com") {
            perms.extend_from_slice(&[
                "audioCapture",
                "videoCapture",
                "displayCapture",
                "clipboardReadWrite",
            ]);
        }

        if let Err(e) = cdp
            .call(
                "Browser.grantPermissions",
                json!({
                    "origin": origin,
                    "permissions": perms,
                }),
                None,
            )
            .await
        {
            log::warn!(
                "[cdp-session][{}] Browser.grantPermissions({:?}) for {} failed: {}",
                account_id,
                perms,
                origin,
                e
            );
        } else {
            log::info!(
                "[cdp-session][{}] granted {:?} for origin={}",
                account_id,
                perms,
                origin
            );
        }
    }

    // Enable the Page domain so `Page.loadEventFired` reaches our
    // `pump_events` callback below. Must happen BEFORE `Page.navigate` so
    // the first top-level load event for the real provider URL isn't missed.
    cdp.call("Page.enable", json!({}), Some(&session_id))
        .await?;

    // Subscribe to lifecycle events too — they carry sub-load progress
    // signals (`init`, `firstPaint`, `DOMContentLoaded`, `load`,
    // `networkAlmostIdle`, `networkIdle`) that the idle-watchdog uses to
    // distinguish a still-progressing load from a stalled one. See
    // [`run_idle_watchdog`] / issue #1213. Best-effort — if it fails, the
    // watchdog still has frameStarted/Stopped + loadEventFired to work with.
    if let Err(e) = cdp
        .call(
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
            Some(&session_id),
        )
        .await
    {
        log::debug!(
            "[cdp-session][{}] Page.setLifecycleEventsEnabled failed: {} — watchdog falls back to frame-only signals",
            account_id,
            e
        );
    }

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
    let cb_progress_tx = progress_tx.clone();
    cdp.pump_events(&session_id, move |method, _params| {
        // Keep the idle-watchdog (#1213) alive on every progress signal.
        // `try_send` so a hypothetical full channel never blocks the CDP
        // event loop — pings are fungible, dropping one is fine.
        if is_progress_signal(method) {
            let _ = cb_progress_tx.try_send(());
        }
        if method == "Page.loadEventFired" {
            emit_load_finished(
                &cb_app,
                &cb_account_id,
                "finished",
                &cb_real_url,
                RevealTrigger::Load,
            );
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
    fn origin_host_is_matches_canonical_origin() {
        assert!(origin_host_is("https://meet.google.com", "meet.google.com"));
        assert!(origin_host_is(
            "http://meet.google.com:8080",
            "meet.google.com"
        ));
        assert!(origin_host_is("https://MEET.GOOGLE.COM", "meet.google.com"));
    }

    #[test]
    fn origin_host_is_rejects_non_match() {
        // Different host
        assert!(!origin_host_is(
            "https://workspace.google.com",
            "meet.google.com"
        ));
        // Subdomain mismatch
        assert!(!origin_host_is(
            "https://chat.meet.google.com",
            "meet.google.com"
        ));
        // Non-http scheme
        assert!(!origin_host_is("about:blank", "meet.google.com"));
        assert!(!origin_host_is("file:///etc/hosts", "meet.google.com"));
    }

    /// The slack-huddle media-perm grant is host-gated by
    /// `origin_host_is(origin, "app.slack.com")`. Lock the matcher so a
    /// future refactor can't silently widen / narrow the set of origins
    /// that get `audioCapture`/`videoCapture`/`displayCapture` etc.
    #[test]
    fn origin_host_is_matches_app_slack_com_for_huddle_grant() {
        // canonical slack web origin
        assert!(origin_host_is("https://app.slack.com", "app.slack.com"));
        // case-insensitive (matches Url-normalised input + raw header)
        assert!(origin_host_is("https://APP.SLACK.COM", "app.slack.com"));
        // explicit port tolerated
        assert!(origin_host_is("https://app.slack.com:443", "app.slack.com"));

        // marketing site / files CDN must NOT receive media perms — only
        // the huddle-bearing app origin
        assert!(!origin_host_is("https://slack.com", "app.slack.com"));
        assert!(!origin_host_is("https://files.slack.com", "app.slack.com"));
        // unrelated provider
        assert!(!origin_host_is("https://meet.google.com", "app.slack.com"));
        // non-http schemes never match (e.g. about:blank popup placeholder)
        assert!(!origin_host_is("about:blank", "app.slack.com"));
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

    /// Issue #1233 — initial attach retry schedule must finish well under
    /// the previous fixed 500ms warmup so the warm path saves wall-clock
    /// on cold opens. Locked at 4 attempts summing to ≤ 600ms.
    #[test]
    fn initial_attach_schedule_under_600ms_total() {
        let total: Duration = INITIAL_ATTACH_SCHEDULE.iter().sum();
        assert_eq!(
            INITIAL_ATTACH_SCHEDULE.len(),
            4,
            "schedule should have 4 attempts; got {:?}",
            INITIAL_ATTACH_SCHEDULE
        );
        assert!(
            total <= Duration::from_millis(600),
            "schedule total {:?} exceeds 600ms budget",
            total
        );
        assert_eq!(
            INITIAL_ATTACH_SCHEDULE[0],
            Duration::ZERO,
            "first attempt must run immediately (CEF prewarm hits)",
        );
    }

    // -- idle-watchdog (#1213) ---------------------------------------------

    #[test]
    fn is_progress_signal_recognises_known_page_methods() {
        assert!(is_progress_signal("Page.frameStartedLoading"));
        assert!(is_progress_signal("Page.frameStoppedLoading"));
        assert!(is_progress_signal("Page.frameNavigated"));
        assert!(is_progress_signal("Page.lifecycleEvent"));
        assert!(is_progress_signal("Page.loadEventFired"));
        assert!(is_progress_signal("Page.domContentEventFired"));
    }

    #[test]
    fn is_progress_signal_rejects_unrelated_methods() {
        // Non-progress Page methods (we want to ignore window-level chatter)
        assert!(!is_progress_signal("Page.javascriptDialogOpening"));
        assert!(!is_progress_signal("Page.fileChooserOpened"));
        // Other domains
        assert!(!is_progress_signal("Network.requestWillBeSent"));
        assert!(!is_progress_signal("Runtime.consoleAPICalled"));
        assert!(!is_progress_signal(""));
        assert!(!is_progress_signal("nonsense"));
    }

    #[tokio::test(start_paused = true)]
    async fn idle_watchdog_fires_after_idle_budget_with_no_progress() {
        let (tx, rx) = mpsc::channel::<()>(8);
        let handle = tokio::spawn(async move {
            run_idle_watchdog(rx, Duration::from_secs(8), Duration::from_secs(60)).await
        });

        // Hold the sender alive so the watchdog can't exit via channel-closed.
        let _hold = tx;
        // Advance past the idle budget.
        tokio::time::advance(Duration::from_secs(9)).await;
        let outcome = handle.await.expect("watchdog task panicked");

        assert_eq!(
            outcome,
            WatchdogOutcome::Idle,
            "watchdog must surface Idle after silence inside hard ceiling"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn idle_watchdog_resets_on_each_progress_ping() {
        let (tx, rx) = mpsc::channel::<()>(8);
        let handle = tokio::spawn(async move {
            run_idle_watchdog(rx, Duration::from_secs(8), Duration::from_secs(60)).await
        });

        // Drip pings every 5s for 25s total. Idle budget is 8s, so as long
        // as we ping at <8s intervals the watchdog must NOT fire.
        for _ in 0..5 {
            tokio::time::advance(Duration::from_secs(5)).await;
            tx.send(()).await.expect("send ping");
        }

        // Drop the sender → watchdog exits clean.
        drop(tx);
        let outcome = handle.await.expect("watchdog task panicked");
        assert_eq!(
            outcome,
            WatchdogOutcome::SenderDropped,
            "drip-ping then sender-drop path must be classified as clean exit, not timeout"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn idle_watchdog_exits_clean_when_sender_dropped_before_idle() {
        let (tx, rx) = mpsc::channel::<()>(8);
        let handle = tokio::spawn(async move {
            run_idle_watchdog(rx, Duration::from_secs(8), Duration::from_secs(60)).await
        });

        // Session ends quickly — drop sender well before idle budget.
        tokio::time::advance(Duration::from_secs(1)).await;
        drop(tx);
        let outcome = handle.await.expect("watchdog task panicked");

        assert_eq!(
            outcome,
            WatchdogOutcome::SenderDropped,
            "sender-dropped path is a clean exit, not a timeout"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn idle_watchdog_hard_ceiling_caps_runaway_progress() {
        let (tx, rx) = mpsc::channel::<()>(64);
        let handle = tokio::spawn(async move {
            run_idle_watchdog(rx, Duration::from_secs(8), Duration::from_secs(60)).await
        });

        // Send a chatty stream of pings every 1s for 65s — under idle
        // budget every time, but past the 60s hard ceiling.
        for _ in 0..70 {
            tokio::time::advance(Duration::from_secs(1)).await;
            let _ = tx.try_send(());
        }
        let _hold = tx; // keep sender alive so close-path doesn't short-circuit
                        // Allow the spawned task to observe the ceiling.
        tokio::time::advance(Duration::from_secs(1)).await;
        let outcome = handle.await.expect("watchdog task panicked");

        assert_eq!(
            outcome,
            WatchdogOutcome::HardCeiling,
            "hard ceiling must override progress pings once total runtime > ceiling"
        );
    }

    /// Regression for the `biased; recv-first` reordering. With `recv` polled
    /// first each iteration, a ping that lands at exactly the same poll as
    /// the idle-budget sleep must keep the watchdog alive (no false-positive
    /// timeout). Without `biased;` `tokio::select!` picks pseudo-randomly.
    #[tokio::test(start_paused = true)]
    async fn idle_watchdog_biased_recv_wins_over_concurrent_idle_wake() {
        let (tx, rx) = mpsc::channel::<()>(8);
        let handle = tokio::spawn(async move {
            run_idle_watchdog(rx, Duration::from_secs(8), Duration::from_secs(60)).await
        });

        // Park exactly on the boundary: advance the full idle budget AND
        // queue a ping. Without `biased;` the timeout branch could win the
        // race; with `biased;` the recv branch is polled first so the loop
        // resets cleanly.
        tx.send(()).await.expect("send ping");
        tokio::time::advance(Duration::from_secs(8)).await;
        // Drop sender so the watchdog exits clean — if it had fired Idle on
        // the previous wake we'd see Idle instead of SenderDropped here.
        drop(tx);
        let outcome = handle.await.expect("watchdog task panicked");
        assert_eq!(outcome, WatchdogOutcome::SenderDropped);
    }

    #[test]
    fn watchdog_outcome_reason_str_distinguishes_idle_and_ceiling() {
        assert_eq!(WatchdogOutcome::Idle.reason_str(), "idle_silence");
        assert_eq!(WatchdogOutcome::HardCeiling.reason_str(), "hard_ceiling");
        assert_eq!(
            WatchdogOutcome::SenderDropped.reason_str(),
            "sender_dropped"
        );
    }
}
