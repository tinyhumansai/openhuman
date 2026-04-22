//! Gmail browser-driven MITM scanner over the Chrome DevTools Protocol.
//!
//! Mirrors the `discord_scanner` / `slack_scanner` architecture:
//!
//!   1. `spawn_scanner` spawns one persistent Tokio task per Gmail account.
//!   2. The task discovers the Gmail page target via `Target.getTargets`,
//!      attaches (`flatten: true`), enables `Network.*` and pumps events:
//!        - `Network.responseReceived` on `mail.google.com/sync/` and
//!          `mail.google.com/mail/u/0/s/` → stored in an in-memory map
//!          (keyed by requestId, capped at `MAX_PENDING_RESPONSES` entries).
//!        - `Network.loadingFinished` for a tracked requestId → issues
//!          `Network.getResponseBody` and emits a `webview:event` with
//!          `kind: "ingest"` / `source: "cdp-http-body"`.
//!
//! ## Concurrency model for body capture
//!
//! `pump_events` owns the WebSocket exclusively (`&mut self`). We need to
//! issue `Network.getResponseBody` inline while still reading further events:
//!
//!   1. Write the request via `send_request` (only writes to sink).
//!   2. Insert `(cdp_id → (oneshot::Sender<Value>, url, mime))` into
//!      `body_pending` — a secondary map parallel to `self.pending`.
//!   3. Continue the read loop. When the matching response arrives, route it
//!      via `body_pending`, extract the body string, and spawn a detached
//!      Tokio task that emits the `webview:event`.
//!
//! The spawned task only needs the pre-extracted strings (body, url, mime,
//! account_id) — no `&mut self` required. Zero extra threads, zero locking
//! in the hot path.
//!
//! Only built with the `cef` feature — the CEF runtime exposes port 9222;
//! WKWebView/wry does not.

pub mod idb;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const CDP_HOST: &str = "127.0.0.1";
const CDP_PORT: u16 = 9222;

/// Back-off between CDP reconnect attempts.
const RECONNECT_BACKOFF: Duration = Duration::from_secs(3);

/// Maximum number of pending response-received entries to track. If the map
/// fills up (e.g. because `loadingFinished` never fires for some requests),
/// the oldest entry is evicted to prevent unbounded growth.
const MAX_PENDING_RESPONSES: usize = 256;

// ---------------------------------------------------------------------------
// Tracked response metadata (stored between responseReceived + loadingFinished)
// ---------------------------------------------------------------------------

/// Metadata stored when we see `Network.responseReceived` for a Gmail sync URL.
#[derive(Debug, Clone)]
struct PendingResponse {
    url: String,
    mime: String,
}

/// Entry in the body-pending map: url + mime carried alongside the pending
/// cdp-id so the read loop can reconstruct the full context when the
/// `getResponseBody` response arrives.
struct BodyPending {
    url: String,
    mime: String,
    request_id: String, // original Gmail requestId string (for logging)
}

// ---------------------------------------------------------------------------
// Public API — spawn scanner
// ---------------------------------------------------------------------------

/// Spawn the per-account Gmail MITM scanner. Idempotent at call site via
/// `ScannerRegistry::ensure_scanner`.
pub fn spawn_scanner<R: Runtime>(app: AppHandle<R>, account_id: String, url_prefix: String) {
    tokio::spawn(async move {
        log::info!(
            "[gmail][{}] mitm up url_prefix={} cdp={}:{}",
            account_id,
            url_prefix,
            CDP_HOST,
            CDP_PORT
        );
        // Let Gmail's sign-in and bootstrap settle before attaching.
        sleep(Duration::from_secs(5)).await;
        loop {
            match run_mitm_session(&app, &account_id, &url_prefix).await {
                Ok(()) => {
                    log::info!(
                        "[gmail][{}] session ended cleanly, reconnecting",
                        account_id
                    );
                }
                Err(e) => {
                    log::warn!(
                        "[gmail][{}] session failed: {} — reconnecting in {:?}",
                        account_id,
                        e,
                        RECONNECT_BACKOFF
                    );
                }
            }
            sleep(RECONNECT_BACKOFF).await;
        }
    });
}

// ---------------------------------------------------------------------------
// Inner session loop (attach → enable → pump events)
// ---------------------------------------------------------------------------

