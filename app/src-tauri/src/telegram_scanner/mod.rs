//! Telegram Web K scanner driven purely over the Chrome DevTools Protocol.
//!
//! Pairs with the embedded CEF webview's remote-debugging port (set in
//! `lib.rs`). One polling loop per tracked Telegram account:
//!
//!   * **IDB tick** (`IDB_SCAN_INTERVAL`, 30s) — walks every Telegram-owned
//!     IndexedDB database via CDP (`IndexedDB.requestDatabaseNames`,
//!     `IndexedDB.requestDatabase`, `IndexedDB.requestData`), materialises
//!     `Runtime.RemoteObject` records into JSON with a fixed, Telegram-
//!     agnostic serializer (`function(){return [this].concat(arguments);}`),
//!     and recursively extracts message / user / chat records from the
//!     `tweb` snapshot. No in-page JavaScript runs beyond that one fixed
//!     serializer, and no DOM scraping.
//!
//! Emits `webview:event` ingest events (for any listening React UI) AND
//! POSTs `openhuman.memory_doc_ingest` directly to the core so memory is
//! populated whether or not the main window is open. Messages are grouped
//! by peer so each peer's transcript upserts a single doc.
//!
//! Only built with the `cef` feature — wry has no remote-debugging port.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

mod dom_snapshot;
mod extract;
mod idb;

const CDP_HOST: &str = "127.0.0.1";
const CDP_PORT: u16 = 9222;
/// How often we walk IDB. Tune down for faster iteration during dev; the
/// walk itself is bounded by per-store record caps in `idb.rs`.
const IDB_SCAN_INTERVAL: Duration = Duration::from_secs(30);

/// One CDP target descriptor (from `Target.getTargets`).
#[derive(Debug, Clone)]
struct CdpTarget {
    id: String,
    kind: String,
    url: String,
}

/// Spawn a per-account CDP poller. Caller is expected to guard against
/// double-spawning via `ScannerRegistry`.
pub fn spawn_scanner<R: Runtime>(app: AppHandle<R>, account_id: String, url_prefix: String) {
    // Independent fast-tick task for the DOM chat-list scrape (replaces
    // the old recipe.js setInterval). Decoupled from the slow IDB loop so
    // an IDB failure doesn't stall the UI's unread-badge updates.
    spawn_dom_poll(app.clone(), account_id.clone(), url_prefix.clone());
    tokio::spawn(async move {
        let fragment = crate::cdp::target_url_fragment(&account_id);
        log::info!(
            "[tg] scanner up account={} url_prefix={} fragment={} interval={:?}",
            account_id,
            url_prefix,
            fragment,
            IDB_SCAN_INTERVAL,
        );
        // Let tweb hydrate IDB before the first scan — otherwise we'd
        // race empty stores on cold start.
        sleep(Duration::from_secs(10)).await;

        loop {
            match scan_once(&account_id, &url_prefix, &fragment).await {
                Ok(dump) => {
                    let harvest = extract::harvest(&dump);
                    log::info!(
                        "[tg][{}] idb extract: {} msgs, {} users, {} chats, self={}",
                        account_id,
                        harvest.messages.len(),
                        harvest.users.len(),
                        harvest.chats.len(),
                        harvest.self_id.as_deref().unwrap_or("?"),
                    );
                    if !harvest.messages.is_empty() {
                        emit_and_persist(&app, &account_id, &harvest);
                    }
                }
                Err(e) => {
                    log::warn!("[tg][{}] idb scan failed: {}", account_id, e);
                }
            }
            sleep(IDB_SCAN_INTERVAL).await;
        }
    });
}

/// Single scan cycle: open CDP, attach to the Telegram page, walk IDB, detach.
async fn scan_once(
    account_id: &str,
    url_prefix: &str,
    url_fragment: &str,
) -> Result<idb::IdbDump, String> {
    let browser_ws = browser_ws_url().await?;
    let mut cdp = CdpConn::open(&browser_ws).await?;

    let targets_v = cdp.call("Target.getTargets", json!({}), None).await?;
    let targets = parse_targets(&targets_v);
    let page_target = targets
        .iter()
        .find(|t| t.kind == "page" && t.url.starts_with(url_prefix) && t.url.contains(url_fragment))
        .ok_or_else(|| {
            format!(
                "no page target matching {} fragment={}",
                url_prefix, url_fragment
            )
        })?;

    let attach = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": page_target.id, "flatten": true }),
            None,
        )
        .await?;
    let session = attach
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "page attach missing sessionId".to_string())?
        .to_string();

    let result = idb::walk(&mut cdp, &session).await;

    let _ = cdp
        .call(
            "Target.detachFromTarget",
            json!({ "sessionId": session }),
            None,
        )
        .await;

    let dump = result?;
    log::info!(
        "[tg][{}] scan ok dbs={} total_records={}",
        account_id,
        dump.dbs.len(),
        dump.dbs
            .iter()
            .flat_map(|d| d.stores.iter())
            .map(|s| s.records.len())
            .sum::<usize>(),
    );
    Ok(dump)
}

