//! IndexedDB scanner driven over the Chrome DevTools Protocol (CDP).
//!
//! We talk to the embedded CEF instance through its remote-debugging port
//! (set via `--remote-debugging-port=9222` in `lib.rs`). For each tracked
//! webview-account target, we periodically:
//!
//!   1. Discover the right CDP page target by URL prefix
//!      (`https://web.whatsapp.com/`).
//!   2. Open its DevTools WebSocket and send `Runtime.evaluate` with the
//!      bundled `scanner.js` payload.
//!   3. The script runs inside the page (so it has access to the
//!      non-extractable `CryptoKey` already resident there), reads
//!      IndexedDB, decrypts envelopes via `crypto.subtle.decrypt`, and
//!      returns a normalized snapshot as JSON.
//!   4. Rust forwards the snapshot to React via the existing
//!      `webview:event` Tauri event so the UI / persistence layer can
//!      consume it without a second pipeline.
//!
//! No DOM scrape, no Tauri-IPC-from-injected-JS, no CSP fight. We "own the
//! browser" through CDP.
//!
//! NOTE: this module is only meaningful with the `cef` feature — the wry
//! runtime does not expose a remote debugging port. We compile-gate the
//! task spawn at the call site.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const CDP_HOST: &str = "127.0.0.1";
const CDP_PORT: u16 = 9222;
/// Cadence for the expensive full scan (IDB walk, spy snapshot, schema
/// dumps). Runs infrequently because each pass serialises thousands of
/// message records.
const FULL_SCAN_INTERVAL: Duration = Duration::from_secs(30);
/// Cadence for the cheap fast scan (DOM `[data-id]` scrape only). Runs at
/// Franz-like 2s so the ingest stream feels live — each tick only hits
/// `document.querySelectorAll` and serialises rendered rows.
const FAST_SCAN_INTERVAL: Duration = Duration::from_secs(2);
/// Inline scan script, executed via `Runtime.evaluate` per full tick.
const SCANNER_JS: &str = include_str!("scanner.js");
/// Fast-tick DOM-only scrape script. Returns `{ok, domMessages, hash}`.
/// `hash` is a rolling FNV-1a over (dataId, body) so the Rust side can
/// skip emission when the visible set hasn't changed.
const DOM_SCAN_JS: &str = include_str!("dom_scan.js");
// The worker_spy / worker_hook / force-extractable scripts are retained in
// the source tree for reference (we explored worker-side crypto capture +
// CSP bypass before pivoting to pure DOM scraping) but are not wired into
// the active scan path — no page reload, no hook install, minimal user
// disruption.
#[allow(dead_code)]
const WORKER_SPY_JS: &str = include_str!("worker_spy.js");
#[allow(dead_code)]
const WORKER_HOOK_JS_RAW: &str = include_str!("worker_hook.js");
#[allow(dead_code)]
const FORCE_EXTRACTABLE_JS: &str = include_str!("force_extractable.js");

/// One CDP target descriptor (from `Target.getTargets`).
#[derive(Debug, Clone)]
struct CdpTarget {
    id: String,
    kind: String,
    url: String,
}

