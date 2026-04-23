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
#[cfg(all(feature = "cef", target_os = "linux"))]
use std::sync::{mpsc::sync_channel, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{
    webview::NewWindowResponse, AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Runtime,
    Url, WebviewBuilder, WebviewUrl,
};
#[cfg(all(feature = "cef", windows))]
use tauri_plugin_notification::NotificationExt;
// `ImplBrowser` exposes `Browser::identifier()` — bring the trait into scope
// so the `with_webview` callback can read the CEF browser id.
#[cfg(feature = "cef")]
use cef::ImplBrowser;

#[cfg(feature = "cef")]
use crate::cdp;

const RUNTIME_JS: &str = include_str!("runtime.js");
// UA spoofing moved from injected JS to CDP `Emulation.setUserAgentOverride`
// under the cef feature; wry builds still need the old JS shim so the recipes
// that emit an `ingest` payload (gmail / linkedin / google-meet) survive
// fingerprint gates on Slack/Google's login flow.
const UA_SPOOF_JS: &str = include_str!("ua_spoof.js");
const LINKEDIN_RECIPE_JS: &str = include_str!("../../recipes/linkedin/recipe.js");
const GMAIL_RECIPE_JS: &str = include_str!("../../recipes/gmail/recipe.js");
const GOOGLE_MEET_RECIPE_JS: &str = include_str!("../../recipes/google-meet/recipe.js");

/// User agent we pretend to be for all external services. Web-app services
/// (WhatsApp, Gmail, Google's login flow) reject "unknown" WebView UAs with
/// upgrade-your-browser / unsupported-browser pages, so we announce as a
/// recent desktop Chrome build for everything.
const CHROME_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                         (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

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
        "browserscan" => Some("https://www.browserscan.net/bot-detection"),
        _ => None,
    }
}

fn provider_user_agent(provider: &str) -> Option<&'static str> {
    match provider {
        "whatsapp" | "telegram" | "linkedin" | "gmail" | "slack" | "discord" | "google-meet"
        | "browserscan" => Some(CHROME_UA),
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

/// Whether to pre-load `ua_spoof.js` for a given provider (wry only — cef
/// handles UA via CDP `Emulation.setUserAgentOverride`). Enabled for
/// services known to run Chromium-specific fingerprinting checks.
fn provider_ua_spoof(provider: &str) -> bool {
    matches!(
        provider,
        "slack" | "gmail" | "linkedin" | "discord" | "google-meet" | "browserscan"
    )
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
        "browserscan" => &["browserscan.net"],
        _ => &[],
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
        _ => false,
    }
}
/// Fire-and-forget handoff to the OS default URL handler. Any error is
/// logged but not propagated — we've already cancelled the in-app
/// navigation so there's nowhere to surface a failure to.
fn open_in_system_browser(url: &str) {
    match tauri_plugin_opener::open_url(url, None::<&str>) {
        Ok(()) => log::info!("[webview-accounts] opened externally: {}", url),
        Err(e) => log::warn!("[webview-accounts] open_url({}) failed: {}", url, e),
    }
}