/// Group messages by peer, emit one `webview:event` per peer, and POST
/// the same payload to `openhuman.memory_doc_ingest`. One memory doc per
/// peer — the transcript inside can be long, each message line still
/// carries its own date + time so the full chronology stays readable.
fn emit_and_persist<R: Runtime>(app: &AppHandle<R>, account_id: &str, harvest: &extract::Harvest) {
    #[derive(Default)]
    struct Group {
        rows: Vec<Value>,
    }
    let mut groups: HashMap<String, Group> = HashMap::new();
    for m in &harvest.messages {
        if m.peer.is_empty() || m.date <= 0 {
            continue;
        }
        let sender_name = if !m.sender.is_empty() {
            harvest
                .users
                .get(&m.sender)
                .cloned()
                .unwrap_or_else(|| m.sender.clone())
        } else {
            String::new()
        };
        let row = json!({
            "date": m.date,
            "sender": sender_name,
            "sender_id": m.sender,
            "body": m.text,
        });
        groups.entry(m.peer.clone()).or_default().rows.push(row);
    }

    let mut emitted = 0usize;
    for (peer_id, group) in groups {
        let mut rows = group.rows;
        rows.sort_by_key(|r| r.get("date").and_then(|v| v.as_i64()).unwrap_or(0));
        // De-duplicate by (date, sender_id, body) — the walker can see the
        // same record in multiple store snapshots, so dedupe is not optional.
        let mut seen: std::collections::HashSet<(i64, String, String)> =
            std::collections::HashSet::new();
        rows.retain(|r| {
            let k = (
                r.get("date").and_then(|v| v.as_i64()).unwrap_or(0),
                r.get("sender_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                r.get("body")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            );
            seen.insert(k)
        });
        if rows.is_empty() {
            continue;
        }
        let peer_name = harvest
            .users
            .get(&peer_id)
            .cloned()
            .or_else(|| harvest.chats.get(&peer_id).cloned())
            .unwrap_or_else(|| peer_id.clone());

        let payload = json!({
            "provider": "telegram",
            "source": "cdp-idb",
            "peerId": peer_id,
            "peerName": peer_name,
            "selfId": harvest.self_id.clone().unwrap_or_default(),
            "messages": rows,
        });
        let envelope = json!({
            "account_id": account_id,
            "provider": "telegram",
            "kind": "ingest",
            "payload": payload.clone(),
            "ts": chrono_now_millis(),
        });
        if let Err(e) = app.emit("webview:event", &envelope) {
            log::warn!("[tg][{}] ingest emit failed: {}", account_id, e);
        } else {
            emitted += 1;
        }
        let acct = account_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = post_memory_doc_ingest(&acct, &payload).await {
                log::warn!("[tg][{}] memory write failed: {}", acct, e);
            }
        });
    }
    log::info!("[tg][{}] emitted {} peer doc(s)", account_id, emitted);
}

/// Unix seconds → UTC `YYYY-MM-DD` (Howard Hinnant civil-from-days).
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

