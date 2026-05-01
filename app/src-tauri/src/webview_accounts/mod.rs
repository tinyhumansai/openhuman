//! Franz-style embedded webview accounts.
//!
//! Hosts third-party web apps (WhatsApp Web, Gmail, …) as a child Tauri
//! `Webview` positioned inside the main React window at a rect chosen by the
//! UI. A small per-provider "recipe" JS file is injected via
//! `initialization_script` to scrape the DOM and pipe state back to Rust as
//! `webview_recipe_event` invocations. Rust forwards each event up to the
//! React UI as a `webview:event` Tauri event; React is responsible for
//! persisting interesting payloads to memory via the existing core RPC.
//!
//! Architecture:
//!   React → invoke('webview_account_open',  …)  → spawn child Webview
//!   React → invoke('webview_account_bounds', …) → reposition / resize
//!   recipe → invoke('webview_recipe_event',  …) → emit('webview:event', …)
//!
//! Per-account session isolation: each account gets its own
//! `data_directory` under `{app_local_data_dir}/webview_accounts/{id}` so
//! cookies and storage don't bleed between accounts (best-effort on
//! WKWebView — see Tauri docs on `data_store_identifier` for the macOS path).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
#[cfg(target_os = "linux")]
use std::sync::{mpsc::sync_channel, OnceLock};
use std::time::Duration;

use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{
    webview::NewWindowResponse, AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Runtime,
    Url, WebviewBuilder, WebviewUrl,
};
#[cfg(windows)]
use tauri_plugin_notification::NotificationExt;
// `ImplBrowser` exposes `Browser::identifier()` — bring the trait into scope
// so the `with_webview` callback can read the CEF browser id.
use cef::ImplBrowser;

use crate::cdp;

const RUNTIME_JS: &str = include_str!("runtime.js");
const LINKEDIN_RECIPE_JS: &str = include_str!("../../recipes/linkedin/recipe.js");
const GMAIL_RECIPE_JS: &str = include_str!("../../recipes/gmail/recipe.js");
const GOOGLE_MEET_RECIPE_JS: &str = include_str!("../../recipes/google-meet/recipe.js");

/// Registered providers and their service URLs. Add a new arm here plus a
/// recipe.js file under `recipes/<id>/` to support another provider.
fn provider_url(provider: &str) -> Option<&'static str> {
    match provider {
        "whatsapp" => Some("https://web.whatsapp.com/"),
        "telegram" => Some("https://web.telegram.org/k/"),
        "linkedin" => Some("https://www.linkedin.com/messaging/"),
        "gmail" => Some("https://mail.google.com/mail/u/0/"),
        "slack" => Some("https://app.slack.com/client/"),
        "discord" => Some("https://discord.com/channels/@me"),
        "google-meet" => Some("https://meet.google.com/"),
        "zoom" => Some("https://zoom.us/"),
        "browserscan" => Some("https://www.browserscan.net/bot-detection"),
        _ => None,
    }
}

/// Returns the injected recipe.js for providers that still rely on the
/// JS-bridge ingest path. Migrated providers (whatsapp, telegram, slack,
/// discord, browserscan) return `None` — their scraping runs natively via
/// CDP in the per-provider scanner modules.
fn provider_recipe_js(provider: &str) -> Option<&'static str> {
    match provider {
        "linkedin" => Some(LINKEDIN_RECIPE_JS),
        "gmail" => Some(GMAIL_RECIPE_JS),
        "google-meet" => Some(GOOGLE_MEET_RECIPE_JS),
        _ => None,
    }
}

/// Whether this provider is supported at all. Derived from
/// `provider_url` so there's one canonical list — new providers added
/// to the `provider_url` match automatically become "supported" here.
fn provider_is_supported(provider: &str) -> bool {
    provider_url(provider).is_some()
}

/// Host suffixes the embedded webview is allowed to navigate within. Any
/// navigation to a host outside this set is cancelled and opened in the
/// user's default browser instead. Gmail / Meet include Google's auth and
/// static asset hosts so the OAuth redirect loop works; Discord includes
/// its CDN subdomains for the same reason.
fn provider_allowed_hosts(provider: &str) -> &'static [&'static str] {
    match provider {
        "whatsapp" => &["whatsapp.com", "whatsapp.net", "wa.me"],
        "telegram" => &["telegram.org", "t.me"],
        "linkedin" => &["linkedin.com", "licdn.com"],
        "gmail" => &[
            "google.com",
            "googleusercontent.com",
            "gstatic.com",
            "googleapis.com",
        ],
        "slack" => &["slack.com", "slack-edge.com", "slackb.com"],
        "discord" => &[
            "discord.com",
            "discord.gg",
            "discordapp.com",
            "discordapp.net",
        ],
        "google-meet" => &[
            "google.com",
            "googleusercontent.com",
            "gstatic.com",
            "googleapis.com",
        ],
        "zoom" => &["zoom.us", "zoom.com", "zoomgov.com", "zdassets.com"],
        "browserscan" => &["browserscan.net"],
        _ => &[],
    }
}

/// Rewrite a provider-specific native-app deep link (e.g. Zoom's
/// `zoomus://zoom.us/join?...`) into a web-client URL so the meeting stays
/// inside the embedded webview instead of failing with
/// ERR_UNKNOWN_URL_SCHEME (CEF has no handler for these schemes).
///
/// Returns `Some(rewritten)` when the provider claims the scheme and a
/// valid web-client URL can be built; `None` otherwise (caller should
/// leave the navigation alone).
fn rewrite_provider_deep_link(provider: &str, url: &Url) -> Option<Url> {
    if provider != "zoom" {
        return None;
    }
    if !matches!(url.scheme(), "zoomus" | "zoommtg") {
        return None;
    }
    // Pull the meeting id out of the query string. Zoom uses `confno` on
    // both `action=join` (joining) and `action=start` (hosting) flows.
    let confno = url
        .query_pairs()
        .find(|(k, _)| k == "confno")
        .map(|(_, v)| v.into_owned());
    let pwd = url
        .query_pairs()
        .find(|(k, _)| k == "pwd" || k == "tk")
        .map(|(_, v)| v.into_owned());
    // Build the rewritten URL via `Url` so `confno` and `pwd` are
    // percent-encoded — inbound Zoom tokens can contain reserved chars
    // (`&`, `#`, `%`, `+`, …) that would corrupt a hand-rolled
    // `format!(…)` string and silently break the join/host flow.
    match confno {
        Some(id) if !id.is_empty() => {
            // Base without trailing slash; `path_segments_mut().push(id)`
            // appends `/id` cleanly. A trailing `/` on the base would yield
            // `/wc/join//id` (empty segment preserved by the Url spec).
            let mut rewritten = Url::parse("https://app.zoom.us/wc/join").ok()?;
            rewritten.path_segments_mut().ok()?.push(&id);
            if let Some(p) = pwd.filter(|p| !p.is_empty()) {
                rewritten.query_pairs_mut().append_pair("pwd", &p);
            }
            Some(rewritten)
        }
        _ => Url::parse("https://app.zoom.us/wc/home").ok(),
    }
}

/// `true` if `url` is considered in-app for `provider`. Non-HTTP(S)
/// schemes (`about:blank`, `data:`, `blob:`) have no host and are always
/// allowed so the webview's own internal navigations keep working.
/// Unknown providers are also permissive — better to accidentally keep a
/// link in-app than to leak it to the system browser.
fn url_is_internal(provider: &str, url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return true;
    };
    let allowed = provider_allowed_hosts(provider);
    if allowed.is_empty() {
        return true;
    }
    allowed
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{}", suffix)))
}

/// `true` if the provider needs `window.open(url)` to return a live
/// window-handle (i.e. the calling site reads the return value and aborts
/// on falsey). Slack Huddles go through `openManagedChildWindow` which
/// calls `window.open("about:blank", …)` and then programmatically
/// navigates the returned popup to the huddle UI. Denying the popup
/// makes the huddle call fail silently with a `beacon/error`. For these
/// cases we allow the default popup so CEF spawns an in-app child window
/// and returns a real handle to the caller.
///
/// Match is intentionally narrow — only the popup URLs the provider
/// actually needs in-app pass. Cmd/Ctrl-click and `target="_blank"`
/// on ordinary links (which carry a concrete URL) still route out to
/// the user's default browser.
fn popup_should_stay_in_app(provider: &str, url: &Url) -> bool {
    match provider {
        "slack" => {
            // Slack's huddle flow opens `about:blank` first, then navigates
            // the popup to the huddle URL — at popup-creation time there is
            // no host yet. Also accept same-origin slack.com hosts so direct
            // `window.open("https://app.slack.com/...")` calls stay in-app.
            if url.scheme() == "about" {
                return true;
            }
            match url.host_str() {
                Some(host) => host == "app.slack.com" || host.ends_with(".slack.com"),
                None => false,
            }
        }
        "zoom" => {
            // Zoom's "Join from browser" / WebClient launch can go through a
            // `window.open("https://app.zoom.us/wc/...")` popup instead of an
            // in-page navigation. Keep those (and any deep-link-rewritten
            // popup targeting the same path) inside the embedded webview so
            // the meeting doesn't pop out to the system browser.
            match url.host_str() {
                Some(host) => {
                    (host == "app.zoom.us" || host == "zoom.us") && url.path().starts_with("/wc/")
                }
                None => false,
            }
        }
        _ => false,
    }
}

/// `true` if `scheme` is a known provider native-desktop-app deep-link
/// scheme. We suppress these instead of routing them to the system
/// browser because macOS hands them to the native provider app
/// (e.g. `slack://magic-login/<token>` signs the native Slack app into
/// the workspace, breaking embedded-webview isolation: the workspace's
/// session ends up inside the native client even though the user only
/// signed in via OpenHuman's embedded webview).
///
/// The HTTPS fallback in each provider's web flow handles sign-in
/// without the deep link, so suppression is safe — the page just
/// continues on the next link in the sequence.
///
/// Caller contract: only suppress when [`rewrite_provider_deep_link`]
/// has already returned `None` for the URL. Schemes we DO know how to
/// rewrite into a web-client URL (e.g. `zoomus://`) must take the
/// rewrite path first; those flows expect to stay in-app, not be
/// silently dropped.
fn is_provider_native_deep_link_scheme(scheme: &str) -> bool {
    matches!(
        scheme,
        "slack" | "discord" | "tg" | "msteams" | "zoomus" | "zoommtg"
    )
}