/// Snapshot returned by `scanner.js`. Mirrors the JS shape verbatim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSnapshot {
    pub ok: bool,
    #[serde(rename = "scannedAt", default)]
    pub scanned_at: i64,
    #[serde(default)]
    pub dbs: Vec<String>,
    #[serde(default)]
    pub chats: serde_json::Map<String, Value>,
    #[serde(default)]
    pub messages: Vec<Value>,
    #[serde(rename = "hadKey", default)]
    pub had_key: bool,
    #[serde(default)]
    pub error: Option<String>,
    /// Up to N most-recent decrypted messages (body preview only) — useful
    /// to confirm decryption produced real text and not garbage. Each
    /// entry: { chatId, chatName, from, fromMe, timestamp, bodyPreview }.
    #[serde(rename = "sampleMessages", default)]
    pub sample_messages: Vec<Value>,
    /// DOM-scraped rendered message bodies (chat currently open in the
    /// webview). WhatsApp doesn't re-decrypt msgRowOpaqueData via
    /// crypto.subtle when rendering, so we read the rendered DOM directly
    /// and join to IndexedDB metadata via the data-id attribute.
    #[serde(rename = "domMessages", default)]
    pub dom_messages: Vec<Value>,
    /// Total CryptoKey objects discovered across all DBs/stores.
    #[serde(rename = "keyCount", default)]
    pub key_count: usize,
    /// Where each CryptoKey was found, in priority order.
    #[serde(rename = "keySources", default)]
    pub key_sources: Vec<String>,
    /// Number of CryptoKeys harvested from `wawc_db_enc/keys` (indexed by
    /// `_keyId` for `msgRowOpaqueData` decryption).
    #[serde(rename = "keyByIdCount", default)]
    pub key_by_id_count: usize,
    /// First few record-keys observed in `wawc_db_enc/keys` — used to
    /// confirm the keyId space matches `msgRowOpaqueData._keyId` values.
    #[serde(rename = "keyByIdSampleIds", default)]
    pub key_by_id_sample_ids: Vec<String>,
    /// Per-scan decrypt counters: how many opaque envelopes we saw, how
    /// many decrypted, how many yielded text, and the histogram of
    /// `_keyId` values seen on messages. Plus a hex preview of the first
    /// successful decrypt (first 64 bytes) so we can see the wire format.
    #[serde(rename = "decryptStats", default)]
    pub decrypt_stats: Option<Value>,
    /// Shape of the first `wawc_db_enc/keys` record + every CryptoKey in
    /// it (path, algorithm, usages). Tells us if we're picking the wrong
    /// key (e.g. HMAC vs AES-GCM).
    #[serde(rename = "keystoreSample", default)]
    pub keystore_sample: Option<Value>,
    /// Captured `crypto.subtle.{deriveKey,deriveBits,decrypt}` calls from
    /// the WhatsApp page (debug spy installed by scanner.js). Tells us
    /// the exact (info, salt) parameters WA uses for HKDF.
    #[serde(rename = "cryptoSpy", default)]
    pub crypto_spy: Option<Value>,
    /// Per-WORKER spy dumps — collected via Worker constructor wrapper
    /// (worker_hook.js) + postMessage round-trip. `wrappedCount` is the
    /// number of currently-tracked workers; `replies` is whatever each
    /// one replied with (HKDF derives + AES-GCM decrypt sizes).
    #[serde(rename = "workerSpies", default)]
    pub worker_spies: Option<Value>,
    /// `window.*` keys matching encryption/local-storage patterns —
    /// would let us call WA's own decryption helper directly if exposed.
    #[serde(rename = "windowGlobals", default)]
    pub window_globals: Vec<String>,
    /// Union of all top-level field names observed across every message
    /// record, with the type signature(s) seen for each. Surfaces fields
    /// like `body` that only appear on certain message types.
    #[serde(rename = "messageKeyUnion", default)]
    pub message_key_union: Option<Value>,
    /// `type → count` over every scanned message — tells us at a glance
    /// how many text vs media vs system messages we actually have.
    #[serde(rename = "messageTypeBreakdown", default)]
    pub message_type_breakdown: Option<Value>,
    /// `type → shape(firstRecord)` so we can see one full example per
    /// message type rather than just the first record overall.
    #[serde(rename = "sampleByType", default)]
    pub sample_by_type: Option<Value>,
    /// First-record shape per "interesting" store name (excluding `message`
    /// itself, which has its own dedicated diagnostics above).
    #[serde(rename = "schemaDump", default)]
    pub schema_dump: serde_json::Map<String, Value>,
    /// OPFS listing — WhatsApp may persist bodies in a SQLite-via-WASM
    /// file in the Origin Private File System rather than IndexedDB.
    #[serde(default)]
    pub opfs: Option<Value>,
    /// Map of dbName → object store names. Logged once per scan so we can
    /// see WhatsApp's actual layout and tighten the store-name hints.
    #[serde(rename = "storeMap", default)]
    pub store_map: serde_json::Map<String, Value>,
}

