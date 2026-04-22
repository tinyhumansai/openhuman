//! Discord HTTP + WebSocket MITM driven over the Chrome DevTools Protocol.
//!
//! Pairs with the embedded CEF webview's remote-debugging port (set in
//! `lib.rs` via `--remote-debugging-port=9222`). One persistent task per
//! tracked Discord account that:
//!
//!   1. Discovers the page target whose URL starts with `https://discord.com`
//!   2. Attaches with `flatten: true`, enables `Network.*`
//!   3. Streams every `Network.requestWillBeSent`, `Network.responseReceived`,
//!      `Network.webSocketCreated`, `Network.webSocketFrameSent` /
//!      `Network.webSocketFrameReceived` event for that session
//!   4. Filters to `discord.com/api/...` HTTP traffic and gateway WS frames,
//!      forwards each match as a `webview:event` envelope (same shape the
//!      WhatsApp / Slack scanners emit) with `provider: "discord"` and
//!      `kind: "ingest"`
//!
//! V1 is observation-only: outbound HTTP request bodies (`request.postData`)
//! and full WebSocket frames are captured directly off the CDP event stream
//! with no follow-up calls. Inbound HTTP response bodies require a separate
//! `Network.getResponseBody` round-trip per request and are skipped here —
//! see TODO at `dispatch_event` for the upgrade path. Discord's gateway is
//! the source of truth for live messages anyway, so V1 covers the live-feed
//! use case without the extra round-trip cost.
//!
//! NOTE: only built with the `cef` feature — wry has no remote-debugging
//! port and never gets compiled in.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::{oneshot, Mutex};
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

mod dom_snapshot;

use crate::cdp::{CDP_HOST, CDP_PORT};

/// How long to wait between reconnect attempts when the CDP WebSocket drops
/// or the page target disappears (e.g. Discord refresh, navigation).
const RECONNECT_BACKOFF: Duration = Duration::from_secs(3);

/// CDP target descriptor (subset of `Target.TargetInfo`).
#[derive(Debug, Clone)]
struct CdpTarget {
    id: String,
    kind: String,
    url: String,
}