/// `true` if a popup request should be denied AND the parent webview
/// should be navigated to the popup URL instead.
///
/// Used for Google's "Sign in" / "Use another account" flow: clicking
/// the link issues `window.open("https://accounts.google.com/...")`. We
/// can't route that to the system browser (the auth cookie would land
/// in the wrong jar) and we don't want to let CEF spawn an unmanaged
/// child window (it has no host rect, so it renders blank/black). The
/// safe option is to deny the popup and replace the parent's URL — the
/// in-app webview was already at mail.google.com so taking over the
/// frame to finish auth is exactly what the user expects.
fn popup_should_navigate_parent(provider: &str, url: &Url) -> Option<Url> {
    match provider {
        "gmail" | "google-meet" => {
            if url.scheme() == "about" {
                return None;
            }
            if is_google_auth_popup(url) {
                return Some(url.clone());
            }
            // Gmeet: "Start an instant meeting" / "New meeting" / clicking
            // a meeting code link calls `window.open(meet.google.com/<roomid>)`
            // to launch the room. Default popup handling would route the
            // URL to the user's system browser, leaking the Meet session
            // out of OpenHuman entirely. Deny the popup and navigate the
            // embedded parent into the room URL instead — matches the
            // user's expectation that the meeting stays in-app.
            if provider == "google-meet" {
                if let Some(host) = url.host_str() {
                    if host == "meet.google.com" {
                        return Some(url.clone());
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn is_google_auth_popup(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let is_google_auth_host =
        host == "accounts.google.com" || host == "accounts.googleusercontent.com";
    if !is_google_auth_host {
        return false;
    }

    let path = url.path().to_ascii_lowercase();
    if path.contains("signin")
        || path.contains("servicelogin")
        || path.contains("accountchooser")
        || path.contains("chooseaccount")
    {
        return true;
    }

    url.query_pairs().any(|(key, value)| {
        let k = key.to_ascii_lowercase();
        let v = value.to_ascii_lowercase();
        matches!(k.as_str(), "flowname" | "service" | "continue")
            && (v.contains("signin")
                || v.contains("servicelogin")
                || v.contains("accountchooser")
                || v.contains("chooseaccount"))
    })
}

fn redact_navigation_url(url: &Url) -> String {
    let mut safe = url.clone();
    safe.set_query(None);
    safe.set_fragment(None);
    safe.to_string()
}
/// Unwrap provider-side "link safety" redirects so the system browser
/// lands on the real destination.
///
/// These wrappers (LinkedIn's `/safety/go/?url=…`, etc.) require the
/// user to be logged into the provider in the destination browser. In
/// our setup the session lives inside the embedded CEF webview's cookie
/// jar, not the user's default browser — opening the wrapper URL there
/// shows a broken safety page instead of completing the redirect.
/// Extract the `url` query param and return the resolved destination.
fn unwrap_provider_redirect(url: &Url) -> Option<Url> {
    let host = url.host_str()?;
    let path = url.path();
    let matches_linkedin = (host == "www.linkedin.com" || host == "linkedin.com")
        && (path == "/safety/go/" || path == "/safety/go" || path == "/redir/redirect");
    if !matches_linkedin {
        return None;
    }
    let (_, raw) = url.query_pairs().find(|(k, _)| k == "url")?;
    Url::parse(&raw).ok()
}

/// Fire-and-forget handoff to the OS default URL handler. Any error is
/// logged but not propagated — we've already cancelled the in-app
/// navigation so there's nowhere to surface a failure to.
///
/// On macOS we shell out to `/usr/bin/open` directly rather than via
/// `tauri_plugin_opener::open_url`: the plugin returned Ok but no browser
/// actually launched in the CEF runtime (suspected sandbox/launch-service
/// interaction with the `open` crate's detached spawn). The direct
/// Command call is equivalent to what a user would type in Terminal and
/// works reliably.
fn open_in_system_browser(url: &str) {
    #[cfg(target_os = "macos")]
    {
        match std::process::Command::new("/usr/bin/open").arg(url).spawn() {
            Ok(_) => log::info!("[webview-accounts] opened externally (macos open): {}", url),
            Err(e) => log::warn!(
                "[webview-accounts] /usr/bin/open {} failed: {} — falling back to opener plugin",
                url,
                e
            ),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        match tauri_plugin_opener::open_url(url, None::<&str>) {
            Ok(()) => log::info!("[webview-accounts] opened externally: {}", url),
            Err(e) => log::warn!("[webview-accounts] open_url({}) failed: {}", url, e),
        }
    }
}

fn payload_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn payload_bool(payload: &serde_json::Value, key: &str) -> Option<bool> {
    payload.get(key).and_then(|v| v.as_bool())
}

fn payload_i64(payload: &serde_json::Value, key: &str) -> Option<i64> {
    payload.get(key).and_then(|v| v.as_i64())
}

fn first_message_field(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get("messages")
        .and_then(|v| v.as_array())
        .and_then(|messages| messages.first())
        .and_then(|message| message.get(key))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn event_timestamp_rfc3339(ts_ms: Option<i64>) -> String {
    ts_ms
        .and_then(|ts| Utc.timestamp_millis_opt(ts).single())
        .unwrap_or_else(Utc::now)
        .to_rfc3339()
}

fn normalize_provider_surfaces_event(args: &RecipeEventArgs) -> Option<serde_json::Value> {
    if args.kind != "ingest" {
        return None;
    }

    let entity_id = payload_string(&args.payload, "entity_id")
        .or_else(|| payload_string(&args.payload, "threadId"))
        .or_else(|| payload_string(&args.payload, "chatId"))
        .or_else(|| payload_string(&args.payload, "snapshotKey"))
        .unwrap_or_else(|| {
            format!(
                "{}:{}:{}",
                args.provider,
                args.account_id,
                args.ts.unwrap_or_else(|| Utc::now().timestamp_millis())
            )
        });

    let thread_id = payload_string(&args.payload, "threadId")
        .or_else(|| payload_string(&args.payload, "chatId"))
        .or_else(|| payload_string(&args.payload, "conversationId"));
    let title = payload_string(&args.payload, "title")
        .or_else(|| payload_string(&args.payload, "chatName"))
        .or_else(|| payload_string(&args.payload, "channelName"));
    let snippet = payload_string(&args.payload, "snippet")
        .or_else(|| first_message_field(&args.payload, "body"));
    let sender_name = payload_string(&args.payload, "senderName")
        .or_else(|| first_message_field(&args.payload, "from"));
    let sender_handle = payload_string(&args.payload, "senderHandle");
    let deep_link = payload_string(&args.payload, "deepLink");
    let unread = payload_i64(&args.payload, "unread").unwrap_or(0);
    let requires_attention = payload_bool(&args.payload, "requires_attention")
        .unwrap_or(unread > 0 || sender_name.is_some() || snippet.is_some());

    Some(json!({
        "provider": args.provider,
        "account_id": args.account_id,
        "event_kind": args.kind,
        "entity_id": entity_id,
        "thread_id": thread_id,
        "title": title,
        "snippet": snippet,
        "sender_name": sender_name,
        "sender_handle": sender_handle,
        "timestamp": event_timestamp_rfc3339(args.ts),
        "deep_link": deep_link,
        "requires_attention": requires_attention,
        "raw_payload": args.payload,
    }))
}

async fn post_provider_surfaces_event(args: &RecipeEventArgs) -> Result<(), String> {
    let Some(params) = normalize_provider_surfaces_event(args) else {
        return Ok(());
    };

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "openhuman.provider_surfaces_ingest_event",
        "params": params,
    });

    let url = std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:7788/rpc".to_string());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("{status}: {body}"));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("decode: {e}"))?;
    if let Some(err) = v.get("error") {
        return Err(format!("rpc error: {err}"));
    }
    Ok(())
}

/// Human-readable label used as the title prefix on native notifications
/// so users can tell which provider fired the ping. Matches the labels
/// in the frontend `PROVIDERS` registry.
pub fn provider_display_name(provider: &str) -> &'static str {
    match provider {
        "whatsapp" => "WhatsApp",
        "telegram" => "Telegram",
        "linkedin" => "LinkedIn",
        "gmail" => "Gmail",
        "slack" => "Slack",
        "discord" => "Discord",
        "google-meet" => "Google Meet",
        "zoom" => "Zoom",
        "browserscan" => "BrowserScan",
        _ => "OpenHuman",
    }
}

#[derive(Default)]
pub struct WebviewAccountsState {
    /// account_id -> webview label (we use `acct_<id>` as the label).
    inner: Mutex<HashMap<String, String>>,
    /// account_id -> CEF `Browser::identifier()`. Populated asynchronously
    /// inside the `with_webview` callback once the renderer hands us the
    /// browser handle, and consumed at close/purge time so we can call
    /// `tauri_runtime_cef::notification::unregister` without leaking
    /// per-browser handler entries across account churn.
    browser_ids: Mutex<HashMap<String, i32>>,
    /// account_id -> CDP session task. One long-lived task per account
    /// keeps the UA override resident (see `cdp::session`); aborted on
    /// close/purge so reopen cycles don't stack multiple live loops.
    cdp_sessions: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    /// account_id -> 15s `webview-account:load{state:"timeout"}` watchdog.
    /// Aborted in close/purge so a watchdog spawned for a now-closed
    /// account can't fire a stale timeout against a freshly-reused id.
    load_watchdogs: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    /// account_id of webviews that have already emitted their first
    /// `webview-account:load{state:"finished"}` event. Used to dedup
    /// triple-signal fires (native on_page_load, CDP `Page.loadEventFired`,
    /// 15 s watchdog) so the frontend only reveals once per cold open.
    loaded_accounts: Mutex<HashSet<String>>,
    /// Last bounds requested by the frontend for a given account, captured at
    /// `webview_account_open` time so the off-screen-spawned webview can be
    /// revealed at the right rect without the frontend having to round-trip
    /// them again.
    requested_bounds: Mutex<HashMap<String, Bounds>>,
    /// Runtime notification-bypass controls used by the settings UI.
    notification_bypass: Mutex<NotificationBypassPrefs>,
}

impl WebviewAccountsState {
    /// Drain every per-account resource owned by this state and abort the
    /// associated background tasks. Returns the `(account_id, label)`
    /// pairs of webviews that still need closing — the caller does the
    /// actual `wv.close()` because that needs an `AppHandle`. Splitting
    /// it out keeps the rest of the teardown unit-testable without
    /// constructing a Tauri runtime.
    ///
    /// Aborts CDP session tasks and load watchdogs, unregisters CEF
    /// notification handlers, and clears the loaded-accounts /
    /// requested-bounds bookkeeping. All collections are drained — a
    /// repeat call returns an empty `Vec` and is a safe no-op.
    fn drain_for_shutdown(&self) -> Vec<(String, String)> {
        let cdp_tasks: Vec<_> = self
            .cdp_sessions
            .lock()
            .ok()
            .map(|mut g| g.drain().collect())
            .unwrap_or_default();
        for (acct, task) in cdp_tasks {
            task.abort();
            log::debug!("[webview-accounts] shutdown abort cdp account={}", acct);
        }
        let watchdogs: Vec<_> = self
            .load_watchdogs
            .lock()
            .ok()
            .map(|mut g| g.drain().collect())
            .unwrap_or_default();
        for (acct, task) in watchdogs {
            task.abort();
            log::debug!(
                "[webview-accounts] shutdown abort watchdog account={}",
                acct
            );
        }
        let browser_ids: Vec<_> = self
            .browser_ids
            .lock()
            .ok()
            .map(|mut g| g.drain().collect())
            .unwrap_or_default();
        for (acct, browser_id) in browser_ids {
            tauri_runtime_cef::notification::unregister(browser_id);
            log::debug!(
                "[notify-cef] shutdown unregistered handler account={} browser_id={}",
                acct,
                browser_id
            );
        }
        if let Ok(mut g) = self.loaded_accounts.lock() {
            g.clear();
        }
        if let Ok(mut g) = self.requested_bounds.lock() {
            g.clear();
        }
        self.inner
            .lock()
            .ok()
            .map(|mut g| g.drain().collect())
            .unwrap_or_default()
    }

    /// Tear down every per-account resource owned by this state — used by
    /// the app's `RunEvent::ExitRequested` path so nothing outlives the
    /// tokio runtime / `AppHandle` (issue #920).
    ///
    /// On top of [`drain_for_shutdown`], this closes every `acct_*` child
    /// webview so CEF browsers tear down before `cef::shutdown()` runs,
    /// and tells the per-account scanner registries to forget the
    /// account so a future open of the same id starts from a clean slate.
    /// All collections are drained — repeat calls are cheap no-ops.
    pub fn shutdown_all<R: Runtime>(&self, app: &AppHandle<R>) {
        let labels = self.drain_for_shutdown();
        for (acct, label) in labels {
            teardown_account_scanners(app, &acct);
            if let Some(wv) = app.get_webview(&label) {
                if let Err(e) = wv.close() {
                    log::warn!(
                        "[webview-accounts] shutdown close({label}) failed account={acct}: {e}"
                    );
                }
            }
        }
        log::info!("[webview-accounts] shutdown_all complete");
    }
}

/// Tell the per-account scanner registries (whatsapp / slack / discord /
/// telegram) to forget `account_id`. Each `forget` is fire-and-forget so
/// the caller doesn't need to be `async`. Shared by `webview_account_close`,
/// `webview_account_purge`, and `WebviewAccountsState::shutdown_all` so
/// every exit path goes through the same teardown.
fn teardown_account_scanners<R: Runtime>(app: &AppHandle<R>, account_id: &str) {
    if let Some(registry) =
        app.try_state::<std::sync::Arc<crate::whatsapp_scanner::ScannerRegistry>>()
    {
        let registry = registry.inner().clone();
        let acct = account_id.to_string();
        tauri::async_runtime::spawn(async move { registry.forget(&acct).await });
    }
    if let Some(registry) = app.try_state::<std::sync::Arc<crate::slack_scanner::ScannerRegistry>>()
    {
        let registry = registry.inner().clone();
        let acct = account_id.to_string();
        tauri::async_runtime::spawn(async move { registry.forget(&acct).await });
    }
    if let Some(registry) =
        app.try_state::<std::sync::Arc<crate::discord_scanner::ScannerRegistry>>()
    {
        let registry = registry.inner().clone();
        let acct = account_id.to_string();
        tauri::async_runtime::spawn(async move { registry.forget(&acct).await });
    }
    if let Some(registry) =
        app.try_state::<std::sync::Arc<crate::telegram_scanner::ScannerRegistry>>()
    {
        let registry = registry.inner().clone();
        let acct = account_id.to_string();
        tauri::async_runtime::spawn(async move { registry.forget(&acct).await });
    }
}

#[derive(Debug, Clone)]
struct NotificationBypassPrefs {
    global_dnd: bool,
    muted_accounts: HashSet<String>,
    bypass_when_focused: bool,
    focused_account: Option<String>,
}

impl Default for NotificationBypassPrefs {
    fn default() -> Self {
        Self {
            global_dnd: false,
            muted_accounts: HashSet::new(),
            // Match the existing UI copy: focused account may suppress toast.
            bypass_when_focused: true,
            focused_account: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NotificationBypassPrefsPayload {
    pub global_dnd: bool,
    pub muted_accounts: Vec<String>,
    pub bypass_when_focused: bool,
}

impl From<&NotificationBypassPrefs> for NotificationBypassPrefsPayload {
    fn from(value: &NotificationBypassPrefs) -> Self {
        let mut muted_accounts = value.muted_accounts.iter().cloned().collect::<Vec<_>>();
        muted_accounts.sort();
        Self {
            global_dnd: value.global_dnd,
            muted_accounts,
            bypass_when_focused: value.bypass_when_focused,
        }
    }
}

/// Title prefix applied to every OS toast fired from an embedded webview.
/// Matches `openhuman_core::webview_notifications::OPENHUMAN_TITLE_PREFIX`
/// — kept inline here so the shell crate doesn't take a build-time dep on
/// the core library. Disambiguates from natively-installed apps (Slack,
/// Discord, Gmail desktop) firing the same message twice.
const OPENHUMAN_TITLE_PREFIX: &str = "OpenHuman: ";

fn slack_scanner_enabled() -> bool {
    std::env::var("OPENHUMAN_DISABLE_SLACK_SCANNER")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            !(v == "1" || v == "true" || v == "yes" || v == "on")
        })
        .unwrap_or(true)
}

/// Serialised fire-event payload shipped to the frontend over the
/// `webview-notification:fired` Tauri event. Carries `account_id` +
/// `provider` so the React side can route a subsequent click back to
/// the originating webview via Redux.
#[derive(Debug, Clone, Serialize)]
struct WebviewNotificationFired {
    account_id: String,
    provider: String,
    title: String,
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
}

/// Linux: one worker thread + bounded queue so a burst of toasts does not
/// spawn unbounded `std::thread` handles (each would block in `wait_for_action`).
#[cfg(target_os = "linux")]
const LINUX_NOTIFY_QUEUE_CAP: usize = 16;

#[cfg(target_os = "linux")]
static LINUX_NOTIFY_TX: OnceLock<std::sync::mpsc::SyncSender<Box<dyn FnOnce() + Send>>> =
    OnceLock::new();

#[cfg(target_os = "linux")]
fn enqueue_linux_notification(job: Box<dyn FnOnce() + Send>) {
    let tx = LINUX_NOTIFY_TX.get_or_init(|| {
        let (tx, rx) = sync_channel::<Box<dyn FnOnce() + Send>>(LINUX_NOTIFY_QUEUE_CAP);
        std::thread::Builder::new()
            .name("openhuman-linux-notify".to_string())
            .spawn(move || {
                while let Ok(j) = rx.recv() {
                    j();
                }
            })
            .expect("spawn openhuman-linux-notify");
        tx
    });
    if let Err(e) = tx.try_send(job) {
        log::warn!(
            "[notify-cef] linux notification queue full (cap={}), dropping toast: {}",
            LINUX_NOTIFY_QUEUE_CAP,
            e
        );
    }
}

/// Translate a `tauri-runtime-cef` notification payload into a native OS
/// toast via `tauri-plugin-notification`, and mirror the fire to the
/// React frontend so it can drive click-to-focus routing.
///
/// Gated on the runtime `NotificationSettings` flag (OFF by default) so
/// v1 ships the plumbing without surprising users with a toast storm the
/// first time they open a busy Slack tab.
fn forward_native_notification<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    provider: &str,
    payload: &tauri_runtime_cef::notification::NotificationPayload,
) {
    if let Some(state) = app.try_state::<WebviewAccountsState>() {
        let prefs = state.notification_bypass.lock().unwrap().clone();
        if prefs.global_dnd {
            log::debug!(
                "[notify-bypass][{}] suppressed global_dnd provider={}",
                account_id,
                provider
            );
            return;
        }
        if prefs.muted_accounts.contains(account_id) {
            log::debug!(
                "[notify-bypass][{}] suppressed muted_account provider={}",
                account_id,
                provider
            );
            return;
        }
        if prefs.bypass_when_focused && prefs.focused_account.as_deref() == Some(account_id) {
            log::debug!(
                "[notify-bypass][{}] suppressed focused_account provider={}",
                account_id,
                provider
            );
            return;
        }
    }

    // Feature flag — bail early when the user hasn't opted in.
    if let Some(settings) =
        app.try_state::<crate::notification_settings::NotificationSettingsState>()
    {
        if !settings.enabled() {
            log::debug!(
                "[notify-cef][{}] suppressed (feature flag off) provider={}",
                account_id,
                provider
            );
            return;
        }
    }

    let provider_label = provider_display_name(provider);
    let raw_title = payload.title.as_str().trim();
    let notify_title = if raw_title.is_empty() {
        format!("{OPENHUMAN_TITLE_PREFIX}{provider_label}")
    } else {
        format!("{OPENHUMAN_TITLE_PREFIX}{provider_label} — {raw_title}")
    };
    let body = payload.body.as_deref().unwrap_or("");
    log::info!(
        "[notify-cef][{}] source={:?} tag={:?} silent={} title_chars={} body_chars={}",
        account_id,
        payload.source,
        payload.tag,
        payload.silent,
        raw_title.chars().count(),
        body.chars().count()
    );
    log::debug!("[notify-cef][{}] raw_title={:?}", account_id, raw_title);

    // Mirror to the frontend BEFORE firing the OS toast so the Redux
    // store has the routing context ready by the time the user clicks.
    let fired = WebviewNotificationFired {
        account_id: account_id.to_string(),
        provider: provider.to_string(),
        title: notify_title.clone(),
        body: body.to_string(),
        tag: payload.tag.clone(),
    };
    if let Err(err) = app.emit("webview-notification:fired", &fired) {
        log::warn!(
            "[notify-cef][{}] emit webview-notification:fired failed: {}",
            account_id,
            err
        );
    }

    // Respect the Web Notification `silent` flag — the mirror event above
    // still updates the in-app notification center, but the OS toast is
    // suppressed so the user is not audibly/visually interrupted for
    // notifications the page explicitly marked as silent.
    if payload.silent {
        log::debug!(
            "[notify-cef][{}] silent=true, suppressing OS toast",
            account_id
        );
        return;
    }

    // Fire the OS toast and wire a click callback that emits `notification:click`
    // so the frontend can bring the originating account into focus.
    //
    // macOS: mac-notification-sys blocks in wait_for_click mode — run on a
    //        blocking thread so the async executor is not stalled.
    // Linux: notify_rust's wait_for_action hooks D-Bus action delivery.
    // Windows: no click callback available; fall back to fire-and-forget.
    let acct_for_click = account_id.to_string();
    let prov_for_click = provider.to_string();
    let app_for_click = app.clone();

    #[cfg(target_os = "macos")]
    {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Each `wait_for_click` thread blocks at ~100% CPU until the user
        // clicks or the toast auto-dismisses. Under notification bursts this
        // can pin many cores; cap concurrent click-wait threads and fall back
        // to fire-and-forget (no click callback) once the budget is reached.
        const MAX_CLICK_WAIT_THREADS: usize = 8;
        static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

        let title_c = notify_title.clone();
        let body_c = body.to_string();
        let app_id = app.config().identifier.clone();
        let prev = IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        if prev >= MAX_CLICK_WAIT_THREADS {
            IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
            log::debug!(
                "[notify-cef][{}] click-wait budget exhausted ({}), firing without click callback",
                account_id,
                prev
            );
            std::thread::spawn(move || {
                let _ = mac_notification_sys::set_application(if tauri::is_dev() {
                    "com.apple.Terminal"
                } else {
                    &app_id
                });
                use mac_notification_sys::Notification as MacNotif;
                let mut n = MacNotif::new();
                n.title(&title_c).message(&body_c);
                let _ = n.send();
            });
            return;
        }

        std::thread::spawn(move || {
            struct Guard;
            impl Drop for Guard {
                fn drop(&mut self) {
                    IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
                }
            }
            let _guard = Guard;

            let _ = mac_notification_sys::set_application(if tauri::is_dev() {
                "com.apple.Terminal"
            } else {
                &app_id
            });
            use mac_notification_sys::{Notification as MacNotif, NotificationResponse};
            let t = title_c;
            let b = body_c;
            let mut n = MacNotif::new();
            n.title(&t).message(&b).wait_for_click(true);
            match n.send() {
                Ok(NotificationResponse::Click) | Ok(NotificationResponse::ActionButton(_)) => {
                    log::info!(
                        "[notify-click][{}] clicked provider={}",
                        acct_for_click,
                        prov_for_click
                    );
                    if let Err(e) = app_for_click.emit(
                        "notification:click",
                        serde_json::json!({
                            "account_id": acct_for_click,
                            "provider": prov_for_click,
                        }),
                    ) {
                        log::warn!(
                            "[notify-click][{}] emit notification:click failed: {}",
                            acct_for_click,
                            e
                        );
                    }
                }
                Ok(other) => {
                    log::info!("[notify-click][{}] response={:?}", acct_for_click, other);
                }
                Err(e) => {
                    log::warn!("[notify-click][{}] send error: {}", acct_for_click, e);
                }
            }
        });
    }

    #[cfg(target_os = "linux")]
    {
        let title_c = notify_title.clone();
        let body_c = body.to_string();
        enqueue_linux_notification(Box::new(move || {
            let t = title_c;
            let b = body_c;
            let mut n = notify_rust::Notification::new();
            n.summary(&t).body(&b);
            match n.show() {
                Ok(handle) => {
                    handle.wait_for_action(|action| {
                        // "__closed" is the synthetic dismiss action; skip it.
                        if action != "__closed" && !action.is_empty() {
                            log::info!(
                                "[notify-click][{}] action={} provider={}",
                                acct_for_click,
                                action,
                                prov_for_click
                            );
                            if let Err(e) = app_for_click.emit(
                                "notification:click",
                                serde_json::json!({
                                    "account_id": acct_for_click,
                                    "provider": prov_for_click,
                                }),
                            ) {
                                log::warn!(
                                    "[notify-click][{}] emit notification:click failed: {}",
                                    acct_for_click,
                                    e
                                );
                            }
                        }
                    });
                }
                Err(e) => {
                    log::warn!("[notify-click][{}] show failed: {}", acct_for_click, e);
                }
            }
        }));
    }

    #[cfg(windows)]
    {
        let mut builder = app.notification().builder().title(&notify_title);
        if !body.is_empty() {
            builder = builder.body(body);
        }
        if let Err(e) = builder.show() {
            log::warn!(
                "[notify-cef][{}] notification show failed: {}",
                account_id,
                e
            );
        }
    }
}

pub(crate) fn forward_synthetic_notification<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    provider: &str,
    title: impl Into<String>,
    body: impl Into<String>,
) {
    let payload = tauri_runtime_cef::notification::NotificationPayload {
        source: tauri_runtime_cef::notification::NotificationSource::Window,
        title: title.into(),
        body: Some(body.into()),
        icon: None,
        tag: None,
        silent: false,
        origin: format!("synthetic://{}", provider),
    };
    forward_native_notification(app, account_id, provider, &payload);
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Deserialize)]
pub struct OpenArgs {
    pub account_id: String,
    pub provider: String,
    /// Optional URL override (debug tooling) — falls back to `provider_url`.
    pub url: Option<String>,
    pub bounds: Option<Bounds>,
}

#[derive(Debug, Deserialize)]
pub struct BoundsArgs {
    pub account_id: String,
    pub bounds: Bounds,
}

#[derive(Debug, Deserialize)]
pub struct AccountIdArgs {
    pub account_id: String,
}

#[derive(Debug, Deserialize)]
pub struct RecipeEventArgs {
    pub account_id: String,
    pub provider: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebviewEvent {
    pub account_id: String,
    pub provider: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub ts: Option<i64>,
}

/// Strip query string and fragment from a URL before emitting to the log.
/// Provider URLs occasionally embed auth material (Telegram WebApp data,
/// OAuth callback codes, sometimes session tokens) and we don't want those
/// to land in the long-lived shell log file. Returns the original input on
/// parse failure so we still surface *something* useful for debugging.
fn redact_url_for_log(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(mut u) => {
            u.set_query(None);
            u.set_fragment(None);
            u.to_string()
        }
        Err(_) => {
            // Fallback: drop everything from the first '?' or '#'.
            raw.split(['?', '#']).next().unwrap_or(raw).to_string()
        }
    }
}

/// Grow the first-cold-open webview back to its full requested bounds and
/// notify the frontend once the page is actually loaded. Called from three
/// signals (native `WebviewBuilder::on_page_load`, CDP `Page.loadEventFired`,
/// and the 15 s watchdog).
///
/// Timeout is a non-terminal state: we emit `webview-account:load{state:
/// "timeout"}` so the frontend can show retry/help UI, but we deliberately do
/// NOT reveal or mark the account as loaded yet. If a later `finished` signal
/// arrives, that call still reveals and emits `state:"finished"`.
///
/// Resetting the terminal loaded marker happens in `webview_account_close` /
/// `webview_account_purge` so a reopen fires again.
///
/// Doing the `set_size` server-side (instead of waiting for the frontend to
/// invoke `webview_account_reveal`) avoids an extra IPC round-trip and the
/// brief blank frame that would otherwise sit between the load event and
/// the frontend's reveal call.
pub(crate) fn emit_load_finished<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    state: &str,
    url: &str,
) {
    let Some(app_state) = app.try_state::<WebviewAccountsState>() else {
        // No state => emit anyway so the frontend doesn't hang; best-effort.
        log::warn!(
            "[webview-accounts][{}] WebviewAccountsState missing — emitting without reveal",
            account_id
        );
        let _ = app.emit(
            "webview-account:load",
            serde_json::json!({"account_id": account_id, "state": state, "url": url}),
        );
        return;
    };

    if state == "timeout" {
        // If we've already observed a terminal load, ignore late watchdogs.
        let already_loaded = app_state
            .loaded_accounts
            .lock()
            .unwrap()
            .contains(account_id);
        if already_loaded {
            log::debug!(
                "[webview-accounts][{}] timeout deduped after terminal load url={}",
                account_id,
                url
            );
            return;
        }

        log::info!(
            "[webview-accounts][{}] load timeout event url={}",
            account_id,
            redact_url_for_log(url)
        );
        if let Err(err) = app.emit(
            "webview-account:load",
            serde_json::json!({
                "account_id": account_id,
                "state": state,
                "url": url,
            }),
        ) {
            log::warn!(
                "[webview-accounts][{}] emit webview-account:load(timeout) failed: {}",
                account_id,
                err
            );
        }
        return;
    }

    let is_first_terminal = app_state
        .loaded_accounts
        .lock()
        .unwrap()
        .insert(account_id.to_string());
    if !is_first_terminal {
        log::debug!(
            "[webview-accounts][{}] load event deduped state={} url={}",
            account_id,
            state,
            url
        );
        return;
    }

    // Restore the webview to its full requested size. The spawn path created
    // it at 1×1 so the React loading spinner wasn't covered; now that the page
    // is painted we can grow it into the placeholder rect.
    let label = app_state.inner.lock().unwrap().get(account_id).cloned();
    let bounds = app_state
        .requested_bounds
        .lock()
        .unwrap()
        .get(account_id)
        .copied();
    match (label, bounds) {
        (Some(label), Some(b)) => {
            if let Some(wv) = app.get_webview(&label) {
                if let Err(e) = wv.set_size(LogicalSize::new(b.width, b.height)) {
                    log::warn!(
                        "[webview-accounts][{}] reveal set_size failed: {}",
                        account_id,
                        e
                    );
                }
                if let Err(e) = wv.set_position(LogicalPosition::new(b.x, b.y)) {
                    log::warn!(
                        "[webview-accounts][{}] reveal set_position failed: {}",
                        account_id,
                        e
                    );
                }
                let _ = wv.show();
                log::info!(
                    "[webview-accounts][{}] revealed label={} bounds={:?} state={}",
                    account_id,
                    label,
                    b,
                    state
                );
            } else {
                log::warn!(
                    "[webview-accounts][{}] reveal: webview {} missing",
                    account_id,
                    label
                );
            }
        }
        _ => {
            log::info!(
                "[webview-accounts][{}] reveal skipped (account closed before load) state={}",
                account_id,
                state
            );
        }
    }

    // Redact the URL in the log: providers like Telegram (`#tgWebAppData=…`)
    // and OAuth callbacks embed auth material in the query/fragment. The full
    // URL still flows to the frontend listener over the Tauri event so any
    // consumer that needs it has access; we just don't persist it to the
    // shell's log file.
    log::info!(
        "[webview-accounts][{}] load event state={} url={}",
        account_id,
        state,
        redact_url_for_log(url)
    );
    if let Err(err) = app.emit(
        "webview-account:load",
        serde_json::json!({
            "account_id": account_id,
            "state": state,
            "url": url,
        }),
    ) {
        log::warn!(
            "[webview-accounts][{}] emit webview-account:load failed: {}",
            account_id,
            err
        );
    }
}

/// Reject any `account_id` that isn't strictly `[A-Za-z0-9_-]+`. The ID comes
/// from IPC (React shell, but also from injected recipe code running inside
/// third-party origins via `webview_recipe_event`), so treat it as untrusted.
/// Enforcing this early prevents `../` sequences from escaping the per-account
/// data directory in `data_directory_for` (which feeds `create_dir_all` and
/// `remove_dir_all`).
fn sanitize_account_id(account_id: &str) -> Result<&str, String> {
    if account_id.is_empty()
        || !account_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("invalid account_id: {account_id:?}"));
    }
    Ok(account_id)
}