/// Spawn a per-account CDP poller. Idempotent at call site (caller tracks
/// account → JoinHandle if it cares about cancellation).
///
/// The scanner runs two interleaved loops:
///   * **Fast tick** (`FAST_SCAN_INTERVAL`, 2s) — cheap DOM scrape. Only
///     emits an ingest event when the visible-row hash changes, so idle
///     windows don't spam the UI.
///   * **Full tick** (`FULL_SCAN_INTERVAL`, 30s) — the expensive IDB walk
///     + spy/keystore snapshot. Always emits.
///
/// Both ticks share the same `webview:event` ingest envelope so downstream
/// consumers don't need to care which one produced the event.
pub fn spawn_scanner<R: Runtime>(app: AppHandle<R>, account_id: String, url_prefix: String) {
    tokio::spawn(async move {
        log::info!(
            "[cdp] scanner up account={} url_prefix={} fast={:?} full={:?}",
            account_id,
            url_prefix,
            FAST_SCAN_INTERVAL,
            FULL_SCAN_INTERVAL
        );
        // Wait a moment for the page to actually load + log in. We'd rather
        // miss the first cycle than thrash the CDP endpoint while the
        // target isn't even there yet.
        sleep(Duration::from_secs(5)).await;
        let mut last_dom_hash: Option<u64> = None;
        let mut last_full: Instant = Instant::now()
            .checked_sub(FULL_SCAN_INTERVAL)
            .unwrap_or_else(Instant::now);
        loop {
            // Gate: run a full IDB scan if enough time has elapsed,
            // otherwise run the cheap DOM-only scan.
            let do_full = last_full.elapsed() >= FULL_SCAN_INTERVAL;
            if !do_full {
                match scan_dom_once(&account_id, &url_prefix).await {
                    Ok(dom) => {
                        if dom.ok {
                            let changed = last_dom_hash != Some(dom.hash)
                                && !dom.dom_messages.is_empty();
                            if changed {
                                log::info!(
                                    "[cdp][{}] fast dom-scan rows={} hash={} (changed)",
                                    account_id,
                                    dom.dom_messages.len(),
                                    dom.hash
                                );
                                emit_dom_only(&app, &account_id, &dom.dom_messages);
                                last_dom_hash = Some(dom.hash);
                            }
                        } else if let Some(err) = dom.error {
                            log::debug!("[cdp][{}] dom-scan err: {}", account_id, err);
                        }
                    }
                    Err(e) => {
                        log::debug!("[cdp][{}] dom-scan failed: {}", account_id, e);
                    }
                }
                sleep(FAST_SCAN_INTERVAL).await;
                continue;
            }
            last_full = Instant::now();
            match scan_once(&app, &account_id, &url_prefix).await {
                Ok(snap) => {
                    log::info!(
                        "[cdp][{}] scan ok dbs={} messages={} chats={} keys={} sources={:?} keyById={} sampleIds={:?}",
                        account_id,
                        snap.dbs.len(),
                        snap.messages.len(),
                        snap.chats.len(),
                        snap.key_count,
                        snap.key_sources,
                        snap.key_by_id_count,
                        snap.key_by_id_sample_ids,
                    );
                    if let Some(ref ds) = snap.decrypt_stats {
                        log::info!("[cdp][{}] decrypt {}", account_id, ds);
                    }
                    if let Some(ref ks) = snap.keystore_sample {
                        log::info!("[cdp][{}] keystore {}", account_id, ks);
                    }
                    if let Some(ref spy) = snap.crypto_spy {
                        log::info!("[cdp][{}] spy {}", account_id, spy);
                    }
                    if let Some(ref ws) = snap.worker_spies {
                        log::info!("[cdp][{}] worker-spies {}", account_id, ws);
                    }
                    if !snap.window_globals.is_empty() {
                        log::info!(
                            "[cdp][{}] globals {:?}",
                            account_id,
                            snap.window_globals
                        );
                    }
                    if let Some(ref types) = snap.message_type_breakdown {
                        log::info!("[cdp][{}] msg-types {}", account_id, types);
                    }
                    if let Some(ref union) = snap.message_key_union {
                        log::info!("[cdp][{}] msg-key-union {}", account_id, union);
                    }
                    if let Some(ref by_type) = snap.sample_by_type {
                        if let Some(map) = by_type.as_object() {
                            for (t, shape) in map {
                                log::info!("[cdp][{}] msg-shape type={} {}", account_id, t, shape);
                            }
                        }
                    }
                    for (store, shape) in &snap.schema_dump {
                        log::info!("[cdp][{}] schema {} {}", account_id, store, shape);
                    }
                    if let Some(ref opfs) = snap.opfs {
                        log::info!("[cdp][{}] opfs {}", account_id, opfs);
                    }
                    for (i, sample) in snap.sample_messages.iter().enumerate() {
                        let chat_name = sample
                            .get("chatName")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        let chat_id = sample
                            .get("chatId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        let from = if sample
                            .get("fromMe")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        {
                            "me".to_string()
                        } else {
                            sample
                                .get("from")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?")
                                .to_string()
                        };
                        let ts = sample.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
                        let body = sample
                            .get("bodyPreview")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        log::info!(
                            "[cdp][{}] msg#{} ts={} chat={} ({}) from={} body={:?}",
                            account_id,
                            i + 1,
                            ts,
                            chat_name,
                            chat_id,
                            from,
                            body
                        );
                    }
                    // DOM-scraped message bodies (the chat the user has open).
                    if !snap.dom_messages.is_empty() {
                        log::info!(
                            "[cdp][{}] dom-scrape count={}",
                            account_id,
                            snap.dom_messages.len()
                        );
                        for (i, dm) in snap.dom_messages.iter().take(5).enumerate() {
                            let chat = dm.get("chatId").and_then(|v| v.as_str()).unwrap_or("?");
                            let msg = dm.get("msgId").and_then(|v| v.as_str()).unwrap_or("?");
                            let from_me = dm
                                .get("fromMe")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            let author = dm
                                .get("author")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let ts = dm
                                .get("preTimestamp")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let body = dm.get("body").and_then(|v| v.as_str()).unwrap_or("");
                            let body_preview = if body.len() > 120 {
                                format!("{}…", &body[..120])
                            } else {
                                body.to_string()
                            };
                            log::info!(
                                "[cdp][{}] dom#{} chat={} msg={} fromMe={} [{}] {}: {:?}",
                                account_id,
                                i + 1,
                                chat,
                                msg,
                                from_me,
                                ts,
                                author,
                                body_preview
                            );
                        }
                    }
                    if !snap.store_map.is_empty() {
                        // Compact one-liner so we can grep store layouts.
                        let layout = snap
                            .store_map
                            .iter()
                            .map(|(db, stores)| {
                                let names = stores
                                    .as_array()
                                    .map(|a| {
                                        a.iter()
                                            .filter_map(|v| v.as_str())
                                            .collect::<Vec<_>>()
                                            .join(",")
                                    })
                                    .unwrap_or_default();
                                format!("{db}:[{names}]")
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        log::info!("[cdp][{}] stores {}", account_id, layout);
                    }
                    emit_snapshot(&app, &account_id, &snap);
                }
                Err(e) => {
                    log::warn!("[cdp][{}] scan failed: {}", account_id, e);
                }
            }
            // After a full scan, go back to fast-tick cadence until the
            // next `FULL_SCAN_INTERVAL` elapses.
            sleep(FAST_SCAN_INTERVAL).await;
        }
    });
}

/// Emit an ingest payload carrying only DOM-scraped rows, grouped by
/// (chatId, day) so React can upsert each day's transcript into memory.
fn emit_dom_only<R: Runtime>(app: &AppHandle<R>, account_id: &str, dom: &[Value]) {
    // Use the most recent contact-names snapshot from a full IDB scan so
    // DOM-only rows get resolved display names too.
    let names = contact_cache_get(account_id);
    emit_grouped_whatsapp(app, account_id, dom, &names, "cdp-dom");
}

/// Per-account snapshot of `{jid -> display name}`. Populated on every
/// full IDB scan (from chats / contacts / group-metadata stores) and read
/// by fast DOM-only ticks so the transcript lines show names instead of
/// raw JIDs even when the scrape comes from the DOM.
fn contact_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, serde_json::Map<String, Value>>> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, serde_json::Map<String, Value>>>> = OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(Default::default()))
}

fn contact_cache_put(account_id: &str, names: &serde_json::Map<String, Value>) {
    if names.is_empty() {
        return;
    }
    let mut g = contact_cache().lock().unwrap();
    g.insert(account_id.to_string(), names.clone());
}

fn contact_cache_get(account_id: &str) -> serde_json::Map<String, Value> {
    let g = contact_cache().lock().unwrap();
    g.get(account_id).cloned().unwrap_or_default()
}

fn chrono_now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

async fn scan_once<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    url_prefix: &str,
) -> Result<ScanSnapshot, String> {
    // One browser-level CDP connection per scan tick. We use it to attach
    // to BOTH the WhatsApp page AND every worker target via
    // `Target.attachToTarget` — workers can't be debugged via direct WS,
    // they must be reached as nested sessions on the browser endpoint.
    let browser_ws = browser_ws_url().await?;
    let mut cdp = CdpConn::open(&browser_ws).await?;

    // 1. Enumerate every CDP target.
    let targets_v = cdp
        .call("Target.getTargets", json!({}), None)
        .await?;
    let targets = parse_targets(&targets_v);
    log::debug!("[cdp][{}] {} targets total", account_id, targets.len());

    // We don't probe worker targets directly — CEF workers never answer
    // Runtime.evaluate calls (confirmed empirically) so each attempt would
    // waste ~10s. The page-level spy in SCANNER_JS captures everything we
    // need. Leave the worker_spy.js / worker_hook.js machinery in the tree
    // for reference; it's intentionally not called from the hot path.

    // Run the full IndexedDB scanner against the WhatsApp page.
    let page_target = targets
        .iter()
        .find(|t| t.kind == "page" && t.url.starts_with(url_prefix))
        .ok_or_else(|| format!("no page target matching {url_prefix}"))?;
    let attach = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": page_target.id, "flatten": true }),
            None,
        )
        .await?;
    let page_session = attach
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "page attach missing sessionId".to_string())?
        .to_string();

    // NOTE: previously we installed a Worker constructor hook via
    // Page.addScriptToEvaluateOnNewDocument and reloaded the page so
    // workers would respawn through our wrapper. That path proved a dead
    // end (WA decrypts message bodies outside crypto.subtle in a way we
    // can't capture) and the reload disrupts the user. DOM scraping +
    // IDB reading from SCANNER_JS is enough — no hook, no reload.

    let scan_v = cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": SCANNER_JS,
                "awaitPromise": true,
                "returnByValue": true,
                "timeout": 30_000,
            }),
            Some(&page_session),
        )
        .await?;
    if let Some(exc) = scan_v.pointer("/exceptionDetails") {
        return Err(format!("page scanner threw: {exc}"));
    }
    let value = scan_v
        .pointer("/result/value")
        .ok_or_else(|| format!("page scanner missing result/value: {scan_v}"))?
        .clone();
    let snap: ScanSnapshot =
        serde_json::from_value(value).map_err(|e| format!("decode snapshot: {e}"))?;
    let _ = cdp
        .call(
            "Target.detachFromTarget",
            json!({ "sessionId": page_session }),
            None,
        )
        .await;
    let _ = app;
    Ok(snap)
}

