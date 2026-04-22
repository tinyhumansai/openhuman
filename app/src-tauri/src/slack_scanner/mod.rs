//! Slack Web scanner driven purely over the Chrome DevTools Protocol (CDP).
//!
//! Pairs with the embedded CEF webview's remote-debugging port (set in
//! `lib.rs`). One polling loop per tracked Slack account:
//!
//!   * **IDB tick** (`IDB_SCAN_INTERVAL`, 30s) — walks every Slack-owned
//!     IndexedDB database via CDP (`IndexedDB.requestDatabaseNames`,
//!     `IndexedDB.requestDatabase`, `IndexedDB.requestData`), materialises
//!     `Runtime.RemoteObject` records into JSON with a fixed, Slack-agnostic
//!     serializer (`function(){return [this].concat(arguments);}`), and
//!     recursively extracts message / user / channel records from the
//!     Redux-persist snapshots Slack stores there. No in-page JavaScript
//!     runs beyond that one fixed serializer, and no DOM scraping.
//!
//! Emits `webview:event` ingest events (for any listening React UI) AND
//! POSTs `openhuman.memory_doc_ingest` directly to the core so memory is
//! populated whether or not the main window is open. Messages are grouped
//! by (channel_id, day) so each day's transcript upserts a single doc.
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

use crate::cdp::{CDP_HOST, CDP_PORT};

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
    spawn_dom_poll(app.clone(), account_id.clone(), url_prefix.clone());
    tokio::spawn(async move {
        let fragment = crate::cdp::target_url_fragment(&account_id);
        log::info!(
            "[sl] scanner up account={} url_prefix={} fragment={} interval={:?}",
            account_id,
            url_prefix,
            fragment,
            IDB_SCAN_INTERVAL,
        );
        // Let Slack hydrate Redux from IDB before the first scan —
        // otherwise we'd race an empty store on cold start.
        sleep(Duration::from_secs(10)).await;

        loop {
            match scan_once(&account_id, &url_prefix, &fragment).await {
                Ok(dump) => {
                    let team_id = infer_team_id(&dump);
                    let (messages, users, channels, workspace_name) = extract::harvest(&dump);
                    log::info!(
                        "[sl][{}] idb extract: {} msgs, {} users, {} channels, team={} workspace={}",
                        account_id,
                        messages.len(),
                        users.len(),
                        channels.len(),
                        team_id.as_deref().unwrap_or("?"),
                        workspace_name.as_deref().unwrap_or("?"),
                    );
                    if !messages.is_empty() {
                        emit_and_persist(
                            &app,
                            &account_id,
                            &messages,
                            &users,
                            &channels,
                            team_id.as_deref().unwrap_or(""),
                            workspace_name.as_deref().unwrap_or(""),
                        );
                    }
                }
                Err(e) => {
                    log::warn!("[sl][{}] idb scan failed: {}", account_id, e);
                }
            }
            sleep(IDB_SCAN_INTERVAL).await;
        }
    });
}

/// Single scan cycle: open CDP, attach to the Slack page, walk IDB, detach.
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
        .ok_or_else(|| format!("no page target matching {url_prefix} fragment={url_fragment}"))?;

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
        "[sl][{}] scan ok dbs={} total_records={}",
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

/// Slack names its per-workspace DB `objectStore-<TEAM_ID>-<USER_ID>`.
/// Pull the `T…` token from the middle. Returns None if no such DB
/// exists — in which case we fall back to the `id`-shape match in
/// `extract::walk` (any record with `id.starts_with('T')`).
fn infer_team_id(dump: &idb::IdbDump) -> Option<String> {
    for db in &dump.dbs {
        if let Some(rest) = db.name.strip_prefix("objectStore-") {
            // e.g. "T01CWHNCJ9Z-U01CT9ADP6H"
            let team = rest.split('-').next().unwrap_or("");
            if team.starts_with('T')
                && team.len() >= 9
                && team.chars().all(|c| c.is_ascii_alphanumeric())
            {
                return Some(team.to_string());
            }
        }
    }
    None
}