fn label_for(account_id: &str) -> String {
    // Webview labels must be alphanumeric + `-` / `_`. Callers that reached
    // here without first going through `sanitize_account_id` still get a
    // defensively-scrubbed label so invalid characters never reach the
    // tauri webview-label parser.
    let safe: String = account_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("acct_{}", safe)
}

fn data_directory_for<R: Runtime>(app: &AppHandle<R>, account_id: &str) -> Result<PathBuf, String> {
    // Guard against path traversal — `account_id` is joined into a filesystem
    // path that is later passed to `create_dir_all` / `remove_dir_all`.
    let account_id = sanitize_account_id(account_id)?;
    let base = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("app_local_data_dir: {e}"))?;
    Ok(base.join("webview_accounts").join(account_id))
}

/// Produce the `initialization_script` payload for this webview.
///
/// Empty for the 5 migrated providers (whatsapp, telegram, slack, discord,
/// browserscan) — they load with ZERO injected JS; their scraping runs via
/// CDP, and the per-account CDP session opener (`cdp::session`) injects the
/// notification-permission shim via `Page.addScriptToEvaluateOnNewDocument`
/// before the real provider URL loads. The 3 deferred providers (gmail,
/// linkedin, google-meet) still get the JS recipe bridge.
fn build_init_script(account_id: &str, provider: &str) -> String {
    let Some(recipe_js) = provider_recipe_js(provider) else {
        return String::new();
    };
    let ctx = serde_json::json!({
        "accountId": account_id,
        "provider": provider,
    });
    format!(
        "window.__OPENHUMAN_RECIPE_CTX__ = {ctx};\n\n{runtime}\n\n{recipe}\n",
        ctx = ctx,
        runtime = RUNTIME_JS,
        recipe = recipe_js
    )
}

