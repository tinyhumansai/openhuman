//! WhatsApp Web scanner driven over the Chrome DevTools Protocol (CDP).
//!
//! We talk to the embedded CEF instance through its remote-debugging port
//! (set via `--remote-debugging-port=19222` in `lib.rs`). Per tracked
//! WhatsApp-account webview, two interleaved loops run:
//!
//!   * **Fast tick** (`FAST_SCAN_INTERVAL`, 2s) — `dom_scan.js` scrapes
//!     rendered `[data-id]` message rows from the DOM. Emits only when
//!     the visible-set hash changes so idle windows stay silent.
//!   * **Full tick** (`FULL_SCAN_INTERVAL`, 30s) — `scanner.js` walks
//!     WhatsApp's IndexedDB stores (model-storage, signal-storage, …) to
//!     pull message metadata, chat names, contact names.
//!
//! Each scan groups messages by `(chatId, day)` and posts one
//! `openhuman.memory_doc_ingest` JSON-RPC call per group to the core, so
//! each day of a conversation upserts a single memory doc. We also emit
//! `webview:event` ingest events so any React UI listening can update
//! live when the main window is open.
//!
//! NOTE: only meaningful with the `cef` feature — the wry runtime does
//! not expose a remote debugging port. Compile-gated at the call site.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::task::AbortHandle;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

mod dom_snapshot;
mod idb;

const CDP_HOST: &str = "127.0.0.1";
// Must match `--remote-debugging-port=19222` in lib.rs and
// `cdp::CDP_PORT`. Was 9222, moved to dodge ollama's listener.
const CDP_PORT: u16 = 19222;
/// Cadence for the expensive full scan — pages the whole IDB via CDP and
/// captures a fresh DOM snapshot. Each pass serialises thousands of
/// message records, so we pay this cost infrequently.
const FULL_SCAN_INTERVAL: Duration = Duration::from_secs(30);
/// Cadence for the cheap fast scan (DOM `[data-id]` scrape only). Runs at
/// Franz-like 2s so the ingest stream feels live — each tick captures the
/// DOM via `DOMSnapshot.captureSnapshot` (pure CDP, no page-world JS).
const FAST_SCAN_INTERVAL: Duration = Duration::from_secs(2);

/// One CDP target descriptor (from `Target.getTargets`).
#[derive(Debug, Clone)]
struct CdpTarget {
    id: String,
    kind: String,
    url: String,
}