/// Group messages by channel (no per-day split), emit one
/// `webview:event` per channel, and POST the same payload to
/// `openhuman.memory_doc_ingest`. One memory doc per channel — the
/// transcript inside can be long, each message line still carries its
/// date so the full chronology stays readable.
#[allow(clippy::too_many_arguments)]
fn emit_and_persist<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    messages: &[extract::ExtractedMessage],
    users: &HashMap<String, String>,
    channels: &HashMap<String, String>,
    team_id: &str,
    workspace_name: &str,
) {
    #[derive(Default)]
    struct Group {
        rows: Vec<Value>,
    }
    let mut groups: HashMap<String, Group> = HashMap::new();
    for m in messages {
        if m.channel.is_empty() || m.ts.is_empty() {
            continue;
        }
        let ts_secs = parse_slack_ts(&m.ts).unwrap_or(0);
        if ts_secs <= 0 {
            continue;
        }
        let sender = users
            .get(&m.user)
            .cloned()
            .or_else(|| {
                if m.user.is_empty() {
                    None
                } else {
                    Some(m.user.clone())
                }
            })
            .unwrap_or_default();
        let row = json!({
            "ts": m.ts,
            "ts_secs": ts_secs,
            "sender": sender,
            "user_id": m.user,
            "body": m.text,
        });
        groups.entry(m.channel.clone()).or_default().rows.push(row);
    }

    let mut emitted = 0usize;
    for (channel_id, group) in groups {
        let mut rows = group.rows;
        rows.sort_by(|a, b| {
            a.get("ts_secs")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .cmp(&b.get("ts_secs").and_then(|v| v.as_i64()).unwrap_or(0))
        });
        // De-duplicate within the channel by `ts` (Slack messages are
        // unique per-channel per-ts). The walker can see the same record
        // in multiple Redux snapshots, so dedupe is not optional.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        rows.retain(|r| {
            let ts = r
                .get("ts")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            !ts.is_empty() && seen.insert(ts)
        });
        if rows.is_empty() {
            continue;
        }
        let channel_name = channels
            .get(&channel_id)
            .cloned()
            .unwrap_or_else(|| channel_id.clone());

        let payload = json!({
            "provider": "slack",
            "source": "cdp-idb",
            "teamId": team_id,
            "workspaceName": workspace_name,
            "channelId": channel_id,
            "channelName": channel_name,
            "messages": rows,
        });
        let envelope = json!({
            "account_id": account_id,
            "provider": "slack",
            "kind": "ingest",
            "payload": payload.clone(),
            "ts": chrono_now_millis(),
        });
        if let Err(e) = app.emit("webview:event", &envelope) {
            log::warn!("[sl][{}] ingest emit failed: {}", account_id, e);
        } else {
            emitted += 1;
        }
        let acct = account_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = post_memory_doc_ingest(&acct, &payload).await {
                log::warn!("[sl][{}] memory write failed: {}", acct, e);
            }
        });
    }
    log::info!("[sl][{}] emitted {} channel doc(s)", account_id, emitted);
}

/// Parse Slack's `"unix_seconds.microseconds"` ts string to unix seconds.
pub(crate) fn parse_slack_ts(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    s.split('.').next()?.parse::<i64>().ok()
}