/// Spawn (or focus) the embedded webview for an account.
#[tauri::command]
pub async fn webview_account_open<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, WebviewAccountsState>,
    args: OpenArgs,
) -> Result<String, String> {
    let label = label_for(&args.account_id);
    log::info!(
        "[webview-accounts] open account_id={} provider={} label={}",
        args.account_id,
        args.provider,
        label
    );

    // Reject unknown providers early. `provider_url` already errors when
    // no URL override is supplied; the `provider_is_supported` check
    // additionally gates custom-URL overrides so an arbitrary provider
    // string can't ride in via the debug `url` field.
    if !provider_is_supported(&args.provider) {
        return Err(format!("unknown provider: {}", args.provider));
    }
    let real_url_str = args
        .url
        .as_deref()
        .or_else(|| provider_url(&args.provider))
        .ok_or_else(|| format!("no url for provider: {}", args.provider))?
        .to_string();
    // Validate the real URL up front — otherwise a malformed debug
    // `args.url` would only fail later inside the async CDP session
    // loop, which is much harder to surface to the caller. The parsed
    // Url also feeds `scanner_url_prefix` so scanners match on the
    // actual origin the user navigated to (honoring debug overrides).
    let real_url: Url = real_url_str
        .parse()
        .map_err(|e| format!("invalid provider url {real_url_str}: {e}"))?;
    // Scanner target-match uses `url.starts_with(prefix)`, so the
    // prefix needs to be the ORIGIN (scheme + host), not the full URL
    // — same-host intra-app navigations must keep matching after the
    // initial load.
    let scanner_url_prefix = format!("{}/", real_url.origin().ascii_serialization());
    let skip_cdp_for_debug = args.provider == "slack" && !slack_scanner_enabled();
    // We normally open the webview at a tiny placeholder URL so the CDP
    // session opener can attach and inject the notification-permission
    // shim (see `cdp/session.rs`) BEFORE the real provider URL loads;
    // without it Slack/Gmail surface in-app "enable notifications"
    // banners. For Slack debug sessions we allow opting out via
    // `OPENHUMAN_DISABLE_SLACK_SCANNER=1`, which also skips the long-lived
    // CDP session so external DevTools can attach cleanly.
    let initial_url_str = if skip_cdp_for_debug {
        real_url_str.clone()
    } else {
        cdp::placeholder_url(&args.account_id)
    };
    let initial_url: Url = initial_url_str
        .parse()
        .map_err(|e| format!("invalid initial url {initial_url_str}: {e}"))?;

    // If a webview for this account already exists, just reposition / show.
    {
        let map = state.inner.lock().unwrap();
        if let Some(existing_label) = map.get(&args.account_id).cloned() {
            drop(map);
            if let Some(existing) = app.get_webview(&existing_label) {
                if let Some(b) = args.bounds {
                    let _ = existing.set_position(LogicalPosition::new(b.x, b.y));
                    let _ = existing.set_size(LogicalSize::new(b.width, b.height));
                    state
                        .requested_bounds
                        .lock()
                        .unwrap()
                        .insert(args.account_id.clone(), b);
                }
                let _ = existing.show();
                log::info!(
                    "[webview-accounts] reused existing label={} for account={}",
                    existing_label,
                    args.account_id
                );
                // Warm re-open: the page is already painted, so skip the
                // loading overlay cycle and tell the frontend to go straight
                // to `open`. We bypass `emit_load_finished` because the
                // `loaded_accounts` dedup set would swallow the emit after
                // the first cold open of this account.
                let reuse_url = existing.url().map(|u| u.to_string()).unwrap_or_default();
                if let Err(err) = app.emit(
                    "webview-account:load",
                    serde_json::json!({
                        "account_id": args.account_id,
                        "state": "reused",
                        "url": reuse_url,
                    }),
                ) {
                    log::warn!(
                        "[webview-accounts][{}] emit reused event failed: {}",
                        args.account_id,
                        err
                    );
                }
                return Ok(existing_label);
            }
            // Stale entry — fall through and rebuild
            log::warn!(
                "[webview-accounts] stale label {} found for account {}, rebuilding",
                existing_label,
                args.account_id
            );
        }
    }

    // Grab the raw Window (not WebviewWindow) so `add_child` works even
    // after we've attached sibling webviews — `get_webview_window` checks
    // `is_webview_window()` which flips to false once a window has more
    // than one webview.
    let parent_window = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    let data_dir = data_directory_for(&app, &args.account_id)?;
    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        log::warn!(
            "[webview-accounts] failed to create data dir {}: {}",
            data_dir.display(),
            err
        );
    }

    let init_script = build_init_script(&args.account_id, &args.provider);

    let mut builder = WebviewBuilder::new(label.clone(), WebviewUrl::External(initial_url))
        .data_directory(data_dir);
    if !init_script.is_empty() {
        builder = builder.initialization_script(&init_script);
    }

    // Keep link clicks that leave the provider's host set in the OS
    // browser, not the embedded webview. Same-host navigations (including
    // OAuth hops to accounts.google.com etc., which we pre-declare per
    // provider) stay in-app. Provider-specific native-app deep links
    // (`zoomus://`, `zoommtg://`, …) are rewritten to the web-client URL
    // and re-navigated in-app so meetings don't bounce out.
    let nav_provider = args.provider.clone();
    let nav_app = app.clone();
    let nav_label = label.clone();
    let nav_account_id = args.account_id.clone();
    builder = builder.on_navigation(move |url| {
        // Notify the frontend on every committed navigation. The
        // `webview-account:load` event is dedup'd per cold open, so it
        // can't be used to spot post-login redirects (e.g. Gmail's
        // accounts.google.com → mail.google.com hop). Frontends that
        // care about live URL transitions — onboarding's auto-detect
        // for "user finished signing in", for instance — listen here.
        if let Err(err) = nav_app.emit(
            "webview-account:navigate",
            serde_json::json!({
                "account_id": nav_account_id,
                "provider": nav_provider,
                "url": redact_navigation_url(url),
            }),
        ) {
            log::debug!(
                "[webview-accounts] emit webview-account:navigate failed: {}",
                err
            );
        }
        // Google Meet: when Google's edge SSR-redirects the post-account-
        // picker URL to `workspace.google.com/products/meet/...` (the
        // marketing landing page), `workspace.google.com` matches the
        // bare `google.com` suffix in `provider_allowed_hosts` so
        // `url_is_internal` would commit the navigation and the user
        // would land on the Workspace marketing page instead of Meet.
        // Catch this here and replace the parent URL with the canonical
        // Meet entry point so the embedded view stays on the app.
        if nav_provider == "google-meet" {
            if let Some(host) = url.host_str() {
                if host == "workspace.google.com" || host.ends_with(".workspace.google.com") {
                    log::info!(
                        "[webview-accounts] gmeet workspace marketing redirect intercepted ({}); rewriting parent to https://meet.google.com/",
                        url
                    );
                    let app = nav_app.clone();
                    let label = nav_label.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(wv) = app.get_webview(&label) {
                            if let Ok(target) = Url::parse("https://meet.google.com/") {
                                if let Err(e) = wv.navigate(target) {
                                    log::warn!(
                                        "[webview-accounts] gmeet workspace rewrite navigate failed label={} err={}",
                                        label,
                                        e
                                    );
                                }
                            }
                        }
                    });
                    return false;
                }
            }
        }
        if let Some(rewritten) = rewrite_provider_deep_link(&nav_provider, url) {
            log::info!(
                "[webview-accounts] deep-link rewrite {} → {} (provider={})",
                url,
                rewritten,
                nav_provider
            );
            let app = nav_app.clone();
            let label = nav_label.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(wv) = app.get_webview(&label) {
                    if let Err(e) = wv.navigate(rewritten) {
                        log::warn!(
                            "[webview-accounts] post-rewrite navigate failed label={} err={}",
                            label,
                            e
                        );
                    }
                }
            });
            return false;
        }
        if url_is_internal(&nav_provider, url) {
            true
        } else {
            // Suppress provider native-desktop-app deep-link schemes that
            // we don't know how to rewrite. macOS would otherwise hand
            // these to the native provider app — `slack://magic-login/…`
            // signs the native Slack app into the workspace, breaking
            // embedded-webview isolation (#1074). The web flow's HTTPS
            // fallback handles sign-in without the deep link.
            if is_provider_native_deep_link_scheme(url.scheme()) {
                log::warn!(
                    "[webview-accounts] suppressing native-app deep-link scheme={} url={} (would breach workspace isolation)",
                    url.scheme(),
                    redact_navigation_url(url)
                );
                return false;
            }
            let target = unwrap_provider_redirect(url)
                .map(|u| u.to_string())
                .unwrap_or_else(|| url.to_string());
            if target != url.as_str() {
                log::info!(
                    "[webview-accounts] external navigation {} → (unwrapped) {} → system browser",
                    url,
                    target
                );
            } else {
                log::info!(
                    "[webview-accounts] external navigation {} → system browser",
                    url
                );
            }
            open_in_system_browser(&target);
            false
        }
    });

    // Cmd/Ctrl-click and `target="_blank"` / `window.open(...)` trigger a
    // new-window request. Default policy: deny and hand the URL to the
    // system browser — matches user intent of "open in new tab outside
    // the app".
    //
    // Exception: some providers (Slack Huddles) spawn popups via
    // `window.open()` and abort the flow if the return value is falsey.
    // For those URLs we allow CEF's default popup handling so an in-app
    // child window opens and the caller gets a real window handle.
    let popup_provider = args.provider.clone();
    let popup_app = app.clone();
    let popup_label = label.clone();
    builder = builder.on_new_window(move |url, _features| {
        if let Some(rewritten) = rewrite_provider_deep_link(&popup_provider, &url) {
            log::info!(
                "[webview-accounts] new-window deep-link rewrite {} → {} (provider={})",
                url,
                rewritten,
                popup_provider
            );
            let app = popup_app.clone();
            let label = popup_label.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(wv) = app.get_webview(&label) {
                    if let Err(e) = wv.navigate(rewritten) {
                        log::warn!(
                            "[webview-accounts] post-rewrite navigate (popup) failed label={} err={}",
                            label,
                            e
                        );
                    }
                }
            });
            return NewWindowResponse::Deny;
        }
        if let Some(target) = popup_should_navigate_parent(&popup_provider, &url) {
            log::info!(
                "[webview-accounts] new-window {} → navigate parent (provider={})",
                redact_navigation_url(&url),
                popup_provider
            );
            let app = popup_app.clone();
            let label = popup_label.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(wv) = app.get_webview(&label) {
                    if let Err(e) = wv.navigate(target) {
                        log::warn!(
                            "[webview-accounts] popup→parent navigate failed label={} err={}",
                            label,
                            e
                        );
                    }
                }
            });
            return NewWindowResponse::Deny;
        }
        if popup_should_stay_in_app(&popup_provider, &url) {
            log::info!(
                "[webview-accounts] new-window request {} → in-app popup (provider={})",
                url,
                popup_provider
            );
            NewWindowResponse::Allow
        } else {
            // Suppress provider native-desktop-app deep-link schemes that
            // we don't know how to rewrite (matches the on_navigation
            // fallback). Without this, a `slack://...` popup would land
            // in the native Slack app via macOS's URL handler and
            // breach embedded-webview workspace isolation (#1074).
            if is_provider_native_deep_link_scheme(url.scheme()) {
                log::warn!(
                    "[webview-accounts] suppressing native-app deep-link scheme={} url={} (would breach workspace isolation)",
                    url.scheme(),
                    redact_navigation_url(&url)
                );
                return NewWindowResponse::Deny;
            }
            let target = unwrap_provider_redirect(&url)
                .map(|u| u.to_string())
                .unwrap_or_else(|| url.to_string());
            if target != url.as_str() {
                log::info!(
                    "[webview-accounts] new-window request {} → (unwrapped) {} → system browser",
                    url,
                    target
                );
            } else {
                log::info!(
                    "[webview-accounts] new-window request {} → system browser",
                    url
                );
            }
            open_in_system_browser(&target);
            NewWindowResponse::Deny
        }
    });

    // Enable devtools on child webviews in debug builds only so recipe
    // diagnostics and IndexedDB state can be inspected. Access on macOS is via
    //   Safari → Develop → <App name> → <webview label>
    // (the parent Tauri window's right-click "Inspect" does not propagate
    // into child webviews on WKWebView). In release builds we leave CDP off
    // so third-party-site webviews are not remotely inspectable.
    if cfg!(debug_assertions) {
        builder = builder.devtools(true);
    }

    // Wire the native page-load signal and forward only *usable* load
    // completions to `emit_load_finished`:
    //   - skip placeholder `about:blank#openhuman-acct-*` commits (otherwise
    //     we reveal a blank viewport before real content arrives),
    //   - treat Chromium network error pages (`chrome-error://…`) as timeout
    //     signals so frontend shows retry/help UI instead of the dino page.
    //
    // Real provider commits still emit `finished`. Dedup against CDP
    // `Page.loadEventFired` + watchdog happens in `emit_load_finished`.
    let page_load_app = app.clone();
    let page_load_account_id = args.account_id.clone();
    let page_load_placeholder_fragment = format!("#{}", cdp::placeholder_marker(&args.account_id));
    let page_load_real_url = real_url_str.clone();
    builder = builder.on_page_load(move |_webview, payload| {
        if !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
            return;
        }
        let url = payload.url();
        if url.scheme() == "data" {
            return;
        }
        if !skip_cdp_for_debug && url.as_str().ends_with(&page_load_placeholder_fragment) {
            log::debug!(
                "[webview-accounts][{}] skipping placeholder native-finished url={}",
                page_load_account_id,
                redact_url_for_log(url.as_str())
            );
            return;
        }
        if url.scheme() == "chrome-error" {
            emit_load_finished(
                &page_load_app,
                &page_load_account_id,
                "timeout",
                &page_load_real_url,
            );
            return;
        }
        emit_load_finished(
            &page_load_app,
            &page_load_account_id,
            "finished",
            url.as_str(),
        );
    });

    let bounds = args.bounds.unwrap_or(Bounds {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    });

    // Park the webview off-screen during its first page load so the React
    // placeholder's loading spinner is not covered by the native CEF subview.
    // `webview_account_reveal` (invoked from the frontend after the load event
    // arrives, or by the 15 s watchdog) moves it back to `bounds` + shows it.
    // We use positive coords well below the parent window rather than large
    // negative values so multi-monitor layouts stay well-defined.
    //
    // Warm-open reuse (when a webview already exists for this account) earlier
    // in this function returns before we get here, so existing webviews keep
    // their current position — we only off-screen the first cold spawn.
    // Spawn strategy: keep the webview at the caller's requested position
    // but shrink the initial size to 1×1 under CEF so the native subview
    // doesn't paint over the React loading spinner. `webview_account_reveal`
    // grows it back to `bounds.width × bounds.height` once the page-loaded
    // signal arrives.
    //
    // Why not move off-screen: moving the NSView after a cold CEF spawn on
    // macOS sometimes leaves the page painted but not repainted at the new
    // origin, leaving the user looking at a blank viewport until they
    // reload. Keeping the position stable and only toggling size sidesteps
    // that repaint edge case while still keeping the webview visually
    // hidden (1 px under the overlay) during load.
    //
    // Warm-open reuse returned earlier in this function, so this only
    // affects the first cold spawn.
    let initial_size = if skip_cdp_for_debug {
        LogicalSize::new(bounds.width, bounds.height)
    } else {
        LogicalSize::new(1.0, 1.0)
    };
    let initial_position = LogicalPosition::new(bounds.x, bounds.y);

    // Remember the bounds the frontend wanted so `webview_account_reveal` has a
    // rect to restore to even if the frontend's bounds cache is empty (e.g.
    // after a page reload races the load event).
    state
        .requested_bounds
        .lock()
        .unwrap()
        .insert(args.account_id.clone(), bounds);
    // Defensive reset: if a prior close/purge was raced by a stale emit we
    // could still have the account marked as "already loaded". Clear here so
    // the fresh spawn is allowed to fire the first event again.
    state
        .loaded_accounts
        .lock()
        .unwrap()
        .remove(&args.account_id);

    let webview = parent_window
        .add_child(builder, initial_position, initial_size)
        .map_err(|e| format!("add_child failed: {e}"))?;

    log::info!(
        "[webview-accounts] spawned label={} requested_bounds={:?} initial_size={:?}",
        webview.label(),
        bounds,
        initial_size
    );

    state
        .inner
        .lock()
        .unwrap()
        .insert(args.account_id.clone(), label.clone());

    // Spawn the per-account CDP session opener: holds an attached session
    // for the lifetime of the webview so `Emulation.setUserAgentOverride`
    // (which reverts on detach) keeps applying, and drives the initial
    // Page.navigate from our placeholder URL to the real provider URL.
    // Also installs the `#openhuman-account-{id}` fragment the scanners
    // match on for multi-account disambiguation.
    // Spawn the per-account CDP session opener, replacing any prior
    // handle for this account (the old one would still be trying to
    // attach to a target that's been torn down).
    {
        if skip_cdp_for_debug {
            log::info!(
                "[webview-accounts] skipping CDP session via OPENHUMAN_DISABLE_SLACK_SCANNER for account={}",
                args.account_id
            );
        } else {
            let cdp::SpawnedSession { session, watchdog } =
                cdp::spawn_session(app.clone(), args.account_id.clone(), real_url_str.clone());
            if let Some(old) = state
                .cdp_sessions
                .lock()
                .unwrap()
                .insert(args.account_id.clone(), session)
            {
                old.abort();
            }
            if let Some(old) = state
                .load_watchdogs
                .lock()
                .unwrap()
                .insert(args.account_id.clone(), watchdog)
            {
                old.abort();
            }
        }
    }

    // For providers we know how to scrape via CDP, kick off the IndexedDB
    // scanner. CDP requires the CEF runtime's remote-debugging port.
    {
        // Prefix is derived from the validated real URL's origin above
        // so debug `args.url` overrides (alt hosts, localhost mirrors)
        // resolve correctly — previously we always used the static
        // `provider_url(...)` default even when the webview had
        // navigated elsewhere.
        if args.provider == "whatsapp" {
            let registry = app
                .try_state::<std::sync::Arc<crate::whatsapp_scanner::ScannerRegistry>>()
                .map(|s| s.inner().clone());
            if let Some(registry) = registry {
                let app_clone = app.clone();
                let acct = args.account_id.clone();
                let prefix = scanner_url_prefix.clone();
                tokio::spawn(async move {
                    registry.ensure_scanner(app_clone, acct, prefix).await;
                });
            } else {
                log::warn!("[webview-accounts] CDP ScannerRegistry not in app state");
            }
        } else if args.provider == "slack" {
            if slack_scanner_enabled() {
                let registry = app
                    .try_state::<std::sync::Arc<crate::slack_scanner::ScannerRegistry>>()
                    .map(|s| s.inner().clone());
                if let Some(registry) = registry {
                    let app_clone = app.clone();
                    let acct = args.account_id.clone();
                    let prefix = scanner_url_prefix.clone();
                    tokio::spawn(async move {
                        registry.ensure_scanner(app_clone, acct, prefix).await;
                    });
                } else {
                    log::warn!("[webview-accounts] slack ScannerRegistry not in app state");
                }
            } else {
                log::info!(
                    "[webview-accounts] slack scanner disabled via OPENHUMAN_DISABLE_SLACK_SCANNER for account={}",
                    args.account_id
                );
            }
        } else if args.provider == "telegram" {
            let registry = app
                .try_state::<std::sync::Arc<crate::telegram_scanner::ScannerRegistry>>()
                .map(|s| s.inner().clone());
            if let Some(registry) = registry {
                let app_clone = app.clone();
                let acct = args.account_id.clone();
                let prefix = scanner_url_prefix.clone();
                tokio::spawn(async move {
                    registry.ensure_scanner(app_clone, acct, prefix).await;
                });
            } else {
                log::warn!("[webview-accounts] telegram ScannerRegistry not in app state");
            }
        } else if args.provider == "discord" {
            // Discord MITM uses CDP `Network.*` to capture HTTP API calls
            // and gateway WebSocket frames — see `discord_scanner/mod.rs`.
            let registry = app
                .try_state::<std::sync::Arc<crate::discord_scanner::ScannerRegistry>>()
                .map(|s| s.inner().clone());
            if let Some(registry) = registry {
                let app_clone = app.clone();
                let acct = args.account_id.clone();
                let prefix = scanner_url_prefix.clone();
                tokio::spawn(async move {
                    registry.ensure_scanner(app_clone, acct, prefix).await;
                });
            } else {
                log::warn!("[webview-accounts] discord ScannerRegistry not in app state");
            }
        }

        // Browser Notification interception, native CEF path. The renderer
        // subprocess (cef-helper) has already replaced `window.Notification`
        // and `ServiceWorkerRegistration.prototype.showNotification` with
        // V8 native bindings that send a `"openhuman.notify"` ProcessMessage
        // to the browser process. `tauri-runtime-cef::notification::register`
        // installs a per-browser callback that the runtime invokes when that
        // IPC arrives. We need the CEF browser id to key the registration —
        // hence the `with_webview` downcast hop. The callback is dispatched
        // from a CEF thread, so keep work inside it short / non-blocking.
        let app_for_register = app.clone();
        let acct_for_register = args.account_id.clone();
        let provider_for_register = args.provider.clone();
        if let Err(err) = webview.with_webview(move |raw| {
            let Some(browser) = raw.downcast_ref::<cef::Browser>() else {
                log::warn!(
                    "[notify-cef] with_webview returned non-cef::Browser handle for account={} — skipping notification registration",
                    acct_for_register
                );
                return;
            };
            let browser_id = browser.identifier();
            if let Some(state) = app_for_register.try_state::<WebviewAccountsState>() {
                state
                    .browser_ids
                    .lock()
                    .unwrap()
                    .insert(acct_for_register.clone(), browser_id);
            }
            let acct_in_handler = acct_for_register.clone();
            let provider_in_handler = provider_for_register.clone();
            let app_in_handler = app_for_register.clone();
            tauri_runtime_cef::notification::register(browser_id, move |payload| {
                forward_native_notification(
                    &app_in_handler,
                    &acct_in_handler,
                    &provider_in_handler,
                    &payload,
                );
            });
            log::info!(
                "[notify-cef] registered handler account={} provider={} browser_id={}",
                acct_for_register,
                provider_for_register,
                browser_id
            );
        }) {
            log::warn!(
                "[notify-cef] with_webview dispatch failed for account={}: {}",
                args.account_id,
                err
            );
        }
    }

    Ok(label)
}