/// Result of a fast DOM-only scan. Small enough to bounce back every 2s.
#[derive(Deserialize, Debug, Default)]
pub struct DomScanResult {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(rename = "domMessages", default)]
    pub dom_messages: Vec<Value>,
    #[serde(default)]
    pub hash: u64,
}

/// Fast tick: open a CDP session, attach to the WhatsApp page, run only
/// the DOM scrape script, detach. No IDB, no spy, no target enumeration of
/// workers. Returns the cheap `DomScanResult`.
async fn scan_dom_once(
    account_id: &str,
    url_prefix: &str,
) -> Result<DomScanResult, String> {
    let browser_ws = browser_ws_url().await?;
    let mut cdp = CdpConn::open(&browser_ws).await?;
    let targets_v = cdp.call("Target.getTargets", json!({}), None).await?;
    let targets = parse_targets(&targets_v);
    let page_target = targets
        .iter()
        .find(|t| t.kind == "page" && t.url.starts_with(url_prefix))
        .ok_or_else(|| format!("no page target matching {url_prefix}"))?;
    let attach = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": page_target.id, "flatten": true }),
            None,
        )
        .await?;
    let page_session = attach
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "page attach missing sessionId".to_string())?
        .to_string();
    let scan_v = cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": DOM_SCAN_JS,
                "awaitPromise": false,
                "returnByValue": true,
                "timeout": 2_000,
            }),
            Some(&page_session),
        )
        .await?;
    if let Some(exc) = scan_v.pointer("/exceptionDetails") {
        let _ = cdp
            .call(
                "Target.detachFromTarget",
                json!({ "sessionId": page_session }),
                None,
            )
            .await;
        return Err(format!("dom-scan threw: {exc}"));
    }
    let value = scan_v
        .pointer("/result/value")
        .cloned()
        .ok_or_else(|| format!("dom-scan missing result/value: {scan_v}"))?;
    let _ = cdp
        .call(
            "Target.detachFromTarget",
            json!({ "sessionId": page_session }),
            None,
        )
        .await;
    let result: DomScanResult =
        serde_json::from_value(value).map_err(|e| format!("decode dom-scan: {e}"))?;
    log::debug!(
        "[cdp][{}] fast dom-scan rows={} hash={}",
        account_id,
        result.dom_messages.len(),
        result.hash
    );
    Ok(result)
}