/// Slack ts shape check: `<10 digits>.<1-8 digits>`.
pub(crate) fn looks_like_slack_ts(s: &str) -> bool {
    let bytes = s.as_bytes();
    let dot = match s.find('.') {
        Some(i) => i,
        None => return false,
    };
    if !(9..=11).contains(&dot) {
        return false;
    }
    if !bytes[..dot].iter().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let frac = &bytes[dot + 1..];
    if frac.is_empty() || frac.len() > 8 {
        return false;
    }
    frac.iter().all(|b| b.is_ascii_digit())
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
/// (channel, day) group. Mirrors `whatsapp_scanner::post_memory_doc_ingest`.
async fn post_memory_doc_ingest(account_id: &str, ingest: &Value) -> Result<(), String> {
    let channel_id = ingest
        .get("channelId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let channel_name = ingest
        .get("channelName")
        .and_then(|v| v.as_str())
        .unwrap_or(channel_id);
    let team_id = ingest
        .get("teamId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let workspace_name = ingest
        .get("workspaceName")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let empty: Vec<Value> = Vec::new();
    let msgs = ingest
        .get("messages")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    if channel_id.is_empty() || msgs.is_empty() {
        return Ok(());
    }

    let mut sorted: Vec<&Value> = msgs.iter().collect();
    sorted.sort_by_key(|m| m.get("ts_secs").and_then(|v| v.as_i64()).unwrap_or(0));

    let first_ts = sorted
        .first()
        .and_then(|m| m.get("ts_secs"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let last_ts = sorted
        .last()
        .and_then(|m| m.get("ts_secs"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    // Full-channel transcript — every line carries its own date + time so
    // the reader can scan chronology without needing per-day splits.
    let transcript: String = sorted
        .iter()
        .map(|m| {
            let ts = m.get("ts_secs").and_then(|v| v.as_i64()).unwrap_or(0);
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
        "# Slack — {workspace} · #{channel}\nchannel_id: {channel_id}\nteam_id: {team_id}\naccount_id: {account_id}\nmessages: {n}\nrange: {first_day} → {last_day}\n\n",
        workspace = if workspace_name.is_empty() {
            "workspace"
        } else {
            workspace_name
        },
        channel = channel_name,
        channel_id = channel_id,
        team_id = team_id,
        account_id = account_id,
        n = sorted.len(),
        first_day = first_day,
        last_day = last_day,
    );
    let content = format!("{header}{transcript}");

    // Key = channel name when available (what the user asked for),
    // falling back to the channel id for anonymous DMs / unnamed rooms.
    // `:` is reserved by the memory layer (it rewrites to `_`), other
    // characters pass through. Slack channel names are already lowercase
    // letters/digits/dashes/underscores, so no further sanitisation needed.
    let namespace = format!("slack-web:{account_id}");
    let key = if channels_key_looks_clean(channel_name) {
        channel_name.to_string()
    } else {
        channel_id.to_string()
    };
    let title = format!("Slack · #{channel_name}");

    let params = json!({
        "namespace": namespace,
        "key": key,
        "title": title,
        "content": content,
        "source_type": "slack-web",
        "priority": "medium",
        "tags": ["slack", "channel-transcript"],
        "metadata": {
            "provider": "slack",
            "account_id": account_id,
            "team_id": team_id,
            "workspace_name": workspace_name,
            "channel_id": channel_id,
            "channel_name": channel_name,
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
        "[sl][{}] memory upsert ok namespace={} key={} msgs={} range={}→{}",
        account_id,
        namespace,
        key,
        sorted.len(),
        first_day,
        last_day,
    );
    Ok(())
}

/// Allow a channel name as a memory-doc key only if it looks like a
/// Slack-style slug — lowercase letters, digits, `-`, `_`. Reject
/// anything with `:` (reserved by the memory layer), spaces, or other
/// surprises; those fall back to the stable channel id.
fn channels_key_looks_clean(name: &str) -> bool {
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
/// with auto-incrementing ids. Same pattern as `whatsapp_scanner::CdpConn`;
/// kept per-module rather than factored out to avoid coupling the two
/// scanners until we actually need to share state.
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

fn spawn_dom_poll<R: Runtime>(app: AppHandle<R>, account_id: String, url_prefix: String) {
    tokio::spawn(async move {
        let fragment = crate::cdp::target_url_fragment(&account_id);
        sleep(Duration::from_secs(8)).await;
        let mut last_hash: Option<u64> = None;
        loop {
            match dom_scan_once(&url_prefix, &fragment).await {
                Ok(scan) => {
                    if !scan.rows.is_empty() && Some(scan.hash) != last_hash {
                        log::info!(
                            "[sl][{}] dom scan rows={} unread={} hash={:x}",
                            account_id,
                            scan.rows.len(),
                            scan.total_unread,
                            scan.hash
                        );
                        last_hash = Some(scan.hash);
                        let envelope = json!({
                            "account_id": account_id,
                            "provider": "slack",
                            "kind": "ingest",
                            "payload": dom_snapshot::ingest_payload(&scan),
                            "ts": chrono_now_millis(),
                        });
                        if let Err(e) = app.emit("webview:event", &envelope) {
                            log::warn!("[sl][{}] dom ingest emit failed: {}", account_id, e);
                        }
                    }
                }
                Err(e) => log::debug!("[sl][{}] dom scan: {}", account_id, e),
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
            log::debug!("[sl] scanner already running for {}", account_id);
            return;
        }
        spawn_scanner(app, account_id, url_prefix);
    }

    pub async fn forget(&self, account_id: &str) {
        let mut g = self.started.lock().await;
        g.remove(account_id);
    }
}