#[tauri::command]
pub async fn webview_account_close<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, WebviewAccountsState>,
    args: AccountIdArgs,
) -> Result<(), String> {
    let label_opt = state.inner.lock().unwrap().remove(&args.account_id);
    let Some(label) = label_opt else {
        log::debug!(
            "[webview-accounts] close: no webview for account {}",
            args.account_id
        );
        return Ok(());
    };
    if let Some(wv) = app.get_webview(&label) {
        if let Err(e) = wv.close() {
            log::warn!("[webview-accounts] close({label}) failed: {e}");
        }
    }
    teardown_account_scanners(&app, &args.account_id);
    if let Some(browser_id) = state.browser_ids.lock().unwrap().remove(&args.account_id) {
        tauri_runtime_cef::notification::unregister(browser_id);
        log::debug!(
            "[notify-cef] unregistered handler account={} browser_id={}",
            args.account_id,
            browser_id
        );
    }
    if let Some(task) = state.cdp_sessions.lock().unwrap().remove(&args.account_id) {
        task.abort();
        log::debug!(
            "[cdp-session] aborted session task for account={}",
            args.account_id
        );
    }
    if let Some(task) = state
        .load_watchdogs
        .lock()
        .unwrap()
        .remove(&args.account_id)
    {
        task.abort();
        log::debug!(
            "[webview-accounts] aborted load watchdog for account={}",
            args.account_id
        );
    }
    // Reset load-overlay bookkeeping so the next open of this account starts
    // with a fresh "not yet loaded" state.
    state
        .loaded_accounts
        .lock()
        .unwrap()
        .remove(&args.account_id);
    state
        .requested_bounds
        .lock()
        .unwrap()
        .remove(&args.account_id);
    log::info!("[webview-accounts] closed label={}", label);
    Ok(())
}

