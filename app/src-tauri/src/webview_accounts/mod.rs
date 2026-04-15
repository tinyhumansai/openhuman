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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Runtime, Url, WebviewBuilder,
    WebviewUrl,
};

const RUNTIME_JS: &str = include_str!("runtime.js");
const UA_SPOOF_JS: &str = include_str!("ua_spoof.js");
const WHATSAPP_RECIPE_JS: &str = include_str!("../../recipes/whatsapp/recipe.js");
const TELEGRAM_RECIPE_JS: &str = include_str!("../../recipes/telegram/recipe.js");
const LINKEDIN_RECIPE_JS: &str = include_str!("../../recipes/linkedin/recipe.js");
const GMAIL_RECIPE_JS: &str = include_str!("../../recipes/gmail/recipe.js");
const SLACK_RECIPE_JS: &str = include_str!("../../recipes/slack/recipe.js");

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
        _ => None,
    }
}

fn provider_user_agent(provider: &str) -> Option<&'static str> {
    match provider {
        "whatsapp" | "telegram" | "linkedin" | "gmail" | "slack" => Some(CHROME_UA),
        _ => None,
    }
}

fn provider_recipe_js(provider: &str) -> Option<&'static str> {
    match provider {
        "whatsapp" => Some(WHATSAPP_RECIPE_JS),
        "telegram" => Some(TELEGRAM_RECIPE_JS),
        "linkedin" => Some(LINKEDIN_RECIPE_JS),
        "gmail" => Some(GMAIL_RECIPE_JS),
        "slack" => Some(SLACK_RECIPE_JS),
        _ => None,
    }
}

/// Whether to pre-load `ua_spoof.js` for a given provider. Enabled only
/// for services known to run Chromium-specific fingerprinting checks —
/// WhatsApp & Telegram are happy with the Chrome UA alone and running the
/// spoof risks breaking perfectly-working integrations for no gain.
fn provider_ua_spoof(provider: &str) -> bool {
    matches!(provider, "slack" | "gmail" | "linkedin")
}

#[derive(Default)]
pub struct WebviewAccountsState {
    /// account_id -> webview label (we use `acct_<id>` as the label).
    inner: Mutex<HashMap<String, String>>,
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
pub struct SuggestionArgs {
    pub account_id: String,
    pub composer_id: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct ComposerActionArgs {
    pub account_id: String,
    pub composer_id: String,
}

#[derive(Debug, Deserialize)]
pub struct EvalArgs {
    pub account_id: String,
    pub js: String,
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

fn label_for(account_id: &str) -> String {
    // Webview labels must be alphanumeric + `-` / `_`. Account IDs come from
    // the React side as UUIDs so this is just defensive.
    let safe: String = account_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    format!("acct_{}", safe)
}

fn data_directory_for<R: Runtime>(app: &AppHandle<R>, account_id: &str) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("app_local_data_dir: {e}"))?;
    Ok(base.join("webview_accounts").join(account_id))
}

fn build_init_script(account_id: &str, provider: &str, recipe_js: &str) -> String {
    // Inject context first so the runtime can read it on load. JSON-encode
    // the values so escaping is safe. Order matters:
    //   1. UA spoof (must land BEFORE page JS reads `navigator`)
    //   2. Recipe context
    //   3. Recipe runtime
    //   4. Per-provider recipe
    let ctx = serde_json::json!({
        "accountId": account_id,
        "provider": provider,
    });
    let spoof = if provider_ua_spoof(provider) {
        UA_SPOOF_JS
    } else {
        ""
    };
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

    let url_str = args
        .url
        .as_deref()
        .or_else(|| provider_url(&args.provider))
        .ok_or_else(|| format!("unknown provider: {}", args.provider))?;
    let url: Url = url_str
        .parse()
        .map_err(|e| format!("invalid url {url_str}: {e}"))?;
    let recipe_js = provider_recipe_js(&args.provider)
        .ok_or_else(|| format!("no recipe registered for provider: {}", args.provider))?;

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

    let init_script = build_init_script(&args.account_id, &args.provider, recipe_js);

    let mut builder = WebviewBuilder::new(label.clone(), WebviewUrl::External(url))
        .initialization_script(&init_script)
        .data_directory(data_dir);

    // Always enable devtools on child webviews so recipe diagnostics and
    // IndexedDB state can be inspected. Access on macOS is via
    //   Safari → Develop → <App name> → <webview label>
    // (the parent Tauri window's right-click "Inspect" does not propagate
    // into child webviews on WKWebView).
    builder = builder.devtools(true);

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

    // For providers we know how to scrape via CDP, kick off the IndexedDB
    // scanner. Compile-gated to `cef` because CDP only exists when the CEF
    // runtime is in use (wry has no remote-debugging port).
    #[cfg(feature = "cef")]
    {
        if args.provider == "whatsapp" {
            if let Some(prefix) = provider_url(&args.provider) {
                let registry = app
                    .try_state::<std::sync::Arc<crate::whatsapp_scanner::ScannerRegistry>>()
                    .map(|s| s.inner().clone());
                if let Some(registry) = registry {
                    let app_clone = app.clone();
                    let acct = args.account_id.clone();
                    let prefix = prefix.to_string();
                    tokio::spawn(async move {
                        registry.ensure_scanner(app_clone, acct, prefix).await;
                    });
                } else {
                    log::warn!("[webview-accounts] CDP ScannerRegistry not in app state");
                }
            }
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
    }
    log::info!("[webview-accounts] closed label={}", label);
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
    let Some(label) = label_opt else { return Ok(()) };
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
    let Some(label) = label_opt else { return Ok(()) };
    if let Some(wv) = app.get_webview(&label) {
        let _ = wv.show();
        log::debug!("[webview-accounts] show label={}", label);
    }
    Ok(())
}

/// Look up the live `Webview` for an account, or return a descriptive error.
fn resolve_webview<R: Runtime>(
    app: &AppHandle<R>,
    state: &tauri::State<'_, WebviewAccountsState>,
    account_id: &str,
) -> Result<tauri::Webview<R>, String> {
    let label = state
        .inner
        .lock()
        .unwrap()
        .get(account_id)
        .cloned()
        .ok_or_else(|| format!("no webview for account {account_id}"))?;
    app.get_webview(&label)
        .ok_or_else(|| format!("webview {label} missing (stale state)"))
}

/// JS-string-escape a Rust `&str` for safe interpolation into a string
/// literal inside an `eval()` payload. We can't use serde_json::to_string
/// for the suggestion text alone because we need to slot it into a single
/// JS expression — wrapping the whole arg list as JSON keeps escaping
/// trustworthy across newlines, quotes, and unicode.
fn build_invoke_recipe(method: &str, args: &[serde_json::Value]) -> Result<String, String> {
    let serialized = args
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("serialize args: {e}"))?
        .join(", ");
    Ok(format!(
        "(function(){{ try {{ if (window.__openhumanRecipe && typeof window.__openhumanRecipe.{m} === 'function') {{ window.__openhumanRecipe.{m}({a}); }} }} catch (e) {{ console.error('[openhuman] {m} failed', e); }} }})();",
        m = method,
        a = serialized,
    ))
}

/// Push a ghost-text suggestion into a composer the recipe has registered
/// via `__openhumanRecipe.attachComposer(...)`. The user accepts with Tab
/// (or whatever `suggestionKey` the recipe attached with).
#[tauri::command]
pub async fn webview_account_set_suggestion<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, WebviewAccountsState>,
    args: SuggestionArgs,
) -> Result<(), String> {
    let wv = resolve_webview(&app, &state, &args.account_id)?;
    let js = build_invoke_recipe(
        "setSuggestion",
        &[
            serde_json::Value::String(args.composer_id.clone()),
            serde_json::Value::String(args.text.clone()),
        ],
    )?;
    log::debug!(
        "[webview-accounts] set_suggestion account={} composer={} len={}",
        args.account_id,
        args.composer_id,
        args.text.chars().count()
    );
    wv.eval(&js).map_err(|e| format!("eval failed: {e}"))
}