async fn run_mitm_session<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    url_prefix: &str,
) -> Result<(), String> {
    let browser_ws = browser_ws_url().await?;
    let mut cdp = CdpConn::open(&browser_ws).await?;

    // Find the Gmail page target.
    let targets_v = cdp.call("Target.getTargets", json!({}), None).await?;
    let targets = parse_targets(&targets_v);
    log::debug!("[gmail][{}] {} targets total", account_id, targets.len());

    let page = targets
        .iter()
        .find(|t| t.kind == "page" && t.url.starts_with(url_prefix))
        .ok_or_else(|| format!("no page target matching {url_prefix}"))?;
    log::info!(
        "[gmail][{}] attaching to target {} url={}",
        account_id,
        page.id,
        page.url
    );

    let attach = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": page.id, "flatten": true }),
            None,
        )
        .await?;
    let session_id = attach
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "page attach missing sessionId".to_string())?
        .to_string();

    // Enable Network domain — this unlocks responseReceived / loadingFinished.
    cdp.call("Network.enable", json!({}), Some(&session_id))
        .await?;
    log::info!(
        "[gmail][{}] Network.enable ok session={}",
        account_id,
        session_id
    );

    // Drop into the event pump. Returns when the CDP WebSocket closes.
    cdp.pump_events(app, account_id, &session_id).await
}

// ---------------------------------------------------------------------------
// CDP target descriptor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct CdpTarget {
    id: String,
    kind: String,
    url: String,
}

fn parse_targets(v: &Value) -> Vec<CdpTarget> {
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
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn browser_ws_url() -> Result<String, String> {
    let url = format!("http://{CDP_HOST}:{CDP_PORT}/json/version");
    let resp = reqwest::Client::builder()
        .user_agent("openhuman-cdp/1.0")
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("reqwest build: {e}"))?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    let v: Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;
    v.get("webSocketDebuggerUrl")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "no webSocketDebuggerUrl in /json/version".to_string())
}

// ---------------------------------------------------------------------------
// CDP connection
// ---------------------------------------------------------------------------

/// CDP client capable of issuing calls while the read loop (`pump_events`) is
/// running. Two maps drive this:
///
/// - `body_pending`: keyed by CDP integer id, carries the context needed to
///   reconstruct the body event when the `getResponseBody` response arrives
///   inline in the same `pump_events` loop.
///
/// The design avoids any locking: `pump_events` holds `&mut self` throughout
/// and drives both the write (via `send_request`) and the read. When a
/// matching response frame arrives, the context is extracted from
/// `body_pending` synchronously and a detached Tokio task is spawned to emit
/// the body event — no blocking, no deadlock.
pub(crate) struct CdpConn {
    sink: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    stream: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    next_id: i64,
    /// CDP integer id → context for in-flight `getResponseBody` calls.
    body_pending: HashMap<i64, BodyPending>,
}

impl CdpConn {
    pub(crate) async fn open(ws_url: &str) -> Result<Self, String> {
        let (ws, _resp) = connect_async(ws_url)
            .await
            .map_err(|e| format!("ws connect: {e}"))?;
        let (sink, stream) = ws.split();
        Ok(Self {
            sink,
            stream,
            next_id: 1,
            body_pending: HashMap::new(),
        })
    }

    /// Blocking one-shot call — only safe **before** `pump_events`.
    pub(crate) async fn call(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        self.call_with_timeout(method, params, session_id, Duration::from_secs(15))
            .await
    }