/// Product of one full scan — IDB walk (via `idb::walk`) joined with a
/// DOM snapshot (via `dom_snapshot::capture_messages`). `messages` carries
/// IDB-sourced metadata only; DOM-sourced bodies are merged in by id at
/// emit time (see `emit_snapshot`).
#[derive(Debug, Clone, Default)]
pub struct ScanSnapshot {
    pub ok: bool,
    pub error: Option<String>,
    /// `jid → display name`, drawn from chat/contact/group-metadata stores.
    pub chats: serde_json::Map<String, Value>,
    /// Normalised message metadata (no bodies — see note above).
    pub messages: Vec<Value>,
    /// DOM-scraped rendered bodies; merged into `messages` by id.
    pub dom_messages: Vec<Value>,
    /// Active chat's display name parsed from
    /// `header[data-testid="conversation-header"]`. Used by the merge step
    /// to reverse-look-up `chatId` for DOM rows that lack one (modern
    /// WhatsApp Web doesn't expose chat JID anywhere on the message rows).
    pub active_chat_name: Option<String>,
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
pub fn spawn_scanner<R: Runtime>(
    app: AppHandle<R>,
    account_id: String,
    url_prefix: String,
) -> Vec<AbortHandle> {
    let task = tokio::spawn(async move {
        let fragment = crate::cdp::target_url_fragment(&account_id);
        log::info!(
            "[wa] scanner up account={} url_prefix={} fragment={} fast={:?} full={:?}",
            account_id,
            url_prefix,
            fragment,
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
                match scan_dom_once(&account_id, &url_prefix, &fragment).await {
                    Ok(dom) => {
                        let changed =
                            last_dom_hash != Some(dom.hash) && !dom.dom_messages.is_empty();
                        if changed {
                            log::info!(
                                "[wa][{}] fast dom-scan rows={} hash={} (changed)",
                                account_id,
                                dom.dom_messages.len(),
                                dom.hash
                            );
                            emit_dom_only(&app, &account_id, &dom.dom_messages);
                            last_dom_hash = Some(dom.hash);
                        }
                    }
                    Err(e) => {
                        log::debug!("[wa][{}] dom-scan failed: {}", account_id, e);
                    }
                }
                sleep(FAST_SCAN_INTERVAL).await;
                continue;
            }
            last_full = Instant::now();
            match scan_once(&app, &account_id, &url_prefix, &fragment).await {
                Ok(snap) => {
                    log::info!(
                        "[wa][{}] full scan ok messages={} chats={} dom={}",
                        account_id,
                        snap.messages.len(),
                        snap.chats.len(),
                        snap.dom_messages.len(),
                    );
                    // Preview a few DOM-scraped rows so it's obvious from the
                    // log whether the active chat produced fresh bodies.
                    for (i, dm) in snap.dom_messages.iter().take(5).enumerate() {
                        let chat = dm.get("chatId").and_then(|v| v.as_str()).unwrap_or("?");
                        let msg = dm.get("msgId").and_then(|v| v.as_str()).unwrap_or("?");
                        let from_me = dm.get("fromMe").and_then(|v| v.as_bool()).unwrap_or(false);
                        let author = dm.get("author").and_then(|v| v.as_str()).unwrap_or("");
                        let ts = dm
                            .get("preTimestamp")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let body = dm.get("body").and_then(|v| v.as_str()).unwrap_or("");
                        let preview: String = body.chars().take(120).collect();
                        log::info!(
                            "[wa][{}] dom#{} chat={} msg={} fromMe={} [{}] {}: {:?}",
                            account_id,
                            i + 1,
                            chat,
                            msg,
                            from_me,
                            ts,
                            author,
                            preview
                        );
                    }
                    emit_snapshot(&app, &account_id, &snap);
                }
                Err(e) => {
                    log::warn!("[wa][{}] scan failed: {}", account_id, e);
                }
            }
            // After a full scan, go back to fast-tick cadence until the
            // next `FULL_SCAN_INTERVAL` elapses.
            sleep(FAST_SCAN_INTERVAL).await;
        }
    });
    vec![task.abort_handle()]
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
fn contact_cache(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, serde_json::Map<String, Value>>> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, serde_json::Map<String, Value>>>,
    > = OnceLock::new();
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
    url_fragment: &str,
) -> Result<ScanSnapshot, String> {
    // One CDP connection per tick — we attach to the WhatsApp page session,
    // run the IDB walk + DOM snapshot, then detach (which frees every
    // RemoteObject the IDB walk materialised, so no per-object releases).
    let browser_ws = browser_ws_url().await?;
    let mut cdp = CdpConn::open(&browser_ws).await?;

    let targets_v = cdp.call("Target.getTargets", json!({}), None).await?;
    let targets = parse_targets(&targets_v);
    log::debug!("[wa][{}] {} targets total", account_id, targets.len());

    let page_target = targets
        .iter()
        .find(|t| {
            t.kind == "page" && t.url.starts_with(url_prefix) && t.url.ends_with(url_fragment)
        })
        .ok_or_else(|| format!("no page target matching {url_prefix} fragment={url_fragment}"))?;
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

    // IDB + DOM are independent — run IDB first (the heavier of the two)
    // so a DOM failure doesn't mask IDB errors. Errors are captured on
    // `snap.error` instead of bubbling so the caller can still act on
    // whatever partial data came back.
    let mut snap = ScanSnapshot {
        ok: true,
        ..Default::default()
    };
    match idb::walk(&mut cdp, &page_session, url_prefix).await {
        Ok((messages, chat_names)) => {
            snap.messages = messages.iter().map(idb::IdbMessage::to_json).collect();
            snap.chats = chat_names
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect();
        }
        Err(e) => {
            snap.ok = false;
            snap.error = Some(format!("idb walk: {e}"));
            log::warn!("[wa][{}] idb walk failed: {}", account_id, e);
        }
    }
    match dom_snapshot::capture_messages(&mut cdp, &page_session).await {
        Ok((rows, _hash, active_chat_name)) => {
            snap.dom_messages = rows.iter().map(dom_snapshot::DomMessage::to_json).collect();
            snap.active_chat_name = active_chat_name;
        }
        Err(e) => {
            // Fast-tick DOM scans will retry every 2s, so degrade gracefully.
            log::warn!("[wa][{}] dom snapshot failed: {}", account_id, e);
        }
    }

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
#[derive(Debug, Default)]
pub struct DomScanResult {
    pub dom_messages: Vec<Value>,
    pub hash: u64,
}

/// Fast tick: open a CDP session, attach to the WhatsApp page, snapshot
/// the DOM via `DOMSnapshot.captureSnapshot`, detach. No IDB, no worker
/// enumeration, no JavaScript runs in the page — the snapshot is produced
/// at the browser's C++ layer. The flat-array response is parsed in Rust
/// (see `dom_snapshot.rs`).
async fn scan_dom_once(
    account_id: &str,
    url_prefix: &str,
    url_fragment: &str,
) -> Result<DomScanResult, String> {
    let browser_ws = browser_ws_url().await?;
    let mut cdp = CdpConn::open(&browser_ws).await?;
    let targets_v = cdp.call("Target.getTargets", json!({}), None).await?;
    let targets = parse_targets(&targets_v);
    let page_target = targets
        .iter()
        .find(|t| {
            t.kind == "page" && t.url.starts_with(url_prefix) && t.url.ends_with(url_fragment)
        })
        .ok_or_else(|| format!("no page target matching {url_prefix} fragment={url_fragment}"))?;
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
    let captured = dom_snapshot::capture_messages(&mut cdp, &page_session).await;
    // Detach no matter what — otherwise dangling sessions pile up on long
    // runs and eventually the CDP endpoint refuses new attachments.
    let _ = cdp
        .call(
            "Target.detachFromTarget",
            json!({ "sessionId": page_session }),
            None,
        )
        .await;
    let (rows, hash, _active_chat_name) = captured?;
    let dom_messages: Vec<Value> = rows.iter().map(dom_snapshot::DomMessage::to_json).collect();
    log::debug!(
        "[wa][{}] fast dom-scan rows={} hash={}",
        account_id,
        dom_messages.len(),
        hash
    );
    Ok(DomScanResult { dom_messages, hash })
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
                Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                    continue
                }
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
            "[wa][{}] snapshot not ok (idb walk failed: {:?}) — falling through with dom-only data",
            account_id,
            snap.error
        );
        // Fall through so DOM messages still reach the structured store.
    }
    // Resolve the active chat's JID from its display name (parsed from the
    // conversation header). Modern WhatsApp Web doesn't put the chat JID
    // anywhere on individual message rows or in the URL, so this is the
    // only signal we have. The IDB-side `chats` map has `name → jid` (we
    // store it as `jid → {name, …}`, so iterate). Match prefers exact
    // case-sensitive equality and falls back to case-insensitive; ignore
    // ambiguous matches (multiple chats with the same display name) so we
    // don't mis-attribute messages.
    let active_chat_jid: Option<String> = snap.active_chat_name.as_deref().and_then(|name| {
        let name_lc = name.to_ascii_lowercase();
        let mut exact: Vec<&str> = Vec::new();
        let mut ci: Vec<&str> = Vec::new();
        let mut substring: Vec<&str> = Vec::new();
        for (jid, chat) in snap.chats.iter() {
            let chat_name = chat.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if chat_name == name {
                exact.push(jid);
            } else if !chat_name.is_empty() && chat_name.to_ascii_lowercase() == name_lc {
                ci.push(jid);
            } else if !chat_name.is_empty()
                && (chat_name.to_ascii_lowercase().contains(&name_lc)
                    || name_lc.contains(&chat_name.to_ascii_lowercase()))
            {
                substring.push(jid);
            }
        }
        // Prefer exact > case-insensitive > substring. Substring only wins
        // when there's exactly one candidate (avoids cross-attribution when
        // many chats share a token like a common first name).
        match (exact.len(), ci.len(), substring.len()) {
            (1, _, _) => Some(exact[0].to_string()),
            (0, 1, _) => Some(ci[0].to_string()),
            (0, 0, 1) => Some(substring[0].to_string()),
            (e, c, s) => {
                let count = e + c + s;
                if count > 1 {
                    log::warn!(
                        "[whatsapp_scanner] ambiguous active-chat resolution: {} candidates for '{}' — skipping backfill",
                        count,
                        name
                    );
                }
                None
            }
        }
    });
    log::info!(
        "[wa][{}] active chat resolution: name={:?} → jid={:?} chats_in_map={}",
        account_id,
        snap.active_chat_name,
        active_chat_jid,
        snap.chats.len()
    );
    // Join DOM-scraped bodies into the messages list by msgId. WhatsApp
    // caches decrypted bodies in memory, so IndexedDB gives us metadata and
    // the DOM gives us text for currently-rendered chats — unioning them
    // here gives downstream consumers a single message list.
    // The merge logic lives in `merge_dom_into_snapshot` so it can be
    // exercised independently in unit tests.
    let (messages, patched, appended) = merge_dom_into_snapshot(
        &snap.messages,
        &snap.dom_messages,
        active_chat_jid.as_deref(),
    );
    if patched > 0 || appended > 0 {
        log::info!(
            "[wa][{}] dom-merge patched={} appended={} total={}",
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
        let (day, ts_secs): (String, i64) =
            if let Some(t) = m.get("timestamp").and_then(|v| v.as_i64()) {
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
        let type_ = m.get("type").cloned().unwrap_or(Value::Null);
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
            log::warn!("[wa][{}] ingest emit failed: {}", account_id, e);
        } else {
            emitted += 1;
        }
        // Direct memory write via core RPC — fire-and-forget so the
        // scanner tick doesn't block on HTTP.
        let acct = account_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = post_memory_doc_ingest(&acct, &payload).await {
                log::warn!("[wa][{}] memory write failed: {}", acct, e);
            }
        });
    }
    if emitted > 0 {
        log::info!(
            "[wa][{}] emitted {} ingest group(s) source={}",
            account_id,
            emitted,
            source
        );
    }

    // Dual-write: also persist structured chat+message data via the
    // dedicated whatsapp_data store. Fire-and-forget alongside the existing
    // memory doc ingest path — does not affect scanner tick timing.
    {
        let acct = account_id.to_string();
        let chats_value = Value::Object(chats.clone());
        // Build normalized message array for the structured ingest.
        // Handles both full IDB-scan shape (chatId, timestamp, from/fromName,
        // type) and fast DOM-only rows (author, preTimestamp, dataId).
        let msgs_for_ingest: Vec<Value> = messages
            .iter()
            .filter_map(|m| {
                // Accept chatId from full-scan or chat/chat_id fallbacks on DOM rows.
                let chat_id = m
                    .get("chatId")
                    .or_else(|| m.get("chat"))
                    .or_else(|| m.get("chat_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())?
                    .to_string();
                let body = m
                    .get("body")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .unwrap_or("");
                // Include non-text messages (stickers/images) so message_count
                // and last_message_ts stay accurate. Empty body is allowed.
                let msg_id = m
                    .get("id")
                    .cloned()
                    .or_else(|| m.get("dataId").cloned())
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                if msg_id.is_empty() {
                    return None;
                }
                // Resolve sender: full-scan uses fromName/from; DOM rows use author.
                let sender = m
                    .get("fromName")
                    .cloned()
                    .or_else(|| m.get("from").cloned())
                    .or_else(|| m.get("author").cloned());
                let sender_jid = m
                    .get("from")
                    .cloned()
                    .or_else(|| m.get("author").cloned())
                    .or_else(|| m.get("participant").cloned());
                // Resolve timestamp: full-scan has numeric timestamp;
                // DOM rows may carry a string preTimestamp that needs parsing.
                let timestamp = m
                    .get("timestamp")
                    .and_then(|v| v.as_i64())
                    .or_else(|| m.get("preTimestamp").and_then(|v| v.as_i64()))
                    .unwrap_or(0);
                Some(json!({
                    "message_id": msg_id,
                    "chat_id": chat_id,
                    "sender": sender.unwrap_or(Value::Null),
                    "sender_jid": sender_jid.unwrap_or(Value::Null),
                    "from_me": m.get("fromMe").and_then(|v| v.as_bool()).unwrap_or(false),
                    "body": body,
                    "timestamp": timestamp,
                    "message_type": m.get("type").cloned().unwrap_or(Value::Null),
                    "source": source,
                }))
            })
            .collect();
        let src = source.to_string();
        tokio::spawn(async move {
            if let Err(e) =
                post_whatsapp_data_ingest(&acct, &chats_value, &msgs_for_ingest, &src).await
            {
                log::warn!(
                    "[wa][{}] whatsapp_data structured ingest failed: {}",
                    acct,
                    e
                );
            }
        });
    }
}

/// Build the JSON-RPC `params` object for `openhuman.memory_doc_ingest`
/// from a single (chatId, day) ingest payload. Extracted as a pure
/// function so it can be tested independently of the HTTP layer.
///
/// Returns `None` when the payload is missing required fields (chatId, day,
/// or a non-empty messages array) — callers should skip the HTTP call.
fn build_doc_ingest_params(account_id: &str, ingest: &Value) -> Option<Value> {
    let chat_id = ingest
        .get("chatId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let day = ingest
        .get("day")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
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
        return None;
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

    Some(json!({
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
    }))
}

/// Build the `openhuman.memory_doc_ingest` payload for a single
/// (chatId, day) group and POST it directly to the core. The shape
/// mirrors `persistWhatsappChatDay` on the React side so the memory docs
/// line up whether the scanner or the UI drove the ingest.
///
/// Retries once (after 500ms) on connection errors so the scanner isn't
/// silently dropped when the core sidecar isn't ready yet at startup.
async fn post_memory_doc_ingest(account_id: &str, ingest: &Value) -> Result<(), String> {
    let params = match build_doc_ingest_params(account_id, ingest) {
        Some(p) => p,
        None => return Ok(()),
    };

    // Extract namespace/key for the success log from the built params.
    let namespace = params
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let key = params
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let msg_count = params
        .get("metadata")
        .and_then(|m| m.get("message_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "openhuman.memory_doc_ingest",
        "params": params,
    });

    let url = crate::core_rpc::core_rpc_url_value();

    // Retry up to 2 attempts with 500ms delay on connection errors (e.g.
    // core sidecar not yet ready at scanner startup). HTTP-level errors
    // (non-2xx responses, JSON-RPC errors) are not retried — they indicate
    // a real problem rather than a startup race.
    let mut last_err = String::new();
    for attempt in 1u8..=2 {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        let req = crate::core_rpc::apply_auth(client.post(&url))
            .map_err(|e| format!("prepare {url}: {e}"))?;
        let send_result = req.json(&body).send().await;
        match send_result {
            Err(e) if e.is_connect() || e.is_timeout() => {
                last_err = format!("POST {url}: {e}");
                if attempt < 2 {
                    log::debug!(
                        "[wa][{}] memory ingest connect error (attempt {}), retrying in 500ms: {}",
                        account_id,
                        attempt,
                        e
                    );
                    sleep(Duration::from_millis(500)).await;
                    continue;
                }
                return Err(last_err);
            }
            Err(e) => return Err(format!("POST {url}: {e}")),
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    let body_text = resp.text().await.unwrap_or_default();
                    return Err(format!("{status}: {body_text}"));
                }
                let v: Value = resp.json().await.map_err(|e| format!("decode: {e}"))?;
                if let Some(err) = v.get("error") {
                    return Err(format!("rpc error: {err}"));
                }
                log::info!(
                    "[wa][{}] memory upsert ok namespace={} key={} msgs={}",
                    account_id,
                    namespace,
                    key,
                    msg_count
                );
                return Ok(());
            }
        }
    }
    Err(last_err)
}