/// Clear the active ghost suggestion in a composer.
#[tauri::command]
pub async fn webview_account_clear_suggestion<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, WebviewAccountsState>,
    args: ComposerActionArgs,
) -> Result<(), String> {
    let wv = resolve_webview(&app, &state, &args.account_id)?;
    let js = build_invoke_recipe(
        "clearSuggestion",
        &[serde_json::Value::String(args.composer_id.clone())],
    )?;
    log::debug!(
        "[webview-accounts] clear_suggestion account={} composer={}",
        args.account_id,
        args.composer_id
    );
    wv.eval(&js).map_err(|e| format!("eval failed: {e}"))
}

/// Programmatically commit (insert) the active suggestion as if the user
/// pressed Tab. Useful for "accept" buttons in the host UI.
#[tauri::command]
pub async fn webview_account_commit_suggestion<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, WebviewAccountsState>,
    args: ComposerActionArgs,
) -> Result<(), String> {
    let wv = resolve_webview(&app, &state, &args.account_id)?;
    let js = build_invoke_recipe(
        "commitSuggestion",
        &[serde_json::Value::String(args.composer_id.clone())],
    )?;
    log::debug!(
        "[webview-accounts] commit_suggestion account={} composer={}",
        args.account_id,
        args.composer_id
    );
    wv.eval(&js).map_err(|e| format!("eval failed: {e}"))
}

/// Generic eval escape hatch — runs `js` inside the account's webview.
/// Prefer the typed commands above; only use this for one-off recipe
/// helpers or debugging.
#[tauri::command]
pub async fn webview_account_eval<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, WebviewAccountsState>,
    args: EvalArgs,
) -> Result<(), String> {
    let wv = resolve_webview(&app, &state, &args.account_id)?;
    log::debug!(
        "[webview-accounts] eval account={} bytes={}",
        args.account_id,
        args.js.len()
    );
    wv.eval(&args.js).map_err(|e| format!("eval failed: {e}"))
}

/// Called from the injected runtime each time the recipe emits an event.
/// We forward to React via a Tauri event so the UI can render and persist.
#[tauri::command]
pub async fn webview_recipe_event<R: Runtime>(
    app: AppHandle<R>,
    args: RecipeEventArgs,
) -> Result<(), String> {
    log::debug!(
        "[webview-accounts] recipe_event account={} provider={} kind={}",
        args.account_id,
        args.provider,
        args.kind
    );
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
        let size = args.payload.get("size").and_then(|v| v.as_i64()).unwrap_or(0);
        log::trace!(
            "[webview-accounts][{}] ws {} {} bytes",
            args.account_id,
            direction,
            size
        );
    } else if args.kind == "composer_input" {
        let composer = args
            .payload
            .get("composerId")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let len = args
            .payload
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.chars().count())
            .unwrap_or(0);
        log::debug!(
            "[webview-accounts][{}] composer_input id={} chars={}",
            args.account_id,
            composer,
            len
        );
    } else if args.kind == "composer_commit" {
        let composer = args
            .payload
            .get("composerId")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let source = args
            .payload
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        log::info!(
            "[webview-accounts][{}] composer_commit id={} source={}",
            args.account_id,
            composer,
            source
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