    /// Same as `call` but with a configurable timeout. Used by `idb.rs`.
    pub(crate) async fn call_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let mut req = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session_id {
            req["sessionId"] = json!(s);
        }
        let raw = serde_json::to_string(&req).map_err(|e| format!("encode: {e}"))?;
        self.sink
            .send(Message::Text(raw))
            .await
            .map_err(|e| format!("ws send: {e}"))?;
        loop {
            let msg = tokio::time::timeout(timeout, self.stream.next())
                .await
                .map_err(|_| format!("ws read timeout (method={method})"))?
                .ok_or_else(|| format!("ws closed (method={method})"))?
                .map_err(|e| format!("ws recv: {e}"))?;
            let text = match msg {
                Message::Text(t) => t,
                Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                    continue
                }
                Message::Close(_) => return Err("ws closed".into()),
            };
            let v: Value = serde_json::from_str(&text).map_err(|e| format!("decode: {e}"))?;
            if v.get("id").and_then(|x| x.as_i64()) != Some(id) {
                continue;
            }
            if let Some(err) = v.get("error") {
                return Err(format!("cdp error: {err}"));
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Write a CDP request to the sink and return the assigned integer id.
    /// The caller immediately inserts a `BodyPending` entry into
    /// `self.body_pending[id]` before returning control to the read loop.
    ///
    /// Writing to the sink while the read loop is paused at an `await` on
    /// `stream.next()` is safe: Tokio's split halves are independent.
    async fn send_request(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<i64, String> {
        let id = self.next_id;
        self.next_id += 1;
        let mut req = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session_id {
            req["sessionId"] = json!(s);
        }
        let raw = serde_json::to_string(&req).map_err(|e| format!("encode: {e}"))?;
        self.sink
            .send(Message::Text(raw))
            .await
            .map_err(|e| format!("ws send: {e}"))?;
        Ok(id)
    }

    /// Pump CDP events. Returns when the WebSocket closes.
    ///
    /// Body-capture flow (per module-level doc):
    ///   `responseReceived` → store in `tracked`;
    ///   `loadingFinished`  → `send_request` + insert `body_pending`;
    ///   matching response  → extract body, spawn emitter task.
    async fn pump_events<R: Runtime>(
        &mut self,
        app: &AppHandle<R>,
        account_id: &str,
        session_id: &str,
    ) -> Result<(), String> {
        log::info!("[gmail][{}] event pump started", account_id);

        // Bounded map: Gmail requestId (string) → PendingResponse.
        let mut tracked: HashMap<String, PendingResponse> = HashMap::new();
        // Insertion-order tracker for bounded eviction.
        let mut tracked_order: VecDeque<String> = VecDeque::new();

        loop {
            let msg = self
                .stream
                .next()
                .await
                .ok_or_else(|| "ws closed".to_string())?
                .map_err(|e| format!("ws recv: {e}"))?;
            let text = match msg {
                Message::Text(t) => t,
                Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                    continue
                }
                Message::Close(_) => {
                    log::info!("[gmail][{}] cdp ws closed", account_id);
                    return Ok(());
                }
            };
            let v: Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("[gmail][{}] decode failed: {}", account_id, e);
                    continue;
                }
            };

            // ── Response to an in-flight getResponseBody ──────────────────
            if let Some(cdp_id) = v.get("id").and_then(|x| x.as_i64()) {
                if let Some(ctx) = self.body_pending.remove(&cdp_id) {
                    if let Some(err) = v.get("error") {
                        log::warn!(
                            "[gmail][{}] getResponseBody cdp error req_id={}: {}",
                            account_id,
                            ctx.request_id,
                            err
                        );
                    } else {
                        let result = v.get("result").cloned().unwrap_or(Value::Null);
                        let body_str = result
                            .get("body")
                            .and_then(|b| b.as_str())
                            .unwrap_or("")
                            .to_string();
                        let b64 = result
                            .get("base64Encoded")
                            .and_then(|b| b.as_bool())
                            .unwrap_or(false);
                        let body_len = body_str.len();
                        log::debug!(
                            "[gmail][{}] body ok req_id={} bytes={} b64={}",
                            account_id,
                            ctx.request_id,
                            body_len,
                            b64
                        );
                        // Emit on a spawned task so we don't block the loop.
                        let app_c = app.clone();
                        let acct_c = account_id.to_string();
                        let rid_c = ctx.request_id.clone();
                        let url_c = ctx.url.clone();
                        let mime_c = ctx.mime.clone();
                        tokio::spawn(async move {
                            emit(
                                &app_c,
                                &acct_c,
                                "ingest",
                                json!({
                                    "provider": "gmail",
                                    "source": "cdp-http-body",
                                    "request_id": rid_c,
                                    "url": url_c,
                                    "mime_type": mime_c,
                                    "body": body_str,
                                    "base64_encoded": b64,
                                }),
                            );
                        });
                    }
                }
                continue;
            }

            // ── CDP event ─────────────────────────────────────────────────
            let method = v.get("method").and_then(|x| x.as_str()).unwrap_or("");
            let evt_session = v.get("sessionId").and_then(|x| x.as_str()).unwrap_or("");
            if !evt_session.is_empty() && evt_session != session_id {
                continue;
            }
            let params = v.get("params").cloned().unwrap_or(Value::Null);

            match method {
                "Network.responseReceived" => {
                    let url = params
                        .pointer("/response/url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !is_gmail_sync_url(url) {
                        continue;
                    }
                    let status = params
                        .pointer("/response/status")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let mime = params
                        .pointer("/response/mimeType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let request_id = match params
                        .get("requestId")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                    {
                        Some(rid) if !rid.is_empty() => rid,
                        _ => continue,
                    };

                    log::debug!(
                        "[gmail][{}] net← req_id={} status={} mime={}",
                        account_id,
                        request_id,
                        status,
                        mime
                    );

                    // Evict oldest tracked entry if at capacity.
                    if tracked.len() >= MAX_PENDING_RESPONSES {
                        if let Some(oldest) = tracked_order.pop_front() {
                            tracked.remove(&oldest);
                            log::trace!(
                                "[gmail][{}] evicted oldest pending req_id={}",
                                account_id,
                                oldest
                            );
                        }
                    }
                    tracked.insert(
                        request_id.clone(),
                        PendingResponse {
                            url: url.to_string(),
                            mime: mime.clone(),
                        },
                    );
                    tracked_order.push_back(request_id.clone());

                    // Light envelope so the UI knows a sync response is in
                    // flight; the full body follows in "cdp-http-body".
                    emit(
                        app,
                        account_id,
                        "ingest",
                        json!({
                            "provider": "gmail",
                            "source": "cdp-http-response",
                            "request_id": request_id,
                            "url": url,
                            "status": status,
                            "mime_type": mime,
                        }),
                    );
                }

                "Network.loadingFinished" => {
                    let request_id = match params
                        .get("requestId")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                    {
                        Some(r) if !r.is_empty() => r,
                        _ => continue,
                    };

                    let meta = match tracked.remove(&request_id) {
                        Some(m) => m,
                        None => {
                            log::trace!(
                                "[gmail][{}] loadingFinished untracked req_id={}",
                                account_id,
                                request_id
                            );
                            continue;
                        }
                    };
                    tracked_order.retain(|r| r != &request_id);

                    log::debug!(
                        "[gmail][{}] body fetch req_id={} mime={}",
                        account_id,
                        request_id,
                        meta.mime
                    );

                    // Issue the body fetch and plant context in body_pending.
                    // The loop will fulfill it when the response arrives.
                    match self
                        .send_request(
                            "Network.getResponseBody",
                            json!({ "requestId": request_id }),
                            Some(session_id),
                        )
                        .await
                    {
                        Ok(cdp_id) => {
                            self.body_pending.insert(
                                cdp_id,
                                BodyPending {
                                    url: meta.url,
                                    mime: meta.mime,
                                    request_id: request_id.clone(),
                                },
                            );
                            log::trace!(
                                "[gmail][{}] body_pending cdp_id={} req_id={}",
                                account_id,
                                cdp_id,
                                request_id
                            );
                        }
                        Err(e) => {
                            log::warn!(
                                "[gmail][{}] send getResponseBody failed req_id={}: {}",
                                account_id,
                                request_id,
                                e
                            );
                        }
                    }
                }

                _ => {} // drop everything else
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Event filter
// ---------------------------------------------------------------------------

/// Returns true for Gmail's batch-RPC and sync endpoints that carry message
/// data. Covers both the `sync/u/N/i/` batch endpoint and the `mail/u/N/s/`
/// streaming endpoint observed in Gmail's network traffic.
fn is_gmail_sync_url(url: &str) -> bool {
    (url.contains("mail.google.com/sync/") || url.contains("mail.google.com/mail/"))
        && !url.contains("/favicon")
        && !url.contains("/images/")
}

fn emit<R: Runtime>(app: &AppHandle<R>, account_id: &str, kind: &str, payload: Value) {
    let envelope = json!({
        "account_id": account_id,
        "provider": "gmail",
        "kind": kind,
        "payload": payload,
        "ts": now_millis(),
    });
    if let Err(e) = app.emit("webview:event", &envelope) {
        log::warn!("[gmail][{}] emit failed: {}", account_id, e);
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Registry (mirrors discord_scanner::ScannerRegistry)
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct ScannerRegistry {
    started: Mutex<std::collections::HashSet<String>>,
}

impl ScannerRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn ensure_scanner<R: Runtime>(
        self: &Arc<Self>,
        app: AppHandle<R>,
        account_id: String,
        url_prefix: String,
    ) {
        let mut g = self.started.lock().await;
        if !g.insert(account_id.clone()) {
            log::debug!("[gmail] mitm already running for {}", account_id);
            return;
        }
        spawn_scanner(app, account_id, url_prefix);
    }

    pub async fn forget(&self, account_id: &str) {
        let mut g = self.started.lock().await;
        g.remove(account_id);
    }
}

// ---------------------------------------------------------------------------
// Backfill Tauri command (called from GmailPanel "Sync now" button)
// ---------------------------------------------------------------------------

/// Trigger a one-shot IndexedDB backfill for the given Gmail account.
///
/// Opens a fresh CDP connection (independent of the live MITM session),
/// enumerates Gmail's IndexedDB databases, pages through records, and emits
/// each as a `webview:event` with `source: "cdp-idb-record"`.
/// Returns the total number of records emitted.
#[tauri::command]
pub async fn gmail_scanner_backfill<R: Runtime>(
    app: AppHandle<R>,
    account_id: String,
) -> Result<usize, String> {
    log::info!("[gmail][{}] backfill command received", account_id);
    idb::backfill(&app, &account_id).await
}