/// Spawn the per-account MITM task. Idempotent at call site — caller guards
/// double-spawn via `ScannerRegistry::ensure_scanner`.
pub fn spawn_scanner<R: Runtime>(app: AppHandle<R>, account_id: String, url_prefix: String) {
    spawn_dom_poll(app.clone(), account_id.clone(), url_prefix.clone());
    tokio::spawn(async move {
        let fragment = crate::cdp::target_url_fragment(&account_id);
        log::info!(
            "[discord][{}] mitm up url_prefix={} fragment={} cdp={}:{}",
            account_id,
            url_prefix,
            fragment,
            CDP_HOST,
            CDP_PORT
        );
        // Let Discord's bootstrap (auth + gateway handshake) settle before
        // we attach — `Network.enable` issued during the cold-start burst
        // tends to race with the renderer's own initialization and we miss
        // the first few frames anyway.
        sleep(Duration::from_secs(4)).await;
        loop {
            match run_mitm_session(&app, &account_id, &url_prefix, &fragment).await {
                Ok(()) => {
                    log::info!(
                        "[discord][{}] session ended cleanly, reconnecting",
                        account_id
                    );
                }
                Err(e) => {
                    log::warn!(
                        "[discord][{}] session failed: {} — reconnecting in {:?}",
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

/// Run one CDP attach → enable → stream-events lifecycle. Returns when the
/// underlying WebSocket closes, the page target disappears, or any
/// dispatch hits an unrecoverable error. Caller loops.
async fn run_mitm_session<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    url_prefix: &str,
    url_fragment: &str,
) -> Result<(), String> {
    let browser_ws = browser_ws_url().await?;
    let mut cdp = CdpConn::open(&browser_ws).await?;

    // Find the discord page target. We don't subscribe to target lifecycle
    // events for V1 — if the user reloads or navigates, the outer loop
    // re-attaches on the next iteration. Cheap and predictable.
    let targets_v = cdp.call("Target.getTargets", json!({}), None).await?;
    let targets = parse_targets(&targets_v);
    log::debug!("[discord][{}] {} targets total", account_id, targets.len());
    let page = targets
        .iter()
        .find(|t| {
            t.kind == "page" && t.url.starts_with(url_prefix) && t.url.ends_with(url_fragment)
        })
        .ok_or_else(|| format!("no page target matching {url_prefix} fragment={url_fragment}"))?;
    log::info!(
        "[discord][{}] attaching to target {} url={}",
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

    // Enable the Network domain on the page session — this is what unlocks
    // the `requestWillBeSent` / `webSocketFrame*` event stream we care about.
    cdp.call("Network.enable", json!({}), Some(&session_id))
        .await?;
    log::info!(
        "[discord][{}] Network.enable ok session={}",
        account_id,
        session_id
    );

    // Now drop into the pure event read loop until the WS closes. Any
    // outstanding `cdp.call` requests will complete via the shared id-keyed
    // dispatch in `pump_events`.
    cdp.pump_events(app, account_id, &session_id).await
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

// ---------- CDP connection ----------------------------------------------------

/// CDP client tuned for **streaming** workloads — unlike the request/reply
/// `CdpConn` used by `whatsapp_scanner` and `slack_scanner`, this one keeps
/// a pending-id table so the read loop can deliver responses to the right
/// caller AND surface inbound CDP events at the same time. Required for
/// MITM because we need to listen continuously to `Network.*` events while
/// occasionally issuing a `Network.getResponseBody` (V1.5).
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
    /// id → oneshot waiting for the matching response.
    pending: HashMap<i64, oneshot::Sender<Result<Value, String>>>,
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
            pending: HashMap::new(),
        })
    }

    /// One-shot CDP call — only safe to use **before** `pump_events` takes
    /// ownership of the read stream. After that, callers must use the
    /// pending-table machinery (not exposed yet — V1 needs no in-stream
    /// calls). For the current setup phase (`Target.getTargets`,
    /// `Target.attachToTarget`, `Network.enable`) we drain inline.
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
            let msg = tokio::time::timeout(Duration::from_secs(15), self.stream.next())
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
            // Inbound CDP events have `method` but no `id`. During setup we
            // can safely drop them — `Network.enable` is the last setup
            // call, so nothing we care about is in flight yet.
            if v.get("id").and_then(|x| x.as_i64()) != Some(id) {
                continue;
            }
            if let Some(err) = v.get("error") {
                return Err(format!("cdp error: {err}"));
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Take over the read stream and dispatch every inbound message until
    /// the WebSocket closes. Events route through `dispatch_event`;
    /// responses route through `pending` (unused in V1 but plumbed so V1.5
    /// can issue `Network.getResponseBody` without a redesign).
    async fn pump_events<R: Runtime>(
        &mut self,
        app: &AppHandle<R>,
        account_id: &str,
        session_id: &str,
    ) -> Result<(), String> {
        log::info!("[discord][{}] event pump started", account_id);
        loop {
            // No timeout here — Discord's gateway sends heartbeats every
            // ~41s, but a fully idle channel can sit silent for minutes.
            // We rely on the WS layer's own keepalive + the outer reconnect
            // loop in `spawn_scanner` to recover from genuine drops.
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
                    log::info!("[discord][{}] cdp ws closed", account_id);
                    return Ok(());
                }
            };
            let v: Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("[discord][{}] decode failed: {}", account_id, e);
                    continue;
                }
            };
            if let Some(id) = v.get("id").and_then(|x| x.as_i64()) {
                // Response to one of our calls. Hand it off.
                if let Some(tx) = self.pending.remove(&id) {
                    let res = if let Some(err) = v.get("error") {
                        Err(format!("cdp error: {err}"))
                    } else {
                        Ok(v.get("result").cloned().unwrap_or(Value::Null))
                    };
                    let _ = tx.send(res);
                }
                continue;
            }
            // Event: dispatch by method.
            let method = v.get("method").and_then(|x| x.as_str()).unwrap_or("");
            // Ignore events for sessions we didn't attach to (CDP
            // multiplexes everything through one ws once flatten=true).
            let evt_session = v.get("sessionId").and_then(|x| x.as_str()).unwrap_or("");
            if !evt_session.is_empty() && evt_session != session_id {
                continue;
            }
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            dispatch_event(app, account_id, method, &params);
        }
    }
}

// ---------- Event filter & emit ----------------------------------------------

/// Dispatch one CDP event. Filters down to:
///   * `Network.requestWillBeSent` for `discord.com/api/` URLs (captures
///     outbound POST/PATCH/DELETE bodies — sent messages, edits, reactions)
///   * `Network.responseReceived` for `discord.com/api/` URLs (captures
///     status + meta; body is a TODO — see V1.5 note above)
///   * `Network.webSocketCreated` for `gateway.discord` URLs (logs only)
///   * `Network.webSocketFrameSent` / `Network.webSocketFrameReceived` for
///     gateway connections (gateway op codes 0/1/etc — Discord's live
///     message stream)
///
/// Everything else (image loads, css, telemetry pings, voice WS, ...) is
/// dropped silently to keep noise out of the event stream.
fn dispatch_event<R: Runtime>(app: &AppHandle<R>, account_id: &str, method: &str, params: &Value) {
    match method {
        "Network.requestWillBeSent" => {
            let url = params
                .pointer("/request/url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !is_discord_api(url) {
                return;
            }
            let req_method = params
                .pointer("/request/method")
                .and_then(|v| v.as_str())
                .unwrap_or("GET")
                .to_string();
            // postData isn't always present on GETs — that's fine, just
            // null it out. For POST/PATCH/PUT it's the JSON Discord is
            // about to send, which is the bit we actually want.
            let post_data = params
                .pointer("/request/postData")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let request_id = params
                .get("requestId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            log::debug!(
                "[discord][{}] http→ {} {} req_id={} body_len={}",
                account_id,
                req_method,
                url,
                request_id,
                post_data.as_ref().map(|s| s.len()).unwrap_or(0)
            );
            emit(
                app,
                account_id,
                "ingest",
                json!({
                    "provider": "discord",
                    "source": "cdp-http-request",
                    "request_id": request_id,
                    "url": url,
                    "method": req_method,
                    "request_body": post_data,
                }),
            );
        }
        "Network.responseReceived" => {
            let url = params
                .pointer("/response/url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !is_discord_api(url) {
                return;
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
            let request_id = params
                .get("requestId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            log::debug!(
                "[discord][{}] http← {} {} status={} mime={}",
                account_id,
                request_id,
                url,
                status,
                mime
            );
            // V1.5 TODO: schedule a `Network.getResponseBody` call here
            // (via the pending-table machinery in CdpConn) to attach the
            // response body. For now we emit meta so React can correlate
            // with the requestWillBeSent event by request_id.
            emit(
                app,
                account_id,
                "ingest",
                json!({
                    "provider": "discord",
                    "source": "cdp-http-response",
                    "request_id": request_id,
                    "url": url,
                    "status": status,
                    "mime_type": mime,
                    "response_body": Value::Null,
                }),
            );
        }
        "Network.webSocketCreated" => {
            let url = params.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if !is_discord_gateway(url) {
                return;
            }
            let request_id = params
                .get("requestId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            log::info!(
                "[discord][{}] ws-open req_id={} url={}",
                account_id,
                request_id,
                url
            );
            emit(
                app,
                account_id,
                "log",
                json!({
                    "level": "info",
                    "msg": format!("discord gateway opened: {url}"),
                    "request_id": request_id,
                }),
            );
        }
        m @ ("Network.webSocketFrameSent" | "Network.webSocketFrameReceived") => {
            // We don't have URL on frame events — only the requestId. We
            // emit unconditionally; consumers can drop frames whose
            // request_id never appeared in a `webSocketCreated` for the
            // gateway. Cheap, and avoids missing the very first frames
            // (which fire before our event filter sees the create event
            // sometimes, depending on attach-vs-handshake timing).
            let direction = if m.ends_with("Sent") {
                "sent"
            } else {
                "received"
            };
            let opcode = params
                .pointer("/response/opcode")
                .and_then(|v| v.as_i64())
                .unwrap_or(-1);
            let payload = params
                .pointer("/response/payloadData")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mask = params
                .pointer("/response/mask")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let request_id = params
                .get("requestId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            log::trace!(
                "[discord][{}] ws-{} req_id={} opcode={} bytes={} mask={}",
                account_id,
                direction,
                request_id,
                opcode,
                payload.len(),
                mask
            );
            emit(
                app,
                account_id,
                "ingest",
                json!({
                    "provider": "discord",
                    "source": "cdp-ws",
                    "request_id": request_id,
                    "direction": direction,
                    "opcode": opcode,
                    // `payloadData` is text for opcode 1, base64 for opcode 2
                    // (binary). Discord defaults to JSON over text frames; if
                    // the user enables zlib/zstd compression we'll see
                    // base64'd binary here and the consumer needs to decode.
                    "payload_data": payload,
                    "mask": mask,
                }),
            );
        }
        _ => {} // ignore everything else
    }
}

fn is_discord_api(url: &str) -> bool {
    // Match `https://discord.com/api/v9/...`, `/api/v10/...`, etc. Filter
    // out the static asset CDN (`cdn.discordapp.com`, `media.discordapp.net`)
    // and the analytics pings — those would drown the event stream with
    // useless noise.
    url.starts_with("https://discord.com/api/")
        || url.starts_with("https://canary.discord.com/api/")
        || url.starts_with("https://ptb.discord.com/api/")
}

fn is_discord_gateway(url: &str) -> bool {
    // Real-time message stream lives on `gateway.discord.gg`; voice/RTC
    // negotiation lives on `*.discord.media` and isn't useful for message
    // mirroring.
    url.starts_with("wss://gateway.discord.gg") || url.starts_with("wss://gateway-")
}

fn emit<R: Runtime>(app: &AppHandle<R>, account_id: &str, kind: &str, payload: Value) {
    let envelope = json!({
        "account_id": account_id,
        "provider": "discord",
        "kind": kind,
        "payload": payload,
        "ts": chrono_now_millis(),
    });
    if let Err(e) = app.emit("webview:event", &envelope) {
        log::warn!("[discord][{}] emit failed: {}", account_id, e);
    }
}

fn chrono_now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------- DOM chat-list poll ----------------------------------------------

const DOM_POLL_INTERVAL: Duration = Duration::from_secs(2);

fn spawn_dom_poll<R: Runtime>(app: AppHandle<R>, account_id: String, url_prefix: String) {
    tokio::spawn(async move {
        let fragment = crate::cdp::target_url_fragment(&account_id);
        sleep(Duration::from_secs(6)).await;
        let mut last_hash: Option<u64> = None;
        let mut voice_active = false;
        loop {
            match dom_scan_once(&url_prefix, &fragment).await {
                Ok(scan) => {
                    if Some(scan.hash) != last_hash {
                        log::info!(
                            "[discord][{}] dom scan rows={} unread={} hash={:x} voice={}",
                            account_id,
                            scan.rows.len(),
                            scan.total_unread,
                            scan.hash,
                            scan.voice.active,
                        );
                        last_hash = Some(scan.hash);
                        let envelope = json!({
                            "account_id": account_id,
                            "provider": "discord",
                            "kind": "ingest",
                            "payload": dom_snapshot::ingest_payload(&scan),
                            "ts": chrono_now_millis(),
                        });
                        if let Err(e) = app.emit("webview:event", &envelope) {
                            log::warn!("[discord][{}] dom ingest emit failed: {}", account_id, e);
                        }
                    }

                    // Emit voice call lifecycle events on state transitions.
                    let now_active = scan.voice.active;
                    if now_active && !voice_active {
                        let channel_id = scan
                            .voice
                            .channel_name
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string());
                        let channel_name = channel_id.clone();
                        let guild_name = scan
                            .voice
                            .guild_name
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string());
                        let started_at = chrono_now_millis();
                        log::info!(
                            "[discord][{}] voice call started channel={:?} guild={:?}",
                            account_id,
                            channel_name,
                            guild_name,
                        );
                        let call_evt = json!({
                            "account_id": account_id,
                            "provider": "discord",
                            "kind": "discord_call_started",
                            "payload": {
                                "channelId": channel_id,
                                "channelName": channel_name,
                                "guildName": guild_name,
                                "url": url_prefix,
                                "startedAt": started_at,
                            },
                            "ts": started_at,
                        });
                        if let Err(e) = app.emit("webview:event", &call_evt) {
                            log::warn!(
                                "[discord][{}] discord_call_started emit failed: {}",
                                account_id,
                                e
                            );
                        }
                    } else if !now_active && voice_active {
                        let ended_at = chrono_now_millis();
                        log::info!("[discord][{}] voice call ended", account_id);
                        let end_evt = json!({
                            "account_id": account_id,
                            "provider": "discord",
                            "kind": "discord_call_ended",
                            "payload": {
                                "channelId": "unknown",
                                "endedAt": ended_at,
                                "reason": "disconnected",
                            },
                            "ts": ended_at,
                        });
                        if let Err(e) = app.emit("webview:event", &end_evt) {
                            log::warn!(
                                "[discord][{}] discord_call_ended emit failed: {}",
                                account_id,
                                e
                            );
                        }
                    }
                    voice_active = now_active;
                }
                Err(e) => log::debug!("[discord][{}] dom scan: {}", account_id, e),
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
        t.url.starts_with(&prefix) && t.url.ends_with(&fragment)
    })
    .await?;
    let scan = dom_snapshot::scan(&mut cdp, &session).await;
    crate::cdp::detach_session(&mut cdp, &session).await;
    scan
}

// ---------- Registry ---------------------------------------------------------

/// Tracks which accounts already have a MITM task running so the webview
/// open-lifecycle can call `ensure_scanner` repeatedly without
/// double-spawning. Same shape as the WhatsApp / Slack registries so the
/// `webview_accounts` wiring is uniform.
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
            log::debug!("[discord] mitm already running for {}", account_id);
            return;
        }
        spawn_scanner(app, account_id, url_prefix);
    }

    pub async fn forget(&self, account_id: &str) {
        let mut g = self.started.lock().await;
        g.remove(account_id);
    }
}