/// Human-readable label used as the title prefix on native notifications
/// so users can tell which provider fired the ping. Matches the labels
/// in the frontend `PROVIDERS` registry.
#[cfg(feature = "cef")]
pub fn provider_display_name(provider: &str) -> &'static str {
    match provider {
        "whatsapp" => "WhatsApp",
        "telegram" => "Telegram",
        "linkedin" => "LinkedIn",
        "gmail" => "Gmail",
        "slack" => "Slack",
        "discord" => "Discord",
        "google-meet" => "Google Meet",
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
    #[cfg(feature = "cef")]
    browser_ids: Mutex<HashMap<String, i32>>,
    /// account_id -> CDP session task. One long-lived task per account
    /// keeps the UA override resident (see `cdp::session`); aborted on
    /// close/purge so reopen cycles don't stack multiple live loops.
    #[cfg(feature = "cef")]
    cdp_sessions: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    /// Runtime notification-bypass controls used by the settings UI.
    notification_bypass: Mutex<NotificationBypassPrefs>,
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
#[cfg(feature = "cef")]
const OPENHUMAN_TITLE_PREFIX: &str = "OpenHuman: ";

#[cfg(feature = "cef")]
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
#[cfg(feature = "cef")]
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
#[cfg(all(feature = "cef", target_os = "linux"))]
const LINUX_NOTIFY_QUEUE_CAP: usize = 16;

#[cfg(all(feature = "cef", target_os = "linux"))]
static LINUX_NOTIFY_TX: OnceLock<std::sync::mpsc::SyncSender<Box<dyn FnOnce() + Send>>> =
    OnceLock::new();

#[cfg(all(feature = "cef", target_os = "linux"))]
fn enqueue_linux_notification(job: Box<dyn FnOnce() + Send>) {
    let tx = LINUX_NOTIFY_TX.get_or_init(|| {
        let (tx, rx) = sync_channel(LINUX_NOTIFY_QUEUE_CAP);
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
#[cfg(feature = "cef")]
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
        let title_c = notify_title.clone();
        let body_c = body.to_string();
        let app_id = app.config().identifier.clone();
        std::thread::spawn(move || {
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

#[cfg(feature = "cef")]
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
/// Under **cef** (production): empty for the 5 migrated providers
/// (whatsapp, telegram, slack, discord, browserscan) — they load with
/// ZERO injected JS; their scraping + UA override runs via CDP. The 3
/// deferred providers (gmail, linkedin, google-meet) still get the JS
/// recipe bridge.
///
/// Under **wry**: there is no CDP, so migrated providers that fingerprint
/// on `navigator.*` still need the `ua_spoof.js` shim even though their
/// scraper is gone. Non-migrated providers keep the full legacy path
/// (spoof + runtime + recipe).
#[cfg(feature = "cef")]
fn build_init_script(account_id: &str, provider: &str) -> String {
    let spoof = if provider_ua_spoof(provider) {
        UA_SPOOF_JS
    } else {
        ""
    };
    let Some(recipe_js) = provider_recipe_js(provider) else {
        return spoof.to_string();
    };
    let ctx = serde_json::json!({
        "accountId": account_id,
        "provider": provider,
    });
    format!(
        "{spoof}\n\nwindow.__OPENHUMAN_RECIPE_CTX__ = {ctx};\n\n{runtime}\n\n{recipe}\n",
        spoof = spoof,
        ctx = ctx,
        runtime = RUNTIME_JS,
        recipe = recipe_js
    )
}

#[cfg(not(feature = "cef"))]
fn build_init_script(account_id: &str, provider: &str) -> String {
    let spoof = if provider_ua_spoof(provider) {
        UA_SPOOF_JS
    } else {
        ""
    };
    // Migrated providers have no recipe under wry either (recipe.js
    // files were deleted with the cef migration), but the UA shim is
    // still worth shipping so fingerprint gates pass.
    let Some(recipe_js) = provider_recipe_js(provider) else {
        return spoof.to_string();
    };
    let ctx = serde_json::json!({
        "accountId": account_id,
        "provider": provider,
    });
    format!(
        "{spoof}\n\nwindow.__OPENHUMAN_RECIPE_CTX__ = {ctx};\n\n{runtime}\n\n{recipe}\n",
        spoof = spoof,
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
    #[cfg(feature = "cef")]
    let scanner_url_prefix = format!("{}/", real_url.origin().ascii_serialization());
    #[cfg(feature = "cef")]
    let skip_cdp_for_debug = args.provider == "slack" && !slack_scanner_enabled();
    // Under cef we normally open the webview at a tiny `data:` placeholder
    // URL so the CDP session opener can attach and apply the UA override
    // BEFORE the real provider URL loads. For Slack debug sessions we allow
    // opting out via `OPENHUMAN_DISABLE_SLACK_SCANNER=1`, which also skips
    // the long-lived CDP session so external DevTools can attach cleanly.
    // Under wry there's no CDP, so navigate straight to the real URL and
    // rely on the injected `ua_spoof.js`.
    #[cfg(feature = "cef")]
    let initial_url_str = if skip_cdp_for_debug {
        real_url_str.clone()
    } else {
        cdp::placeholder_data_url(&args.account_id)
    };
    #[cfg(not(feature = "cef"))]
    let initial_url_str = real_url_str.clone();
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
                }
                let _ = existing.show();
                log::info!(
                    "[webview-accounts] reused existing label={} for account={}",
                    existing_label,
                    args.account_id
                );
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
    // provider) stay in-app.
    let nav_provider = args.provider.clone();
    builder = builder.on_navigation(move |url| {
        if url_is_internal(&nav_provider, url) {
            true
        } else {
            log::info!(
                "[webview-accounts] external navigation {} → system browser",
                url
            );
            open_in_system_browser(url.as_str());
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
    builder = builder.on_new_window(move |url, _features| {
        if popup_should_stay_in_app(&popup_provider, &url) {
            log::info!(
                "[webview-accounts] new-window request {} → in-app popup (provider={})",
                url,
                popup_provider
            );
            NewWindowResponse::Allow
        } else {
            log::info!(
                "[webview-accounts] new-window request {} → system browser",
                url
            );
            open_in_system_browser(url.as_str());
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

    if let Some(ua) = provider_user_agent(&args.provider) {
        builder = builder.user_agent(ua);
    }

    let bounds = args.bounds.unwrap_or(Bounds {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    });

    let webview = parent_window
        .add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.width, bounds.height),
        )
        .map_err(|e| format!("add_child failed: {e}"))?;

    log::info!(
        "[webview-accounts] spawned label={} bounds={:?}",
        webview.label(),
        bounds
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
    #[cfg(feature = "cef")]
    {
        if skip_cdp_for_debug {
            log::info!(
                "[webview-accounts] skipping CDP session via OPENHUMAN_DISABLE_SLACK_SCANNER for account={}",
                args.account_id
            );
        } else {
            let handle = cdp::spawn_session(args.account_id.clone(), real_url_str.clone());
            let mut sessions = state.cdp_sessions.lock().unwrap();
            if let Some(old) = sessions.insert(args.account_id.clone(), handle) {
                old.abort();
            }
        }
    }

    // For providers we know how to scrape via CDP, kick off the IndexedDB
    // scanner. Compile-gated to `cef` because CDP only exists when the CEF
    // runtime is in use (wry has no remote-debugging port).
    #[cfg(feature = "cef")]
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
    #[cfg(feature = "cef")]
    {
        if let Some(registry) =
            app.try_state::<std::sync::Arc<crate::whatsapp_scanner::ScannerRegistry>>()
        {
            let registry = registry.inner().clone();
            let acct = args.account_id.clone();
            tokio::spawn(async move { registry.forget(&acct).await });
        }
        if let Some(registry) =
            app.try_state::<std::sync::Arc<crate::slack_scanner::ScannerRegistry>>()
        {
            let registry = registry.inner().clone();
            let acct = args.account_id.clone();
            tokio::spawn(async move { registry.forget(&acct).await });
        }
        if let Some(registry) =
            app.try_state::<std::sync::Arc<crate::discord_scanner::ScannerRegistry>>()
        {
            let registry = registry.inner().clone();
            let acct = args.account_id.clone();
            tokio::spawn(async move { registry.forget(&acct).await });
        }
        if let Some(registry) =
            app.try_state::<std::sync::Arc<crate::telegram_scanner::ScannerRegistry>>()
        {
            let registry = registry.inner().clone();
            let acct = args.account_id.clone();
            tokio::spawn(async move { registry.forget(&acct).await });
        }
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
    }
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

    #[cfg(feature = "cef")]
    {
        if let Some(registry) =
            app.try_state::<std::sync::Arc<crate::whatsapp_scanner::ScannerRegistry>>()
        {
            let registry = registry.inner().clone();
            let acct = args.account_id.clone();
            tokio::spawn(async move { registry.forget(&acct).await });
        }
        if let Some(registry) =
            app.try_state::<std::sync::Arc<crate::slack_scanner::ScannerRegistry>>()
        {
            let registry = registry.inner().clone();
            let acct = args.account_id.clone();
            tokio::spawn(async move { registry.forget(&acct).await });
        }
        if let Some(registry) =
            app.try_state::<std::sync::Arc<crate::discord_scanner::ScannerRegistry>>()
        {
            let registry = registry.inner().clone();
            let acct = args.account_id.clone();
            tokio::spawn(async move { registry.forget(&acct).await });
        }
        if let Some(registry) =
            app.try_state::<std::sync::Arc<crate::telegram_scanner::ScannerRegistry>>()
        {
            let registry = registry.inner().clone();
            let acct = args.account_id.clone();
            tokio::spawn(async move { registry.forget(&acct).await });
        }
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
    }

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
/// CEF path is effectively granted because interception is handled in-app.
#[tauri::command]
pub fn webview_notification_permission_state() -> String {
    #[cfg(feature = "cef")]
    {
        "granted".to_string()
    }
    #[cfg(not(feature = "cef"))]
    {
        "default".to_string()
    }
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