/// POST a structured `openhuman.whatsapp_data_ingest` payload to the core.
///
/// This is the dual-write path alongside `post_memory_doc_ingest`. It
/// persists chats and messages into the dedicated `whatsapp_data.db` SQLite
/// store so the agent can query them via structured RPC tools.
async fn post_whatsapp_data_ingest(
    account_id: &str,
    chats: &Value,
    messages: &[Value],
    source: &str,
) -> Result<(), String> {
    if messages.is_empty() && chats.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        return Ok(());
    }

    // Convert chats map values to {name: string|null} once, before batching.
    // The scanner passes chats as either:
    //   - Value::String(display_name) — contact-cache format
    //   - Value::Object({name: ..., ...}) — full IDB scan format
    let chats_param: serde_json::Map<String, Value> = chats
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(jid, v)| {
                    let name = if let Some(s) = v.as_str() {
                        if s.is_empty() {
                            Value::Null
                        } else {
                            Value::String(s.to_string())
                        }
                    } else {
                        v.get("name")
                            .and_then(|n| n.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| Value::String(s.to_string()))
                            .unwrap_or(Value::Null)
                    };
                    (jid.clone(), json!({ "name": name }))
                })
                .collect()
        })
        .unwrap_or_default();

    // Split messages into chunks to stay well under the HTTP body size limit.
    // Chats are sent only with the first batch (upserts are idempotent).
    const BATCH_SIZE: usize = 500;
    let empty_chats = Value::Object(serde_json::Map::new());
    let url = crate::core_rpc::core_rpc_url_value();

    // Build at least one batch even when messages is empty (chats-only upsert).
    let chunks: Vec<&[Value]> = if messages.is_empty() {
        vec![&[]]
    } else {
        messages.chunks(BATCH_SIZE).collect()
    };

    let total_batches = chunks.len();
    log::debug!(
        "[wa][{}] whatsapp_data_ingest chats={} messages={} batches={} source={}",
        account_id,
        chats_param.len(),
        messages.len(),
        total_batches,
        source
    );

    for (batch_idx, chunk) in chunks.iter().enumerate() {
        let batch_chats = if batch_idx == 0 {
            Value::Object(chats_param.clone())
        } else {
            empty_chats.clone()
        };
        let params = json!({
            "account_id": account_id,
            "chats": batch_chats,
            "messages": chunk,
        });
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "openhuman.whatsapp_data_ingest",
            "params": params,
        });

        let mut last_err = String::new();
        let mut succeeded = false;
        for attempt in 1u8..=2 {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .map_err(|e| format!("http client: {e}"))?;
            let req = crate::core_rpc::apply_auth(client.post(&url))
                .map_err(|e| format!("prepare {url}: {e}"))?;
            let send_result = req.json(&body).send().await;
            match send_result {
                Err(e) if e.is_connect() || e.is_timeout() => {
                    last_err = format!("POST {url}: {e}");
                    if attempt < 2 {
                        log::debug!(
                            "[wa][{}] whatsapp_data_ingest connect error batch={}/{} attempt {}: {}",
                            account_id,
                            batch_idx + 1,
                            total_batches,
                            attempt,
                            last_err
                        );
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    continue;
                }
                Err(e) => return Err(format!("POST {url}: {e}")),
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        let body_text = resp.text().await.unwrap_or_default();
                        return Err(format!("{status}: {body_text}"));
                    }
                    let v: Value = resp.json().await.map_err(|e| format!("decode: {e}"))?;
                    if let Some(err) = v.get("error") {
                        return Err(format!("rpc error: {err}"));
                    }
                    succeeded = true;
                    break;
                }
            }
        }
        if !succeeded {
            return Err(last_err);
        }
    }

    log::debug!(
        "[wa][{}] whatsapp_data_ingest ok messages={} batches={}",
        account_id,
        messages.len(),
        total_batches,
    );
    Ok(())
}