/// Close the webview AND wipe its on-disk `data_directory` so cookies,
/// storage and cached credentials are forgotten. Use this for the
/// user-initiated "logout" action — `webview_account_close` keeps the
/// data dir intact so the next open restores the session.
#[tauri::command]
pub async fn webview_account_purge<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, WebviewAccountsState>,
    args: AccountIdArgs,
) -> Result<(), String> {
    // Close first so the native webview releases its file handles before we
    // try to delete the data directory.
    let label_opt = state.inner.lock().unwrap().remove(&args.account_id);
    if let Some(label) = label_opt.as_ref() {
        if let Some(wv) = app.get_webview(label) {
            if let Err(e) = wv.close() {
                log::warn!("[webview-accounts] purge close({label}) failed: {e}");
            }
        }
    }

    teardown_account_scanners(&app, &args.account_id);
    if let Some(browser_id) = state.browser_ids.lock().unwrap().remove(&args.account_id) {
        tauri_runtime_cef::notification::unregister(browser_id);
        log::debug!(
            "[notify-cef] purge unregistered handler account={} browser_id={}",
            args.account_id,
            browser_id
        );
    }
    if let Some(task) = state.cdp_sessions.lock().unwrap().remove(&args.account_id) {
        task.abort();
        log::debug!(
            "[cdp-session] purge aborted session task for account={}",
            args.account_id
        );
    }
    if let Some(task) = state
        .load_watchdogs
        .lock()
        .unwrap()
        .remove(&args.account_id)
    {
        task.abort();
        log::debug!(
            "[webview-accounts] purge aborted load watchdog for account={}",
            args.account_id
        );
    }
    state
        .loaded_accounts
        .lock()
        .unwrap()
        .remove(&args.account_id);
    state
        .requested_bounds
        .lock()
        .unwrap()
        .remove(&args.account_id);

    let data_dir = data_directory_for(&app, &args.account_id)?;
    if data_dir.exists() {
        if let Err(err) = std::fs::remove_dir_all(&data_dir) {
            // WKWebView can keep handles open briefly after `close()` — log
            // and keep going rather than failing the logout outright.
            log::warn!(
                "[webview-accounts] purge remove_dir_all {} failed: {}",
                data_dir.display(),
                err
            );
        } else {
            log::info!("[webview-accounts] purged data dir {}", data_dir.display());
        }
    }

    log::info!(
        "[webview-accounts] purged account={} label={:?}",
        args.account_id,
        label_opt
    );
    Ok(())
}

#[tauri::command]
pub async fn webview_account_bounds<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, WebviewAccountsState>,
    args: BoundsArgs,
) -> Result<(), String> {
    let label_opt = state.inner.lock().unwrap().get(&args.account_id).cloned();
    let Some(label) = label_opt else {
        return Err(format!("no webview for account {}", args.account_id));
    };
    let wv = app
        .get_webview(&label)
        .ok_or_else(|| format!("webview {label} missing"))?;
    wv.set_position(LogicalPosition::new(args.bounds.x, args.bounds.y))
        .map_err(|e| format!("set_position: {e}"))?;
    wv.set_size(LogicalSize::new(args.bounds.width, args.bounds.height))
        .map_err(|e| format!("set_size: {e}"))?;
    log::trace!(
        "[webview-accounts] bounds label={} -> {:?}",
        label,
        args.bounds
    );
    // Keep the in-state bounds synced so `webview_account_reveal` has the
    // latest rect even if the frontend's own cache is cleared between the
    // `webview_account_open` call and the `webview-account:load` signal.
    state
        .requested_bounds
        .lock()
        .unwrap()
        .insert(args.account_id.clone(), args.bounds);
    Ok(())
}

/// Move an off-screen-spawned webview back to the frontend's desired rect and
/// show it. Invoked by the frontend when it receives the `webview-account:load`
/// event so the loading spinner is uncovered only after the page has painted.
///
/// Called as the final step of the first-open flow:
///   1. `webview_account_open` — CEF subview spawned off-screen
///   2. native `on_page_load` OR CDP `Page.loadEventFired` OR 15 s watchdog
///   3. frontend listener → `webview_account_reveal`
#[tauri::command]
pub async fn webview_account_reveal<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, WebviewAccountsState>,
    args: BoundsArgs,
) -> Result<(), String> {
    let label_opt = state.inner.lock().unwrap().get(&args.account_id).cloned();
    let Some(label) = label_opt else {
        // Reveal race: the webview was closed before the load event arrived.
        // Return Ok so the frontend doesn't surface an error.
        log::debug!(
            "[webview-accounts] reveal: no webview for account {}",
            args.account_id
        );
        return Ok(());
    };
    let wv = app
        .get_webview(&label)
        .ok_or_else(|| format!("webview {label} missing"))?;
    wv.set_position(LogicalPosition::new(args.bounds.x, args.bounds.y))
        .map_err(|e| format!("set_position: {e}"))?;
    wv.set_size(LogicalSize::new(args.bounds.width, args.bounds.height))
        .map_err(|e| format!("set_size: {e}"))?;
    wv.show().map_err(|e| format!("show: {e}"))?;
    state
        .requested_bounds
        .lock()
        .unwrap()
        .insert(args.account_id.clone(), args.bounds);
    log::info!(
        "[webview-accounts] revealed label={} -> {:?}",
        label,
        args.bounds
    );
    Ok(())
}