fn chrono_now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Build and POST the `openhuman.memory_doc_ingest` payload for a single
/// peer transcript. Mirrors `slack_scanner::post_memory_doc_ingest`.
async fn post_memory_doc_ingest(account_id: &str, ingest: &Value) -> Result<(), String> {
    let peer_id = ingest
        .get("peerId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let peer_name = ingest
        .get("peerName")
        .and_then(|v| v.as_str())
        .unwrap_or(peer_id);
    let self_id = ingest
        .get("selfId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let empty: Vec<Value> = Vec::new();
    let msgs = ingest
        .get("messages")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    if peer_id.is_empty() || msgs.is_empty() {
        return Ok(());
    }

    let mut sorted: Vec<&Value> = msgs.iter().collect();
    sorted.sort_by_key(|m| m.get("date").and_then(|v| v.as_i64()).unwrap_or(0));

    let first_ts = sorted
        .first()
        .and_then(|m| m.get("date"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let last_ts = sorted
        .last()
        .and_then(|m| m.get("date"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let transcript: String = sorted
        .iter()
        .map(|m| {
            let ts = m.get("date").and_then(|v| v.as_i64()).unwrap_or(0);
            let stamp = if ts > 0 {
                let day = seconds_to_ymd(ts);
                let secs_of_day = (ts.rem_euclid(86_400)) as u32;
                format!(
                    "{} {:02}:{:02}Z",
                    day,
                    secs_of_day / 3600,
                    (secs_of_day / 60) % 60
                )
            } else {
                "?".to_string()
            };
            let who = m
                .get("sender")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("?");
            let body = m
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .replace(['\r', '\n'], " ");
            format!("[{stamp}] {who}: {body}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let first_day = if first_ts > 0 {
        seconds_to_ymd(first_ts)
    } else {
        String::new()
    };
    let last_day = if last_ts > 0 {
        seconds_to_ymd(last_ts)
    } else {
        String::new()
    };
    let header = format!(
        "# Telegram — {peer}\npeer_id: {peer_id}\naccount_id: {account_id}\nmessages: {n}\nrange: {first_day} → {last_day}\n\n",
        peer = peer_name,
        peer_id = peer_id,
        account_id = account_id,
        n = sorted.len(),
        first_day = first_day,
        last_day = last_day,
    );
    let content = format!("{header}{transcript}");

    // Key = peer name when clean, falling back to the raw peer id.
    // `:` is reserved by the memory layer (it rewrites to `_`).
    let namespace = format!("telegram-web:{account_id}");
    let key = if peer_key_looks_clean(peer_name) {
        peer_name.to_string()
    } else {
        peer_id.to_string()
    };
    let title = format!("Telegram · {peer_name}");

    let params = json!({
        "namespace": namespace,
        "key": key,
        "title": title,
        "content": content,
        "source_type": "telegram-web",
        "priority": "medium",
        "tags": ["telegram", "peer-transcript"],
        "metadata": {
            "provider": "telegram",
            "account_id": account_id,
            "peer_id": peer_id,
            "peer_name": peer_name,
            "self_id": self_id,
            "first_day": first_day,
            "last_day": last_day,
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

    let url = std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:7788/rpc".to_string());
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
        "[tg][{}] memory upsert ok namespace={} key={} msgs={} range={}→{}",
        account_id,
        namespace,
        key,
        sorted.len(),
        first_day,
        last_day,
    );
    Ok(())
}

/// Allow a peer name as a memory-doc key only if it stays within a
/// conservative ASCII-ish slug shape. Reject anything with `:` (reserved
/// by the memory layer), spaces, or non-ASCII; those fall back to the
/// stable peer id. Telegram titles are often unicode / contain spaces, so
/// this will frequently return false — that's the safe default.
fn peer_key_looks_clean(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
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

/// Minimal CDP client — keeps a WebSocket open and sends JSON-RPC requests
/// with auto-incrementing ids. Same pattern as `slack_scanner::CdpConn`;
/// kept per-module rather than factored out to avoid coupling scanners
/// until we actually need to share state.
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

    pub(crate) async fn call(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        self.call_with_timeout(method, params, session_id, Duration::from_secs(30))
            .await
    }

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
        let body = serde_json::to_string(&req).map_err(|e| format!("encode: {e}"))?;
        self.sink
            .send(Message::Text(body))
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
}

const DOM_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Fast DOM-only poll — runs every 2s, emits an `ingest` webview:event
/// only when the row-set hash changes. Pure CDP: DOMSnapshot.captureSnapshot
/// runs at the browser's C++ layer, no JS executes in the page world.
fn spawn_dom_poll<R: Runtime>(app: AppHandle<R>, account_id: String, url_prefix: String) {
    tokio::spawn(async move {
        let fragment = crate::cdp::target_url_fragment(&account_id);
        // Wait long enough for tweb to populate the chatlist — polling
        // before that would just emit empty ingests.
        sleep(Duration::from_secs(8)).await;
        let mut last_hash: Option<u64> = None;
        loop {
            match dom_scan_once(&url_prefix, &fragment).await {
                Ok(scan) => {
                    if !scan.rows.is_empty() && Some(scan.hash) != last_hash {
                        log::info!(
                            "[tg][{}] dom scan rows={} unread={} hash={:x}",
                            account_id,
                            scan.rows.len(),
                            scan.total_unread,
                            scan.hash
                        );
                        last_hash = Some(scan.hash);
                        let envelope = json!({
                            "account_id": account_id,
                            "provider": "telegram",
                            "kind": "ingest",
                            "payload": dom_snapshot::ingest_payload(&scan),
                            "ts": chrono_now_millis(),
                        });
                        if let Err(e) = app.emit("webview:event", &envelope) {
                            log::warn!("[tg][{}] dom ingest emit failed: {}", account_id, e);
                        }
                    }
                }
                Err(e) => {
                    log::debug!("[tg][{}] dom scan: {}", account_id, e);
                }
            }
            sleep(DOM_POLL_INTERVAL).await;
        }
    });
}

async fn dom_scan_once(
    url_prefix: &str,
    url_fragment: &str,
) -> Result<dom_snapshot::DomScan, String> {
    let prefix = url_prefix.to_string();
    let fragment = url_fragment.to_string();
    let (mut cdp, session) = crate::cdp::connect_and_attach_matching(move |t| {
        t.url.starts_with(&prefix) && t.url.contains(&fragment)
    })
    .await?;
    let scan = dom_snapshot::scan(&mut cdp, &session).await;
    crate::cdp::detach_session(&mut cdp, &session).await;
    scan
}

/// Registry to prevent double-spawning scanners for the same account.
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
            log::debug!("[tg] scanner already running for {}", account_id);
            return;
        }
        spawn_scanner(app, account_id, url_prefix);
    }

    pub async fn forget(&self, account_id: &str) {
        let mut g = self.started.lock().await;
        g.remove(account_id);
    }
}