#[allow(dead_code)]
fn short(id: &str) -> &str {
    &id[..8.min(id.len())]
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
                        url: t.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Discover the browser-level WebSocket endpoint via `/json/version`.
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

/// Minimal CDP request/response client: keeps a WebSocket open, sends
/// JSON-RPC requests with auto-incrementing ids, awaits the matching
/// response. Inbound CDP events (no `id`) and unrelated responses are
/// drained but ignored. Not concurrent — `call` is sequential.
struct CdpConn {
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
}

impl CdpConn {
    async fn open(ws_url: &str) -> Result<Self, String> {
        let (ws, _resp) = connect_async(ws_url)
            .await
            .map_err(|e| format!("ws connect: {e}"))?;
        let (sink, stream) = ws.split();
        Ok(Self {
            sink,
            stream,
            next_id: 1,
        })
    }

    async fn call(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let mut req = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session_id {
            req["sessionId"] = json!(s);
        }
        let body = serde_json::to_string(&req).map_err(|e| format!("encode: {e}"))?;
        self.sink
            .send(Message::Text(body))
            .await
            .map_err(|e| format!("ws send: {e}"))?;

        loop {
            let msg = tokio::time::timeout(Duration::from_secs(35), self.stream.next())
                .await
                .map_err(|_| format!("ws read timeout (method={method})"))?
                .ok_or_else(|| format!("ws closed (method={method})"))?
                .map_err(|e| format!("ws recv: {e}"))?;
            let text = match msg {
                Message::Text(t) => t,
                Message::Binary(_)
                | Message::Ping(_)
                | Message::Pong(_)
                | Message::Frame(_) => continue,
                Message::Close(_) => return Err("ws closed".into()),
            };
            let v: Value = serde_json::from_str(&text).map_err(|e| format!("decode: {e}"))?;
            // Skip CDP events (have `method` instead of `id`) + responses
            // for other ids.
            if v.get("id").and_then(|x| x.as_i64()) != Some(id) {
                continue;
            }
            if let Some(err) = v.get("error") {
                return Err(format!("cdp error: {err}"));
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}


/// Forward the snapshot to React via the same `webview:event` channel
/// recipe ingest already uses. UI code can listen for kind == "ingest".
fn emit_snapshot<R: Runtime>(app: &AppHandle<R>, account_id: &str, snap: &ScanSnapshot) {
    if !snap.ok {
        log::warn!(
            "[cdp][{}] snapshot not ok, error={:?}",
            account_id,
            snap.error
        );
        return;
    }
    // Join DOM-scraped bodies into the messages list by msgId. WhatsApp
    // caches decrypted bodies in memory, so IndexedDB gives us metadata and
    // the DOM gives us text for currently-rendered chats — unioning them
    // here gives downstream consumers a single message list.
    let mut messages = snap.messages.clone();
    if !snap.dom_messages.is_empty() {
        use std::collections::{HashMap, HashSet};
        // Index DOM rows by BOTH full `data-id` ("true_chatId_msgId") AND
        // bare msgId — IDB's `_serialized` matches data-id but some paths
        // use just the msgId. Each map entry keeps its source dataId so a
        // patch via either key only consumes the row once.
        let mut by_msg_id: HashMap<String, (String, Value)> = HashMap::new();
        for dm in &snap.dom_messages {
            let did = dm
                .get("dataId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if did.is_empty() {
                continue;
            }
            by_msg_id.insert(did.clone(), (did.clone(), dm.clone()));
            if let Some(mid) = dm.get("msgId").and_then(|v| v.as_str()) {
                by_msg_id
                    .entry(mid.to_string())
                    .or_insert_with(|| (did.clone(), dm.clone()));
            }
        }
        let mut consumed: HashSet<String> = HashSet::new();
        let mut patched = 0usize;
        for m in messages.iter_mut() {
            let mid_opt = m
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let has_body = m
                .get("body")
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if has_body {
                continue;
            }
            if let Some(mid) = mid_opt {
                if let Some((did, dm)) = by_msg_id.get(&mid).cloned() {
                    if consumed.contains(&did) {
                        continue;
                    }
                    if let Some(body) = dm.get("body").and_then(|v| v.as_str()) {
                        if let Some(obj) = m.as_object_mut() {
                            obj.insert("body".to_string(), json!(body));
                            obj.insert("bodySource".to_string(), json!("dom"));
                            patched += 1;
                            consumed.insert(did);
                        }
                    }
                }
            }
        }
        // Unmatched DOM rows with a body get appended (dedup by dataId).
        let mut appended = 0usize;
        let mut appended_dids: HashSet<String> = HashSet::new();
        for (_key, (did, dm)) in by_msg_id {
            if consumed.contains(&did) || appended_dids.contains(&did) {
                continue;
            }
            if dm
                .get("body")
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false)
            {
                messages.push(json!({
                    "id": dm.get("dataId").cloned().unwrap_or(Value::Null),
                    "chatId": dm.get("chatId").cloned().unwrap_or(Value::Null),
                    "fromMe": dm.get("fromMe").cloned().unwrap_or(Value::Null),
                    "body": dm.get("body").cloned().unwrap_or(Value::Null),
                    "author": dm.get("author").cloned().unwrap_or(Value::Null),
                    "preTimestamp": dm.get("preTimestamp").cloned().unwrap_or(Value::Null),
                    "bodySource": "dom-only",
                }));
                appended += 1;
                appended_dids.insert(did);
            }
        }
        log::info!(
            "[cdp][{}] dom-merge patched={} appended={} total={}",
            account_id,
            patched,
            appended,
            messages.len()
        );
    }
    // Cache the contact/chat name map so the next fast DOM-only tick can
    // resolve sender JIDs → display names without re-walking IDB.
    contact_cache_put(account_id, &snap.chats);
    // Also emit one grouped `whatsapp` ingest event per (chatId, day) so
    // the React listener can call `openhuman.memory_doc_ingest` with a
    // stable namespace/key that upserts cleanly.
    emit_grouped_whatsapp(app, account_id, &messages, &snap.chats, "cdp-indexeddb");
}

/// Parse a unix-seconds timestamp to a UTC `YYYY-MM-DD` string. Uses the
/// Howard Hinnant civil-from-days algorithm — no external deps.
fn seconds_to_ymd(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y_real = (if m <= 2 { y + 1 } else { y }) as i32;
    format!("{:04}-{:02}-{:02}", y_real, m, d)
}

/// Parse WA's `data-pre-plain-text` timestamp (e.g. `"4:53 AM, 7/5/2025"`)
/// to `YYYY-MM-DD`. Returns None if the format doesn't match.
fn parse_pre_timestamp_ymd(s: &str) -> Option<String> {
    // Everything after the first comma is the date: "4:53 AM, 7/5/2025"
    let (_, date_part) = s.split_once(',')?;
    let date_part = date_part.trim();
    let parts: Vec<&str> = date_part.split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let m: u32 = parts[0].trim().parse().ok()?;
    let d: u32 = parts[1].trim().parse().ok()?;
    let y: i32 = parts[2].trim().parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1900..=3000).contains(&y) {
        return None;
    }
    Some(format!("{:04}-{:02}-{:02}", y, m, d))
}

/// Group messages by (chatId, day) and emit one `webview:event` per group
/// matching the shape `persistWhatsappChatDay` (React) consumes. React in
/// turn calls `openhuman.memory_doc_ingest` to upsert each day's transcript
/// into the memory layer.
fn emit_grouped_whatsapp<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    messages: &[Value],
    chats: &serde_json::Map<String, Value>,
    source: &str,
) {
    use std::collections::HashMap;
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // Group: (chatId, day) -> Vec<normalized message>
    let mut groups: HashMap<(String, String), Vec<Value>> = HashMap::new();
    for m in messages {
        let chat_id = match m.get("chatId").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        // Require body — memory docs without content are noise.
        let body = m
            .get("body")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if body.is_empty() {
            continue;
        }

        // Derive day + canonical timestamp (seconds).
        let (day, ts_secs): (String, i64) = if let Some(t) = m.get("timestamp").and_then(|v| v.as_i64()) {
            (seconds_to_ymd(t), t)
        } else if let Some(pre) = m.get("preTimestamp").and_then(|v| v.as_str()) {
            match parse_pre_timestamp_ymd(pre) {
                Some(d) => (d, now_secs),
                None => (seconds_to_ymd(now_secs), now_secs),
            }
        } else {
            (seconds_to_ymd(now_secs), now_secs)
        };

        // React expects `fromMe`, `from`, `body`, `timestamp` (sec), `type`.
        let from_me = m.get("fromMe").and_then(|v| v.as_bool()).unwrap_or(false);
        let raw_from: Option<String> = m
            .get("from")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        // Prefer: chats[from].name → DOM `author` (parsed from data-pre-plain-text)
        //       → chats[chatId].name (1:1 chats where chatId == sender)
        //       → raw JID as last resort.
        let author_from_dom = m
            .get("author")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let resolved_name: Option<String> = raw_from
            .as_ref()
            .and_then(|jid| {
                chats
                    .get(jid)
                    .and_then(|c| c.get("name"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .or(author_from_dom)
            .or_else(|| {
                chats
                    .get(&chat_id)
                    .and_then(|c| c.get("name"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });
        // `from` field keeps the JID so downstream code can key by it;
        // `fromName` carries the human-readable label for the transcript.
        let from_value = raw_from
            .clone()
            .or_else(|| resolved_name.clone())
            .unwrap_or_default();
        let id = m
            .get("id")
            .cloned()
            .or_else(|| m.get("dataId").cloned())
            .unwrap_or(Value::Null);
        let type_ = m
            .get("type")
            .cloned()
            .unwrap_or(Value::Null);
        let normalized = json!({
            "id": id,
            "chatId": chat_id.clone(),
            "fromMe": from_me,
            "from": from_value,
            "fromName": resolved_name,
            "body": body,
            "timestamp": ts_secs,
            "type": type_,
        });
        groups.entry((chat_id, day)).or_default().push(normalized);
    }

    // Emit one event per (chatId, day). Match envelope shape React expects
    // so when the main window IS open the UI updates live. In parallel we
    // POST the same payload directly to the core RPC so the memory write
    // happens regardless of whether the React listener is attached.
    let mut emitted = 0usize;
    for ((chat_id, day), msgs) in groups {
        let chat_name = chats
            .get(&chat_id)
            .and_then(|c| c.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or(&chat_id)
            .to_string();
        let payload = json!({
            "provider": "whatsapp",
            "source": source,
            "chatId": chat_id,
            "chatName": chat_name,
            "day": day,
            "messages": msgs,
        });
        let envelope = json!({
            "account_id": account_id,
            "provider": "whatsapp",
            "kind": "ingest",
            "payload": payload.clone(),
            "ts": chrono_now_millis(),
        });
        if let Err(e) = app.emit("webview:event", &envelope) {
            log::warn!("[cdp][{}] ingest emit failed: {}", account_id, e);
        } else {
            emitted += 1;
        }
        // Direct memory write via core RPC — fire-and-forget so the
        // scanner tick doesn't block on HTTP.
        let acct = account_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = post_memory_doc_ingest(&acct, &payload).await {
                log::warn!("[cdp][{}] memory write failed: {}", acct, e);
            }
        });
    }
    if emitted > 0 {
        log::info!(
            "[cdp][{}] emitted {} ingest group(s) source={}",
            account_id,
            emitted,
            source
        );
    }
}

/// Resolve the core JSON-RPC URL — same rule as the `core_rpc_url` Tauri
/// command in lib.rs: env var or default loopback port.
fn core_rpc_url_value() -> String {
    std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:7788/rpc".to_string())
}

/// Build the `openhuman.memory_doc_ingest` payload for a single
/// (chatId, day) group and POST it directly to the core. The shape
/// mirrors `persistWhatsappChatDay` on the React side so the memory docs
/// line up whether the scanner or the UI drove the ingest.
async fn post_memory_doc_ingest(account_id: &str, ingest: &Value) -> Result<(), String> {
    let chat_id = ingest
        .get("chatId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let day = ingest.get("day").and_then(|v| v.as_str()).unwrap_or_default();
    let chat_name = ingest
        .get("chatName")
        .and_then(|v| v.as_str())
        .unwrap_or(chat_id);
    let empty: Vec<Value> = Vec::new();
    let msgs: &Vec<Value> = ingest
        .get("messages")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    if chat_id.is_empty() || day.is_empty() || msgs.is_empty() {
        return Ok(());
    }

    // Build a stable transcript — sorted by timestamp, one line per msg.
    let mut sorted: Vec<&Value> = msgs.iter().collect();
    sorted.sort_by_key(|m| m.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0));
    let transcript: String = sorted
        .iter()
        .map(|m| {
            let ts = m.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
            let hhmm = if ts > 0 {
                let secs_of_day = (ts.rem_euclid(86_400)) as u32;
                format!("{:02}:{:02}Z", secs_of_day / 3600, (secs_of_day / 60) % 60)
            } else {
                "--:--".to_string()
            };
            let who = if m.get("fromMe").and_then(|v| v.as_bool()).unwrap_or(false) {
                "me".to_string()
            } else {
                // Prefer the resolved display name; fall back to raw JID
                // (the "from" field), then "?".
                m.get("fromName")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| m.get("from").and_then(|v| v.as_str()))
                    .filter(|s| !s.is_empty())
                    .unwrap_or("?")
                    .to_string()
            };
            let body = m
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .replace(['\r', '\n'], " ");
            let type_ = m
                .get("type")
                .and_then(|v| v.as_str())
                .filter(|t| *t != "chat" && !t.is_empty())
                .map(|t| format!(" [{t}]"))
                .unwrap_or_default();
            format!("[{hhmm}] {who}{type_}: {body}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let header = format!(
        "# WhatsApp — {chat_name} — {day}\nchat_id: {chat_id}\naccount_id: {account_id}\nmessages: {n}\n\n",
        n = sorted.len()
    );
    let content = format!("{header}{transcript}");

    let namespace = format!("whatsapp-web:{account_id}");
    let key = format!("{chat_id}:{day}");
    let title = format!("WhatsApp · {chat_name} · {day}");

    let params = json!({
        "namespace": namespace,
        "key": key,
        "title": title,
        "content": content,
        "source_type": "whatsapp-web",
        "priority": "medium",
        "tags": ["whatsapp", "chat-transcript", day],
        "metadata": {
            "provider": "whatsapp",
            "account_id": account_id,
            "chat_id": chat_id,
            "chat_name": chat_name,
            "day": day,
            "message_count": sorted.len(),
        },
        "category": "core",
    });
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "openhuman.memory_doc_ingest",
        "params": params,
    });

    let url = core_rpc_url_value();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
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
    let v: Value = resp.json().await.map_err(|e| format!("decode: {e}"))?;
    if let Some(err) = v.get("error") {
        return Err(format!("rpc error: {err}"));
    }
    log::info!(
        "[cdp][{}] memory upsert ok namespace={} key={} msgs={}",
        account_id,
        namespace,
        key,
        sorted.len()
    );
    Ok(())
}

/// Track which (account_id, provider) pairs we've already started a scanner
/// for. The webview lifecycle can call `ensure_scanner` repeatedly without
/// double-spawning.
#[derive(Default)]
pub struct ScannerRegistry {
    started: Mutex<std::collections::HashSet<String>>,
}

/// Per-account flag: have we installed the Worker constructor hook +
/// reloaded the page yet for this account? Module-level so we don't have
/// to thread the registry through every CDP call.
#[allow(dead_code)]
fn hook_already_installed_for(account_id: &str) -> bool {
    use std::sync::OnceLock;
    static HOOKED: OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    let set = HOOKED.get_or_init(|| std::sync::Mutex::new(Default::default()));
    let mut g = set.lock().unwrap();
    !g.insert(account_id.to_string())
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
            log::debug!("[cdp] scanner already running for {}", account_id);
            return;
        }
        spawn_scanner(app, account_id, url_prefix);
    }

    pub async fn forget(&self, account_id: &str) {
        let mut g = self.started.lock().await;
        g.remove(account_id);
    }
}