#[tauri::command]
pub async fn webview_account_hide<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, WebviewAccountsState>,
    args: AccountIdArgs,
) -> Result<(), String> {
    let label_opt = state.inner.lock().unwrap().get(&args.account_id).cloned();
    let Some(label) = label_opt else {
        return Ok(());
    };
    if let Some(wv) = app.get_webview(&label) {
        let _ = wv.hide();
        log::debug!("[webview-accounts] hide label={}", label);
    }
    Ok(())
}

#[tauri::command]
pub async fn webview_account_show<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, WebviewAccountsState>,
    args: AccountIdArgs,
) -> Result<(), String> {
    let label_opt = state.inner.lock().unwrap().get(&args.account_id).cloned();
    let Some(label) = label_opt else {
        return Ok(());
    };
    if let Some(wv) = app.get_webview(&label) {
        let _ = wv.show();
        log::debug!("[webview-accounts] show label={}", label);
    }
    Ok(())
}

/// Web-shape notification permission state used by frontend parity code.
/// Effectively granted because interception is handled in-app via CEF.
#[tauri::command]
pub fn webview_notification_permission_state() -> String {
    "granted".to_string()
}

/// Request notification permission and return web-shape state.
#[tauri::command]
pub fn webview_notification_permission_request() -> String {
    webview_notification_permission_state()
}

/// Enable/disable global DND for embedded webview OS toasts.
#[tauri::command]
pub fn webview_notification_set_dnd(
    state: tauri::State<'_, WebviewAccountsState>,
    enabled: bool,
) -> Result<(), String> {
    let mut prefs = state.notification_bypass.lock().unwrap();
    prefs.global_dnd = enabled;
    log::debug!("[notify-bypass] set global_dnd={enabled}");
    Ok(())
}

/// Mute/unmute a specific embedded account for OS toasts.
#[tauri::command]
pub fn webview_notification_mute_account(
    state: tauri::State<'_, WebviewAccountsState>,
    account_id: String,
    muted: bool,
) -> Result<(), String> {
    let account_id = sanitize_account_id(&account_id)?.to_string();
    let mut prefs = state.notification_bypass.lock().unwrap();
    if muted {
        prefs.muted_accounts.insert(account_id.clone());
    } else {
        prefs.muted_accounts.remove(&account_id);
    }
    log::debug!(
        "[notify-bypass] set muted account_id={} muted={}",
        account_id,
        muted
    );
    Ok(())
}

/// Return current bypass preferences for the settings UI.
#[tauri::command]
pub fn webview_notification_get_bypass_prefs(
    state: tauri::State<'_, WebviewAccountsState>,
) -> NotificationBypassPrefsPayload {
    let prefs = state.notification_bypass.lock().unwrap();
    NotificationBypassPrefsPayload::from(&*prefs)
}

/// Track which account is currently focused in the shell UI.
#[tauri::command]
pub fn webview_set_focused_account(
    state: tauri::State<'_, WebviewAccountsState>,
    account_id: Option<String>,
) -> Result<(), String> {
    let mut prefs = state.notification_bypass.lock().unwrap();
    prefs.focused_account = match account_id {
        Some(id) => Some(sanitize_account_id(&id)?.to_string()),
        None => None,
    };
    log::debug!(
        "[notify-bypass] set focused_account={}",
        prefs.focused_account.as_deref().unwrap_or("<none>")
    );
    Ok(())
}

