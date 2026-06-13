//! CDP target discovery + per-attach helpers.
//!
//! Each CEF webview is its own browser instance with its own DevTools
//! channel (see [`super::in_process`]), so the multi-target multiplexer
//! that used to live in this module has been simplified — there is no
//! more `browser_ws_url()` HTTP discovery and no remote attach. The
//! remaining helpers (`Target.getTargets` walk, `Target.attachToTarget`
//! flatten-attach, detach) still apply because the page itself may
//! contain iframes / workers that the scanners care about.

use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, Runtime};

use super::{in_process::CdpRegistry, CdpConn, CDP_HOST, CDP_PORT};

#[derive(Debug, Clone)]
pub struct CdpTarget {
    pub id: String,
    pub kind: String,
    pub url: String,
    pub title: String,
}

/// Legacy TCP WebSocket discovery — kept for the per-scanner `CdpConn`
/// duplicates that have not yet migrated to the in-process transport.
/// New code paths use [`conn_for_account`] which goes through the
/// in-process channel installed by `webview_accounts::open`.
///
/// Returns the browser-level WebSocket URL by hitting
/// `http://{CDP_HOST}:{CDP_PORT}/json/version`. Requires Chromium to
/// have been spawned with `--remote-debugging-port=<CDP_PORT>` — see
/// `app/src-tauri/src/lib.rs`.
pub async fn browser_ws_url() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("openhuman-cdp/1.0")
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("reqwest build: {e}"))?;
    let mut last_err: Option<String> = None;
    for host in [CDP_HOST, "localhost"] {
        let url = format!("http://{host}:{CDP_PORT}/json/version");
        match client.get(&url).send().await {
            Ok(resp) => match resp.json::<Value>().await {
                Ok(v) => {
                    if let Some(ws) = v.get("webSocketDebuggerUrl").and_then(|x| x.as_str()) {
                        return Ok(ws.to_string());
                    }
                    last_err = Some(format!("no webSocketDebuggerUrl in {url}"));
                }
                Err(e) => {
                    last_err = Some(format!("parse {url}: {e}"));
                }
            },
            Err(e) => {
                last_err = Some(format!("GET {url}: {e}"));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "failed to resolve CDP websocket URL".to_string()))
}

/// Parse the response of a `Target.getTargets` CDP call into a list of
/// targets. Public so scanners using the lower-level [`CdpConn::call`]
/// can interpret target lists.
pub fn parse_targets(v: &Value) -> Vec<CdpTarget> {
    v.get("targetInfos")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    Some(CdpTarget {
                        id: t.get("targetId")?.as_str()?.to_string(),
                        kind: t.get("type")?.as_str()?.to_string(),
                        url: t
                            .get("url")
                            .and_then(|u| u.as_str())
                            .unwrap_or("")
                            .to_string(),
                        title: t
                            .get("title")
                            .and_then(|u| u.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Get a [`CdpConn`] for an account-keyed webview, looking up the
/// pre-installed in-process transport from the [`CdpRegistry`] managed
/// on `app`.
///
/// On a cache miss, falls back to
/// [`super::in_process::install_for_account`] so a transient install
/// failure during `webview_accounts::open` (logged as a warning by the
/// account-open path, not fatal) doesn't permanently lock the account
/// out of CDP. The install call is idempotent and cheap on the cached
/// path. Still returns `Err` when the webview itself has not yet been
/// created — caller backs off and retries.
pub fn conn_for_account<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
) -> Result<CdpConn, String> {
    let registry = app
        .try_state::<CdpRegistry>()
        .ok_or_else(|| "CdpRegistry not managed by app".to_string())?;
    if let Some(transport) = registry.by_account(account_id) {
        return Ok(CdpConn::new(transport));
    }
    // Retry — the install path is idempotent. The most common cause of
    // a cache miss here is an earlier non-fatal `install_for_account`
    // failure in `webview_accounts::open` (warn-logged) that left the
    // webview alive without a transport.
    let transport = super::in_process::install_for_account(account_id)
        .map_err(|e| format!("no cdp transport for account {account_id} (install retry: {e})"))?;
    Ok(CdpConn::new(transport))
}

/// Full short-lived attach sequence on the account's webview via the
/// in-process channel: look up the [`CdpRegistry`] transport for the
/// given account, find the matching page target via
/// `Target.getTargets`, attach with `flatten: true`. Caller gets a
/// ready `CdpConn` + session id. Caller MUST `detach_session` (or drop
/// the `CdpConn`) when done so the session id doesn't linger inside
/// CEF.
pub async fn connect_and_attach_matching_in_process<R, F>(
    app: &AppHandle<R>,
    account_id: &str,
    pred: F,
) -> Result<(CdpConn, String), String>
where
    R: Runtime,
    F: Fn(&CdpTarget) -> bool,
{
    let mut cdp = conn_for_account(app, account_id)?;
    let target = find_page_target_where(&mut cdp, pred).await?;
    let attach = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": target.id, "flatten": true }),
            None,
        )
        .await?;
    let session = attach
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "attach missing sessionId".to_string())?
        .to_string();
    Ok((cdp, session))
}

/// Legacy TCP-WS attach helper, kept for the per-scanner `CdpConn`
/// duplicates that still discover targets via the global
/// `Target.getTargets` walk. New code paths use
/// [`connect_and_attach_matching_in_process`].
pub async fn connect_and_attach_matching<F>(pred: F) -> Result<(CdpConn, String), String>
where
    F: Fn(&CdpTarget) -> bool,
{
    let ws = browser_ws_url().await?;
    let mut cdp = CdpConn::open_ws(&ws).await?;
    let target = find_page_target_where(&mut cdp, pred).await?;
    let attach = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": target.id, "flatten": true }),
            None,
        )
        .await?;
    let session = attach
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "attach missing sessionId".to_string())?
        .to_string();
    Ok((cdp, session))
}

pub async fn detach_session(cdp: &mut CdpConn, session_id: &str) {
    let _ = cdp
        .call(
            "Target.detachFromTarget",
            json!({ "sessionId": session_id }),
            None,
        )
        .await;
}

/// Generalised target search — caller supplies the predicate
/// (url-hash marker, title marker, etc). Used by the per-account
/// session opener, which matches on `#openhuman-account-{id}` so
/// multiple webviews on the same origin don't collide.
pub async fn find_page_target_where<F>(cdp: &mut CdpConn, pred: F) -> Result<CdpTarget, String>
where
    F: Fn(&CdpTarget) -> bool,
{
    let targets_v = cdp.call("Target.getTargets", json!({}), None).await?;
    let targets = parse_targets(&targets_v);
    targets
        .into_iter()
        .find(|t| t.kind == "page" && pred(t))
        .ok_or_else(|| "no matching page target".to_string())
}