/// Merge DOM-scraped rows into an IDB-sourced message list.
///
/// Extracted from `emit_snapshot` so the merge logic can be tested
/// independently of the Tauri `AppHandle`. Behaviour:
///
/// 1. Build an index of DOM rows keyed by both their full `dataId` and bare
///    `msgId` (the current WA Web format emits only the bare hex id).
/// 2. Patch IDB messages that have an empty `body` with the DOM row's body;
///    mark the DOM row as consumed.
/// 3. Append unmatched DOM rows that have a non-empty body, stamping
///    `chatId` from `active_chat_jid` when the row lacks one.
///
/// Returns the merged message list along with patch/append counts for
/// diagnostic logging.
fn merge_dom_into_snapshot(
    idb_messages: &[Value],
    dom_messages: &[Value],
    active_chat_jid: Option<&str>,
) -> (Vec<Value>, usize, usize) {
    use std::collections::{HashMap, HashSet};

    let mut messages = idb_messages.to_vec();

    if dom_messages.is_empty() {
        return (messages, 0, 0);
    }

    // Index DOM rows by full dataId and bare msgId.
    let mut by_msg_id: HashMap<String, (String, Value)> = HashMap::new();
    for dm in dom_messages {
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
        let mid_opt = m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
        let has_body = m
            .get("body")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if has_body {
            continue;
        }
        if let Some(mid) = mid_opt {
            let bare_mid = mid.rsplitn(2, '_').next().map(str::to_string);
            let lookup = by_msg_id
                .get(&mid)
                .cloned()
                .or_else(|| bare_mid.as_deref().and_then(|b| by_msg_id.get(b).cloned()));
            if let Some((did, dm)) = lookup {
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

    // Append unmatched DOM rows that have a body.
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
            let dom_chat_id = dm
                .get("chatId")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| Value::String(s.to_string()))
                .or_else(|| active_chat_jid.map(|j| Value::String(j.to_string())))
                .unwrap_or(Value::Null);
            messages.push(json!({
                "id": dm.get("dataId").cloned().unwrap_or(Value::Null),
                "chatId": dom_chat_id,
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

    (messages, patched, appended)
}

/// Track which (account_id, provider) pairs we've already started a scanner
/// for. The webview lifecycle can call `ensure_scanner` repeatedly without
/// double-spawning.
#[derive(Default)]
pub struct ScannerRegistry {
    started: Mutex<std::collections::HashMap<String, Vec<AbortHandle>>>,
}

impl ScannerRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn ensure_scanner<R: Runtime>(
        &self,
        app: AppHandle<R>,
        account_id: String,
        url_prefix: String,
    ) {
        let mut g = self.started.lock();
        if g.contains_key(&account_id) {
            log::debug!("[wa] scanner already running for {}", account_id);
            return;
        }
        let handles = spawn_scanner(app, account_id.clone(), url_prefix);
        g.insert(account_id, handles);
    }

    pub fn forget(&self, account_id: &str) {
        let handles = self.started.lock().remove(account_id);
        if let Some(handles) = handles {
            let count = handles.len();
            for handle in handles {
                handle.abort();
            }
            log::info!("[wa] aborted {} scanner task(s) for {}", count, account_id);
        }
    }

    pub fn forget_all(&self) -> usize {
        let entries: Vec<_> = self.started.lock().drain().collect();
        let task_count = entries.iter().map(|(_, handles)| handles.len()).sum();
        for (account_id, handles) in entries {
            for handle in handles {
                handle.abort();
            }
            log::debug!("[wa] aborted scanner tasks for {}", account_id);
        }
        if task_count > 0 {
            log::info!("[wa] aborted {} scanner task(s)", task_count);
        }
        task_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_pending_tasks(
        registry: &ScannerRegistry,
        account_id: &str,
        count: usize,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        let mut tasks = Vec::with_capacity(count);
        let mut abort_handles = Vec::with_capacity(count);
        for _ in 0..count {
            let task = tokio::spawn(async {
                std::future::pending::<()>().await;
            });
            abort_handles.push(task.abort_handle());
            tasks.push(task);
        }
        registry
            .started
            .lock()
            .insert(account_id.to_string(), abort_handles);
        tasks
    }

    async fn assert_cancelled(task: tokio::task::JoinHandle<()>) {
        let err = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("aborted scanner task should finish")
            .expect_err("scanner task should be cancelled");
        assert!(err.is_cancelled());
    }

    async fn assert_all_cancelled(tasks: Vec<tokio::task::JoinHandle<()>>) {
        for task in tasks {
            assert_cancelled(task).await;
        }
    }

    #[tokio::test]
    async fn registry_forget_aborts_all_handles_for_account_only() {
        let registry = ScannerRegistry::default();
        let account_tasks = insert_pending_tasks(&registry, "acct-1", 2);
        let survivor_tasks = insert_pending_tasks(&registry, "acct-2", 1);

        registry.forget("acct-1");

        {
            let guard = registry.started.lock();
            assert_eq!(guard.len(), 1);
            assert!(guard.contains_key("acct-2"));
        }
        assert_all_cancelled(account_tasks).await;
        assert!(
            !survivor_tasks[0].is_finished(),
            "forget(acct-1) must not abort acct-2"
        );

        assert_eq!(registry.forget_all(), 1);
        assert_all_cancelled(survivor_tasks).await;
    }

    #[tokio::test]
    async fn registry_forget_missing_account_is_noop() {
        let registry = ScannerRegistry::default();
        let mut tasks = insert_pending_tasks(&registry, "acct-1", 1);

        registry.forget("missing");

        {
            let guard = registry.started.lock();
            assert_eq!(guard.len(), 1);
            assert!(guard.contains_key("acct-1"));
        }
        assert!(
            !tasks[0].is_finished(),
            "forget(missing) must not abort existing scanners"
        );

        registry.forget("acct-1");
        assert_cancelled(tasks.pop().expect("task")).await;
    }

    #[tokio::test]
    async fn registry_forget_all_aborts_all_tasks_and_reports_handle_count() {
        let registry = ScannerRegistry::default();
        let task_a = insert_pending_tasks(&registry, "acct-1", 2);
        let task_b = insert_pending_tasks(&registry, "acct-2", 3);

        assert_eq!(registry.forget_all(), 5);

        assert!(registry.started.lock().is_empty());
        assert_all_cancelled(task_a).await;
        assert_all_cancelled(task_b).await;
    }

    #[tokio::test]
    async fn registry_forget_all_is_repeatable_noop_after_drain() {
        let registry = ScannerRegistry::default();
        assert_eq!(registry.forget_all(), 0);

        let tasks = insert_pending_tasks(&registry, "acct-1", 1);
        assert_eq!(registry.forget_all(), 1);
        assert_eq!(registry.forget_all(), 0);

        assert!(registry.started.lock().is_empty());
        assert_all_cancelled(tasks).await;
    }

    // ── seconds_to_ymd ────────────────────────────────────────────────────────

    #[test]
    fn seconds_to_ymd_known_timestamp() {
        // Unix timestamp 1_700_000_000 = 2023-11-14 (UTC).
        assert_eq!(seconds_to_ymd(1_700_000_000), "2023-11-14");
    }

    #[test]
    fn seconds_to_ymd_epoch_zero() {
        // Unix epoch origin = 1970-01-01.
        assert_eq!(seconds_to_ymd(0), "1970-01-01");
    }

    #[test]
    fn seconds_to_ymd_output_format_is_yyyy_mm_dd() {
        let s = seconds_to_ymd(1_700_000_000);
        // Must match YYYY-MM-DD: 10 chars, digit/digit/digit/digit-...-...
        assert_eq!(s.len(), 10, "expected 10-char date string, got: {s}");
        let parts: Vec<&str> = s.split('-').collect();
        assert_eq!(parts.len(), 3, "expected 3 dash-separated parts: {s}");
        assert_eq!(parts[0].len(), 4, "year must be 4 digits: {s}");
        assert_eq!(parts[1].len(), 2, "month must be 2 digits: {s}");
        assert_eq!(parts[2].len(), 2, "day must be 2 digits: {s}");
        assert!(
            parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())),
            "all parts must be numeric: {s}"
        );
    }

    // ── parse_pre_timestamp_ymd ───────────────────────────────────────────────

    #[test]
    fn parse_pre_timestamp_ymd_valid_wa_format() {
        // WhatsApp Web format: "4:53 AM, 7/5/2025"
        let result = parse_pre_timestamp_ymd("4:53 AM, 7/5/2025");
        assert_eq!(result.as_deref(), Some("2025-07-05"));
    }

    #[test]
    fn parse_pre_timestamp_ymd_another_valid_date() {
        // "10:01 PM, 11/14/2023" — matches our known ts
        let result = parse_pre_timestamp_ymd("10:01 PM, 11/14/2023");
        assert_eq!(result.as_deref(), Some("2023-11-14"));
    }

    #[test]
    fn parse_pre_timestamp_ymd_empty_string_returns_none() {
        assert!(parse_pre_timestamp_ymd("").is_none());
    }

    #[test]
    fn parse_pre_timestamp_ymd_no_comma_returns_none() {
        assert!(parse_pre_timestamp_ymd("4:53 AM 7/5/2025").is_none());
    }

    #[test]
    fn parse_pre_timestamp_ymd_invalid_date_parts_return_none() {
        // Month 13 is out of range.
        assert!(parse_pre_timestamp_ymd("10:00 AM, 13/5/2025").is_none());
        // Day 32 is out of range.
        assert!(parse_pre_timestamp_ymd("10:00 AM, 1/32/2025").is_none());
    }

    #[test]
    fn parse_pre_timestamp_ymd_garbage_returns_none() {
        assert!(parse_pre_timestamp_ymd("not a timestamp at all").is_none());
    }

    // ── emit_grouped_whatsapp grouping ────────────────────────────────────────

    /// Build a minimal message Value that `emit_grouped_whatsapp` will accept.
    fn make_msg(chat_id: &str, ts: i64, body: &str, from_me: bool) -> Value {
        json!({
            "chatId": chat_id,
            "body": body,
            "timestamp": ts,
            "fromMe": from_me,
            "from": if from_me { "me" } else { chat_id },
        })
    }

    #[test]
    fn grouping_produces_correct_group_count_and_keys() {
        use std::collections::HashMap;

        // 3 messages in alice@c.us on day 2023-11-14 (ts ≈ 1_700_000_000).
        // 2 messages in group@g.us on a different day (ts ≈ 1_700_100_000 =
        // 2023-11-15 UTC).
        let day1_ts = 1_700_000_000i64; // 2023-11-14
        let day2_ts = 1_700_100_000i64; // 2023-11-15

        let messages = vec![
            make_msg("alice@c.us", day1_ts, "Hello", false),
            make_msg("alice@c.us", day1_ts + 60, "How are you?", false),
            make_msg("alice@c.us", day1_ts + 120, "Fine thanks", true),
            make_msg("group@g.us", day2_ts, "Meeting at 3pm", false),
            make_msg("group@g.us", day2_ts + 30, "Got it", true),
        ];

        // Collect groups the same way emit_grouped_whatsapp does it.
        let empty_chats = serde_json::Map::new();
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let mut groups: HashMap<(String, String), Vec<Value>> = HashMap::new();
        for m in &messages {
            let chat_id = match m.get("chatId").and_then(|v| v.as_str()) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            };
            let body = m
                .get("body")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            if body.is_empty() {
                continue;
            }
            let day: String = if let Some(t) = m.get("timestamp").and_then(|v| v.as_i64()) {
                seconds_to_ymd(t)
            } else {
                seconds_to_ymd(now_secs)
            };
            let _ = &empty_chats;
            groups.entry((chat_id, day)).or_default().push(m.clone());
        }

        assert_eq!(groups.len(), 2, "expected exactly 2 (chatId, day) groups");

        let alice_day = seconds_to_ymd(day1_ts);
        let group_day = seconds_to_ymd(day2_ts);

        let alice_key = ("alice@c.us".to_string(), alice_day.clone());
        let group_key = ("group@g.us".to_string(), group_day.clone());

        assert!(
            groups.contains_key(&alice_key),
            "alice group missing; groups: {groups:?}"
        );
        assert!(
            groups.contains_key(&group_key),
            "group@g.us group missing; groups: {groups:?}"
        );

        assert_eq!(
            groups[&alice_key].len(),
            3,
            "alice chat should have 3 messages"
        );
        assert_eq!(
            groups[&group_key].len(),
            2,
            "group chat should have 2 messages"
        );
    }

    // ── transcript format ─────────────────────────────────────────────────────

    #[test]
    fn build_doc_ingest_params_transcript_contains_senders_and_bodies() {
        let day_ts = 1_700_000_000i64; // 2023-11-14
        let ingest = json!({
            "chatId": "alice@c.us",
            "chatName": "Alice",
            "day": seconds_to_ymd(day_ts),
            "messages": [
                {
                    "chatId": "alice@c.us",
                    "fromMe": false,
                    "from": "alice@c.us",
                    "fromName": "Alice",
                    "body": "Hey there!",
                    "timestamp": day_ts,
                },
                {
                    "chatId": "alice@c.us",
                    "fromMe": true,
                    "from": "me",
                    "fromName": null,
                    "body": "Hi Alice!",
                    "timestamp": day_ts + 60,
                },
                {
                    "chatId": "alice@c.us",
                    "fromMe": false,
                    "from": "alice@c.us",
                    "fromName": "Alice",
                    "body": "How are you?",
                    "timestamp": day_ts + 120,
                },
            ],
        });

        let params = build_doc_ingest_params("test-acct@c.us", &ingest)
            .expect("should build params for valid ingest");

        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .expect("content must be present");

        // Senders should appear in the transcript.
        assert!(
            content.contains("Alice"),
            "transcript must contain sender name 'Alice'; content:\n{content}"
        );
        assert!(
            content.contains("me"),
            "transcript must contain 'me' for self-sent messages; content:\n{content}"
        );

        // Bodies must be present.
        assert!(
            content.contains("Hey there!"),
            "transcript must contain first message body; content:\n{content}"
        );
        assert!(
            content.contains("Hi Alice!"),
            "transcript must contain second message body; content:\n{content}"
        );
        assert!(
            content.contains("How are you?"),
            "transcript must contain third message body; content:\n{content}"
        );

        // Lines must appear in ascending timestamp order — verify by position.
        let pos_hey = content.find("Hey there!").expect("Hey there not found");
        let pos_hi = content.find("Hi Alice!").expect("Hi Alice not found");
        let pos_how = content.find("How are you?").expect("How are you not found");
        assert!(
            pos_hey < pos_hi && pos_hi < pos_how,
            "transcript lines must be in timestamp order"
        );
    }

    // ── build_doc_ingest_params payload shape ─────────────────────────────────

    #[test]
    fn build_doc_ingest_params_namespace_and_key_format() {
        let day = "2023-11-14";
        let ingest = json!({
            "chatId": "alice@c.us",
            "chatName": "Alice",
            "day": day,
            "messages": [
                { "chatId": "alice@c.us", "fromMe": false, "from": "alice@c.us",
                  "fromName": "Alice", "body": "Hello", "timestamp": 1_700_000_000i64 }
            ],
        });

        let params =
            build_doc_ingest_params("test-acct@c.us", &ingest).expect("should build params");

        assert_eq!(
            params.get("namespace").and_then(|v| v.as_str()),
            Some("whatsapp-web:test-acct@c.us"),
            "namespace must be 'whatsapp-web:<account_id>'"
        );
        assert_eq!(
            params.get("key").and_then(|v| v.as_str()),
            Some("alice@c.us:2023-11-14"),
            "key must be '<chat_id>:<day>'"
        );
        assert_eq!(
            params.get("source_type").and_then(|v| v.as_str()),
            Some("whatsapp-web"),
            "source_type must be 'whatsapp-web'"
        );

        // Content must be non-empty and contain the body.
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .expect("content must be present");
        assert!(!content.is_empty(), "content must not be empty");
        assert!(
            content.contains("Hello"),
            "content must contain message body; got:\n{content}"
        );
    }

    #[test]
    fn build_doc_ingest_params_missing_chat_id_returns_none() {
        let ingest = json!({
            "chatName": "Alice",
            "day": "2023-11-14",
            "messages": [
                { "chatId": "alice@c.us", "fromMe": false, "body": "Hello", "timestamp": 1i64 }
            ],
        });
        assert!(
            build_doc_ingest_params("acct", &ingest).is_none(),
            "missing chatId must return None"
        );
    }

    #[test]
    fn build_doc_ingest_params_empty_messages_returns_none() {
        let ingest = json!({
            "chatId": "alice@c.us",
            "chatName": "Alice",
            "day": "2023-11-14",
            "messages": [],
        });
        assert!(
            build_doc_ingest_params("acct", &ingest).is_none(),
            "empty messages must return None"
        );
    }

    // ── DOM-IDB merge ─────────────────────────────────────────────────────────

    #[test]
    fn merge_dom_patches_empty_body_from_idb_message() {
        // IDB message with empty body; matching DOM row has the decrypted body.
        let idb = vec![json!({
            "id": "abc123",
            "chatId": "alice@c.us",
            "fromMe": false,
            "body": "",
        })];
        let dom = vec![json!({
            "dataId": "abc123",
            "msgId": "abc123",
            "chatId": "alice@c.us",
            "fromMe": false,
            "body": "Hello",
            "author": "Alice",
            "preTimestamp": null,
        })];

        let (merged, patched, appended) = merge_dom_into_snapshot(&idb, &dom, None);

        assert_eq!(patched, 1, "one message should be patched");
        assert_eq!(appended, 0, "no messages should be appended");
        assert_eq!(merged.len(), 1, "still one message in merged list");

        let body = merged[0]
            .get("body")
            .and_then(|v| v.as_str())
            .expect("body must be present");
        assert_eq!(body, "Hello", "patched body must equal DOM body");

        let source = merged[0]
            .get("bodySource")
            .and_then(|v| v.as_str())
            .expect("bodySource must be present");
        assert_eq!(source, "dom", "bodySource must be 'dom' after patching");
    }

    #[test]
    fn merge_dom_appends_unmatched_row_with_active_chat_backfill() {
        // No IDB messages; DOM has a row with no chatId.  active_chat_jid
        // should be stamped onto the appended message.
        let idb: Vec<Value> = vec![];
        let dom = vec![json!({
            "dataId": "newrow1",
            "msgId": "newrow1",
            "chatId": "",   // empty — needs backfill
            "fromMe": false,
            "body": "Hey from active chat",
            "author": "Bob",
            "preTimestamp": null,
        })];

        let (merged, patched, appended) = merge_dom_into_snapshot(&idb, &dom, Some("bob@c.us"));

        assert_eq!(patched, 0, "nothing to patch");
        assert_eq!(appended, 1, "one row should be appended");
        assert_eq!(merged.len(), 1, "merged list should have 1 entry");

        let chat_id = merged[0]
            .get("chatId")
            .and_then(|v| v.as_str())
            .expect("chatId must be present");
        assert_eq!(
            chat_id, "bob@c.us",
            "chatId should be backfilled from active_chat_jid"
        );

        let body_source = merged[0]
            .get("bodySource")
            .and_then(|v| v.as_str())
            .expect("bodySource must be present");
        assert_eq!(body_source, "dom-only");
    }

    #[test]
    fn merge_dom_does_not_append_row_without_body() {
        // DOM rows without a body should be silently skipped.
        let idb: Vec<Value> = vec![];
        let dom = vec![json!({
            "dataId": "empty1",
            "msgId": "empty1",
            "chatId": "alice@c.us",
            "fromMe": false,
            "body": "",
        })];

        let (merged, patched, appended) = merge_dom_into_snapshot(&idb, &dom, None);

        assert_eq!(patched, 0);
        assert_eq!(appended, 0, "empty-body DOM rows must not be appended");
        assert!(
            merged.is_empty(),
            "no messages should appear in merged list"
        );
    }

    #[test]
    fn merge_dom_does_not_consume_row_twice() {
        // Two IDB messages with the same bare msgId; only the first match
        // should consume the DOM row.
        let idb = vec![
            json!({ "id": "chat_abc", "chatId": "alice@c.us", "fromMe": false, "body": "" }),
            json!({ "id": "chat_abc_2", "chatId": "alice@c.us", "fromMe": true, "body": "" }),
        ];
        // DOM row keyed only by bare msgId "abc".
        let dom = vec![json!({
            "dataId": "abc",
            "msgId": "abc",
            "chatId": "alice@c.us",
            "fromMe": false,
            "body": "Only once",
        })];

        let (merged, patched, _appended) = merge_dom_into_snapshot(&idb, &dom, None);

        // Exactly one of the two IDB messages should be patched.
        assert_eq!(patched, 1, "DOM row must be consumed at most once");
        assert_eq!(merged.len(), 2, "both IDB messages must survive merge");
        let patched_bodies: Vec<&str> = merged
            .iter()
            .filter_map(|m| m.get("body").and_then(|v| v.as_str()))
            .filter(|b| *b == "Only once")
            .collect();
        assert_eq!(
            patched_bodies.len(),
            1,
            "body 'Only once' must appear exactly once in merged list"
        );
    }

    #[test]
    fn merge_dom_empty_dom_returns_idb_messages_unchanged() {
        let idb = vec![
            json!({ "id": "m1", "chatId": "a@c.us", "body": "hello" }),
            json!({ "id": "m2", "chatId": "a@c.us", "body": "" }),
        ];
        let dom: Vec<Value> = vec![];

        let (merged, patched, appended) = merge_dom_into_snapshot(&idb, &dom, None);

        assert_eq!(patched, 0);
        assert_eq!(appended, 0);
        assert_eq!(merged.len(), 2, "IDB messages must be returned unchanged");
        assert_eq!(
            merged[0].get("body").and_then(|v| v.as_str()),
            Some("hello")
        );
    }
}