/// Called from the injected runtime each time the recipe emits an event.
/// We forward to React via a Tauri event so the UI can render and persist.
#[tauri::command]
pub async fn webview_recipe_event<R: Runtime>(
    app: AppHandle<R>,
    webview: tauri::Webview<R>,
    args: RecipeEventArgs,
) -> Result<(), String> {
    // The event can only be trusted if the invoking webview is the
    // `acct_<account_id>` webview for the account in the payload. A
    // compromised renderer or a sibling child webview must not be able to
    // forge events for another account.
    let caller_label = webview.label().to_string();
    let expected_label = label_for(&args.account_id);
    if caller_label != expected_label {
        log::warn!(
            "[webview-accounts] recipe_event rejected: caller_label={} expected={} account={}",
            caller_label,
            expected_label,
            args.account_id
        );
        return Err("webview label does not match account_id".to_string());
    }
    log::debug!(
        "[webview-accounts] recipe_event account={} provider={} kind={}",
        args.account_id,
        args.provider,
        args.kind
    );
    if args.provider == "google-meet" {
        match args.kind.as_str() {
            "meet_call_started" => {
                let code = args
                    .payload
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                log::info!("[gmeet][{}] call_started code={}", args.account_id, code);
            }
            "meet_captions" => {
                let code = args
                    .payload
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let n = args
                    .payload
                    .get("captions")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                log::info!(
                    "[gmeet][{}] captions code={} rows={}",
                    args.account_id,
                    code,
                    n
                );
            }
            "meet_call_ended" => {
                let code = args
                    .payload
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let reason = args
                    .payload
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                log::info!(
                    "[gmeet][{}] call_ended code={} reason={}",
                    args.account_id,
                    code,
                    reason
                );
            }
            _ => {}
        }
    }
    if args.kind == "ingest" {
        if let Some(messages) = args.payload.get("messages").and_then(|v| v.as_array()) {
            log::info!(
                "[webview-accounts] ingest from acct_{}: {} messages",
                args.account_id,
                messages.len()
            );
        }
    } else if args.kind == "ws_message" {
        let direction = args
            .payload
            .get("direction")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let size = args
            .payload
            .get("size")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        log::trace!(
            "[webview-accounts][{}] ws {} {} bytes",
            args.account_id,
            direction,
            size
        );
    } else if args.kind == "log" {
        let level = args
            .payload
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("info");
        let msg = args
            .payload
            .get("msg")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match level {
            "warn" => log::warn!("[webview-accounts][{}] {}", args.account_id, msg),
            "error" => log::error!("[webview-accounts][{}] {}", args.account_id, msg),
            _ => log::info!("[webview-accounts][{}] {}", args.account_id, msg),
        }
    }

    if let Err(err) = post_provider_surfaces_event(&args).await {
        log::warn!(
            "[webview-accounts] provider_surfaces ingest failed account={} provider={} kind={}: {}",
            args.account_id,
            args.provider,
            args.kind,
            err
        );
    }

    let event = WebviewEvent {
        account_id: args.account_id,
        provider: args.provider,
        kind: args.kind,
        payload: args.payload,
        ts: args.ts,
    };
    app.emit("webview:event", &event)
        .map_err(|e| format!("emit failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("valid url")
    }

    // ── shutdown teardown ──────────────────────────────────

    /// Smoke-test [`WebviewAccountsState::drain_for_shutdown`] in isolation
    /// from the Tauri runtime. Populates the state with representative
    /// per-account resources (CDP / watchdog `JoinHandle`s, a CEF browser
    /// id, an `acct_*` label, plus the small bookkeeping sets) and asserts
    /// that one call drains every collection and aborts the long-running
    /// tasks, that the returned label list is what `shutdown_all` will
    /// `wv.close()` against, and that a second call is a safe no-op.
    ///
    /// `shutdown_all` itself takes an `AppHandle` and is exercised end-to-
    /// end at runtime; the inner `drain_for_shutdown` covers the part of
    /// the teardown that doesn't need a Tauri runtime to verify.
    #[tokio::test]
    async fn drain_for_shutdown_clears_state_and_repeat_is_noop() {
        use std::time::Duration;

        let state = WebviewAccountsState::default();

        let cdp_task = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        let watchdog_task = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        state
            .cdp_sessions
            .lock()
            .unwrap()
            .insert("acct-1".into(), cdp_task);
        state
            .load_watchdogs
            .lock()
            .unwrap()
            .insert("acct-1".into(), watchdog_task);
        state
            .browser_ids
            .lock()
            .unwrap()
            .insert("acct-1".into(), 42);
        state
            .inner
            .lock()
            .unwrap()
            .insert("acct-1".into(), "acct_1".into());
        state
            .loaded_accounts
            .lock()
            .unwrap()
            .insert("acct-1".into());
        state.requested_bounds.lock().unwrap().insert(
            "acct-1".into(),
            Bounds {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
        );

        let labels = state.drain_for_shutdown();

        assert_eq!(
            labels,
            vec![("acct-1".to_string(), "acct_1".to_string())],
            "shutdown_all should close the acct_* webview returned here"
        );
        assert!(state.cdp_sessions.lock().unwrap().is_empty());
        assert!(state.load_watchdogs.lock().unwrap().is_empty());
        assert!(state.browser_ids.lock().unwrap().is_empty());
        assert!(state.inner.lock().unwrap().is_empty());
        assert!(state.loaded_accounts.lock().unwrap().is_empty());
        assert!(state.requested_bounds.lock().unwrap().is_empty());

        // Second call must be a safe no-op: nothing left to drain.
        let labels2 = state.drain_for_shutdown();
        assert!(labels2.is_empty());
        assert!(state.cdp_sessions.lock().unwrap().is_empty());
        assert!(state.inner.lock().unwrap().is_empty());
    }

    // ── provider registry match arms ──────────────────────────────────

    #[test]
    fn zoom_registered_in_provider_url() {
        assert_eq!(provider_url("zoom"), Some("https://zoom.us/"));
    }

    #[test]
    fn zoom_has_no_recipe_js_injection() {
        // Per the CLAUDE.md "no new JS injection" rule for CEF child
        // webviews, Zoom must rely solely on Rust `on_navigation` +
        // `on_new_window` (plus CDP from scanner modules, if any) — no
        // `recipe.js` should be registered.
        assert!(provider_recipe_js("zoom").is_none());
    }

    #[test]
    fn zoom_allowed_hosts_covers_core_domains() {
        let hosts = provider_allowed_hosts("zoom");
        assert!(hosts.contains(&"zoom.us"), "zoom.us in allowlist");
        assert!(hosts.contains(&"zoomgov.com"), "zoomgov.com in allowlist");
        assert!(hosts.contains(&"zdassets.com"), "zdassets.com in allowlist");
    }

    #[test]
    fn zoom_is_supported() {
        assert!(provider_is_supported("zoom"));
    }

    // ── url_is_internal: subdomain + exact match ──────────────────────

    #[test]
    fn zoom_web_client_subdomain_is_internal() {
        assert!(url_is_internal(
            "zoom",
            &url("https://app.zoom.us/wc/join/123")
        ));
    }

    #[test]
    fn zoom_apex_domain_is_internal() {
        assert!(url_is_internal("zoom", &url("https://zoom.us/signin")));
    }

    #[test]
    fn zoom_external_host_is_not_internal() {
        assert!(!url_is_internal(
            "zoom",
            &url("https://unrelated.example.com/")
        ));
    }

    // ── rewrite_provider_deep_link: Zoom flows ────────────────────────

    #[test]
    fn rewrite_join_flow_with_confno() {
        let rewritten = rewrite_provider_deep_link(
            "zoom",
            &url("zoomus://zoom.us/join?action=join&confno=9819254358"),
        )
        .expect("rewrite should succeed");
        assert_eq!(rewritten.as_str(), "https://app.zoom.us/wc/join/9819254358");
    }

    #[test]
    fn rewrite_start_flow_with_confno() {
        let rewritten = rewrite_provider_deep_link(
            "zoom",
            &url("zoomus://zoom.us/start?action=start&confno=86449940711"),
        )
        .expect("rewrite should succeed");
        assert_eq!(
            rewritten.as_str(),
            "https://app.zoom.us/wc/join/86449940711"
        );
    }

    #[test]
    fn rewrite_preserves_pwd_query_param() {
        let rewritten = rewrite_provider_deep_link(
            "zoom",
            &url("zoomus://zoom.us/join?action=join&confno=111&pwd=secret"),
        )
        .expect("rewrite should succeed");
        assert_eq!(
            rewritten.as_str(),
            "https://app.zoom.us/wc/join/111?pwd=secret"
        );
    }

    #[test]
    fn rewrite_falls_back_to_tk_when_pwd_absent() {
        let rewritten = rewrite_provider_deep_link(
            "zoom",
            &url("zoommtg://zoom.us/join?confno=222&tk=tokenvalue"),
        )
        .expect("rewrite should succeed");
        assert_eq!(
            rewritten.as_str(),
            "https://app.zoom.us/wc/join/222?pwd=tokenvalue"
        );
    }

    #[test]
    fn rewrite_accepts_zoommtg_scheme() {
        let rewritten = rewrite_provider_deep_link(
            "zoom",
            &url("zoommtg://zoom.us/join?action=join&confno=333"),
        )
        .expect("rewrite should succeed");
        assert_eq!(rewritten.as_str(), "https://app.zoom.us/wc/join/333");
    }

    #[test]
    fn rewrite_without_confno_falls_back_to_home() {
        let rewritten =
            rewrite_provider_deep_link("zoom", &url("zoomus://zoom.us/home?action=home"))
                .expect("rewrite should succeed");
        assert_eq!(rewritten.as_str(), "https://app.zoom.us/wc/home");
    }

    #[test]
    fn rewrite_with_empty_confno_falls_back_to_home() {
        let rewritten =
            rewrite_provider_deep_link("zoom", &url("zoomus://zoom.us/join?action=join&confno="))
                .expect("rewrite should succeed");
        assert_eq!(rewritten.as_str(), "https://app.zoom.us/wc/home");
    }

    #[test]
    fn rewrite_rejects_non_zoom_provider() {
        assert!(rewrite_provider_deep_link(
            "slack",
            &url("zoomus://zoom.us/join?action=join&confno=444")
        )
        .is_none());
        assert!(rewrite_provider_deep_link(
            "google-meet",
            &url("zoomus://zoom.us/join?action=join&confno=555")
        )
        .is_none());
    }

    #[test]
    fn rewrite_rejects_http_zoom_url() {
        // Ordinary https zoom.us navigations must pass through untouched so
        // the existing `url_is_internal` flow decides.
        assert!(rewrite_provider_deep_link("zoom", &url("https://zoom.us/j/9819254358")).is_none());
    }

    #[test]
    fn rewrite_rejects_unknown_scheme() {
        assert!(rewrite_provider_deep_link(
            "zoom",
            &url("msteams://teams.microsoft.com/l/meetup-join/666")
        )
        .is_none());
    }

    // ── is_provider_native_deep_link_scheme: native-app suppression ───
    //
    // These guard the workspace-isolation contract from #1074: provider
    // native-desktop-app deep-link schemes must NEVER reach the system
    // browser, because macOS hands them off to the native provider app
    // which then signs the user into the workspace using session tokens
    // intended only for the embedded webview (see slack://magic-login
    // smoking gun in the #1074 trace).

    #[test]
    fn deep_link_scheme_matches_known_provider_native_apps() {
        // Slack desktop ("slack://T01.../magic-login/<token>")
        assert!(is_provider_native_deep_link_scheme("slack"));
        // Discord desktop
        assert!(is_provider_native_deep_link_scheme("discord"));
        // Telegram desktop ("tg://join?invite=…")
        assert!(is_provider_native_deep_link_scheme("tg"));
        // Microsoft Teams
        assert!(is_provider_native_deep_link_scheme("msteams"));
        // Zoom client (both variants registered by the installer)
        assert!(is_provider_native_deep_link_scheme("zoomus"));
        assert!(is_provider_native_deep_link_scheme("zoommtg"));
    }

    #[test]
    fn deep_link_scheme_rejects_legitimate_external_schemes() {
        // HTTP(S) — the bread-and-butter external link.
        assert!(!is_provider_native_deep_link_scheme("https"));
        assert!(!is_provider_native_deep_link_scheme("http"));
        // Mail clients are legit external — must NOT be suppressed.
        assert!(!is_provider_native_deep_link_scheme("mailto"));
        // Telephone / sms are legit external too.
        assert!(!is_provider_native_deep_link_scheme("tel"));
        assert!(!is_provider_native_deep_link_scheme("sms"));
        // about: / data: / blob: handled elsewhere; never deep-link.
        assert!(!is_provider_native_deep_link_scheme("about"));
        assert!(!is_provider_native_deep_link_scheme("data"));
        assert!(!is_provider_native_deep_link_scheme("blob"));
        // Empty / unrelated string.
        assert!(!is_provider_native_deep_link_scheme(""));
        assert!(!is_provider_native_deep_link_scheme("file"));
    }

    #[test]
    fn deep_link_scheme_matches_real_world_slack_magic_login_url() {
        // Real slack://-flavoured magic-login URL recorded in the
        // #1074 CDP trace. The handler must catch it before
        // open_in_system_browser is reached.
        let parsed = url("slack://T01CWHNCJ9Z/magic-login/11035712490054-abc");
        assert!(is_provider_native_deep_link_scheme(parsed.scheme()));
    }

    #[test]
    fn deep_link_scheme_does_not_match_https_app_slack_com() {
        // The web-flow URL stays untouched — only the slack:// scheme is
        // suppressed; ordinary HTTPS slack navigations route normally.
        let parsed = url("https://app.slack.com/client/T01CWHNCJ9Z");
        assert!(!is_provider_native_deep_link_scheme(parsed.scheme()));
    }

    /// Locks the contract that zoomus:// stays on the rewrite path
    /// (handled by `rewrite_provider_deep_link` for the "zoom" provider)
    /// rather than being silently suppressed.
    ///
    /// The wiring in on_navigation / on_new_window calls
    /// `rewrite_provider_deep_link` BEFORE the suppress check, so a
    /// rewriteable scheme is rewritten and never reaches the suppress
    /// branch. This test pins both halves of that contract: the rewrite
    /// still succeeds for zoom, AND the scheme is recognised as a
    /// native-app deep-link (so if a future provider config dropped the
    /// rewrite, suppression would be the safe fallback rather than
    /// leaking to the system browser).
    #[test]
    fn zoomus_join_still_rewrites_and_is_recognized_as_native_scheme() {
        let zoom_url = url("zoomus://zoom.us/join?action=join&confno=9819254358");
        assert!(is_provider_native_deep_link_scheme(zoom_url.scheme()));
        let rewritten = rewrite_provider_deep_link("zoom", &zoom_url)
            .expect("zoom rewrite should still succeed before suppress branch");
        assert_eq!(rewritten.as_str(), "https://app.zoom.us/wc/join/9819254358");
    }

    #[test]
    fn rewrite_percent_encodes_reserved_chars_in_pwd() {
        // Zoom tokens commonly contain `&` / `=` / `%` / `#` / `+` which
        // would corrupt a hand-rolled format!() URL. The `Url`-based
        // builder must percent-encode them.
        let rewritten = rewrite_provider_deep_link(
            "zoom",
            &url("zoomus://zoom.us/join?action=join&confno=777&pwd=a%26b%3Dc"),
        )
        .expect("rewrite should succeed");
        // `url::Url` round-trips the encoded `%26` (`&`) and `%3D` (`=`)
        // back into the rewritten query.
        assert!(
            rewritten.as_str().contains("pwd=a%26b%3Dc"),
            "expected encoded pwd, got {}",
            rewritten.as_str()
        );
    }

    #[test]
    fn rewrite_percent_encodes_confno_segment() {
        // Defensive — path segments never should carry reserved chars but
        // the helper must not corrupt them if they do.
        let rewritten = rewrite_provider_deep_link(
            "zoom",
            &url("zoomus://zoom.us/join?action=join&confno=abc%2Fdef"),
        )
        .expect("rewrite should succeed");
        // `/` inside the id must be percent-encoded, not merged into the path.
        assert!(
            rewritten.path().ends_with("/abc%2Fdef"),
            "expected encoded path segment, got {}",
            rewritten.path()
        );
    }

    // ── popup_should_stay_in_app: Zoom WebClient popups ───────────────

    #[test]
    fn zoom_webclient_popup_stays_in_app() {
        assert!(popup_should_stay_in_app(
            "zoom",
            &url("https://app.zoom.us/wc/join/999")
        ));
    }

    #[test]
    fn zoom_apex_webclient_popup_stays_in_app() {
        assert!(popup_should_stay_in_app(
            "zoom",
            &url("https://zoom.us/wc/join/999")
        ));
    }

    #[test]
    fn zoom_non_wc_popup_does_not_stay_in_app() {
        // Marketing / blog / download-link popups should hand off to the
        // system browser, not grow an in-app child window.
        assert!(!popup_should_stay_in_app(
            "zoom",
            &url("https://zoom.us/about")
        ));
    }

    #[test]
    fn zoom_popup_to_foreign_host_does_not_stay_in_app() {
        assert!(!popup_should_stay_in_app(
            "zoom",
            &url("https://example.com/wc/join/888")
        ));
    }

    // ── popup_should_navigate_parent: Gmail Google-auth popups ────────

    #[test]
    fn gmail_accounts_popup_navigates_parent() {
        // Clicking "Sign in" on mail.google.com opens accounts.google.com
        // in a popup; routing it to the system browser breaks the OAuth
        // round-trip because the auth response would land in the wrong
        // cookie jar. Allowing CEF to spawn an unmanaged popup leaves it
        // blank/black (no host slot, no bounds). Navigate the parent
        // instead so the auth flow lands in the existing webview.
        assert_eq!(
            popup_should_navigate_parent(
                "gmail",
                &url("https://accounts.google.com/signin/v2/identifier"),
            )
            .map(|u| u.to_string()),
            Some("https://accounts.google.com/signin/v2/identifier".to_string())
        );
    }

    #[test]
    fn gmail_about_blank_popup_does_not_navigate_parent() {
        // about:blank popups are non-actionable — there's no URL yet to
        // navigate the parent to. Fall through to the popup branch.
        assert!(popup_should_navigate_parent("gmail", &url("about:blank")).is_none());
    }

    #[test]
    fn gmail_foreign_host_popup_does_not_navigate_parent() {
        // External link popups still belong in the system browser.
        assert!(
            popup_should_navigate_parent("gmail", &url("https://example.com/share?u=…")).is_none()
        );
    }

    #[test]
    fn gmail_internal_non_auth_popup_does_not_navigate_parent() {
        assert!(popup_should_navigate_parent(
            "gmail",
            &url("https://mail.google.com/mail/u/0/#inbox")
        )
        .is_none());
    }

    #[test]
    fn google_meet_accounts_popup_navigates_parent() {
        assert!(popup_should_navigate_parent(
            "google-meet",
            &url("https://accounts.google.com/signin/v2/identifier"),
        )
        .is_some());
    }

    #[test]
    fn gmeet_room_popup_navigates_parent() {
        // "Start an instant meeting" / "New meeting" calls
        // window.open(meet.google.com/<roomid>) to launch a room.
        // Without intervention this would route to system Chrome and
        // leak the meeting out of OpenHuman.
        assert_eq!(
            popup_should_navigate_parent(
                "google-meet",
                &url("https://meet.google.com/abc-defg-hij"),
            )
            .map(|u| u.to_string()),
            Some("https://meet.google.com/abc-defg-hij".to_string())
        );
    }

    #[test]
    fn gmeet_landing_popup_navigates_parent() {
        // Bare meet.google.com (no room code) should also be kept
        // in-app — matches the "back to Meet home" UX after hangup.
        assert!(
            popup_should_navigate_parent("google-meet", &url("https://meet.google.com/"),)
                .is_some()
        );
    }

    #[test]
    fn gmeet_workspace_popup_does_not_navigate_parent() {
        // workspace.google.com is the marketing page; if it ever
        // arrives via window.open() we let the default external-route
        // logic handle it (covered in the on_navigation rewrite path
        // separately).
        assert!(popup_should_navigate_parent(
            "google-meet",
            &url("https://workspace.google.com/products/meet/"),
        )
        .is_none());
    }

    #[test]
    fn gmeet_unrelated_popup_does_not_navigate_parent() {
        // External link in the post-call review screen, for instance.
        // Should NOT navigate the parent — should fall through to the
        // system-browser path.
        assert!(
            popup_should_navigate_parent("google-meet", &url("https://example.com/blog"),)
                .is_none()
        );
    }

    #[test]
    fn gmail_service_login_popup_navigates_parent() {
        assert_eq!(
            popup_should_navigate_parent(
                "gmail",
                &url("https://accounts.google.com/ServiceLogin?continue=https://mail.google.com"),
            )
            .map(|u| u.to_string()),
            Some(
                "https://accounts.google.com/ServiceLogin?continue=https://mail.google.com"
                    .to_string()
            )
        );
    }

    #[test]
    fn redact_navigation_url_strips_query_and_fragment() {
        let redacted = redact_navigation_url(&url(
            "https://accounts.google.com/o/oauth2/v2/auth?code=secret#frag",
        ));
        assert_eq!(redacted, "https://accounts.google.com/o/oauth2/v2/auth");
    }
}
