//! Discord HTTP + WebSocket MITM driven over the Chrome DevTools Protocol.
//!
//! Attaches to the embedded CEF webview via the in-process CDP transport
//! installed by `webview_accounts::open` (no TCP listener). One persistent
//! task per tracked Discord account that:
//!
//!   1. Discovers the page target whose URL starts with `https://discord.com`
//!   2. Attaches with `flatten: true`, enables `Network.*`
//!   3. Streams every `Network.requestWillBeSent`, `Network.responseReceived`,
//!      `Network.webSocketCreated`, `Network.webSocketFrameSent` /
//!      `Network.webSocketFrameReceived` event for that session
//!   4. Filters to `discord.com/api/...` HTTP traffic and gateway WS frames,
//!      then turns gateway message events into per-channel transcript updates
//!      that are emitted to the UI and written straight into core memory.
//!
//! V1 parses live gateway events only. Outbound HTTP request bodies
//! (`request.postData`) are observed for debugging, but transcript ingest is
//! driven by `MESSAGE_CREATE` / `MESSAGE_UPDATE` frames from the gateway.
//! Inbound HTTP response bodies still require a `Network.getResponseBody`
//! round-trip and are left as a future backfill upgrade.
//!
//! NOTE: only built with the `cef` feature — wry has no remote-debugging
//! port and never gets compiled in.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::watch;
use tokio::task::AbortHandle;
use tokio::time::sleep;

mod dom_snapshot;

/// How long to wait between reconnect attempts when the CDP WebSocket drops
/// or the page target disappears (e.g. Discord refresh, navigation).
const RECONNECT_BACKOFF: Duration = Duration::from_secs(3);
const MAX_CHANNEL_MESSAGES: usize = 400;
/// Idle window after which the event pump assumes the attached page target is
/// stale/destroyed (reload, renderer crash, hard navigation) and returns so
/// the outer loop re-attaches. Chosen at >2x Discord's ~41s gateway heartbeat:
/// a live session always emits gateway WS frames within this window, so a
/// longer silence means the session is dead, not merely quiet.
const PUMP_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscordPersistMessage {
    id: String,
    author: String,
    author_id: String,
    body: String,
    timestamp_ms: i64,
    source_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscordChannelSnapshot {
    channel_id: String,
    channel_name: String,
    guild_id: Option<String>,
    messages: Vec<DiscordPersistMessage>,
}

#[derive(Default)]
struct DiscordChannelState {
    name: Option<String>,
    guild_id: Option<String>,
    messages: Vec<DiscordPersistMessage>,
}

#[derive(Default)]
struct MemoryUpsertRegistry {
    workers: Mutex<HashMap<String, watch::Sender<Value>>>,
}

#[derive(Default)]
struct DiscordIngestState {
    channels: HashMap<String, DiscordChannelState>,
}

static MEMORY_UPSERT_REGISTRY: OnceLock<MemoryUpsertRegistry> = OnceLock::new();

impl DiscordIngestState {
    fn apply_gateway_payload(&mut self, payload: &str) -> Vec<DiscordChannelSnapshot> {
        let event: Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        if event.get("op").and_then(|v| v.as_i64()) != Some(0) {
            return Vec::new();
        }
        let kind = event.get("t").and_then(|v| v.as_str()).unwrap_or("");
        let data = event.get("d").cloned().unwrap_or(Value::Null);
        match kind {
            "READY" => {
                if let Some(channels) = data.get("private_channels").and_then(|v| v.as_array()) {
                    for channel in channels {
                        self.apply_channel_meta(channel, None);
                    }
                }
                Vec::new()
            }
            "GUILD_CREATE" => {
                let guild_id = data
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                if let Some(channels) = data.get("channels").and_then(|v| v.as_array()) {
                    for channel in channels {
                        self.apply_channel_meta(channel, guild_id.clone());
                    }
                }
                if let Some(threads) = data.get("threads").and_then(|v| v.as_array()) {
                    for thread in threads {
                        self.apply_channel_meta(thread, guild_id.clone());
                    }
                }
                Vec::new()
            }
            "CHANNEL_CREATE" | "CHANNEL_UPDATE" | "THREAD_CREATE" | "THREAD_UPDATE" => {
                let channel_id = data
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                self.apply_channel_meta(
                    &data,
                    data.get("guild_id")
                        .and_then(|v| v.as_str())
                        .map(ToOwned::to_owned),
                );
                channel_id
                    .and_then(|id| self.snapshot_for_channel(&id))
                    .into_iter()
                    .collect()
            }
            "MESSAGE_CREATE" => self.apply_message_event(&data, false).into_iter().collect(),
            "MESSAGE_UPDATE" => self.apply_message_event(&data, true).into_iter().collect(),
            _ => Vec::new(),
        }
    }

    fn apply_channel_meta(&mut self, value: &Value, fallback_guild_id: Option<String>) {
        let Some(channel_id) = value.get("id").and_then(|v| v.as_str()) else {
            return;
        };
        let state = self.channels.entry(channel_id.to_string()).or_default();
        if let Some(name) = channel_label(value) {
            state.name = Some(name);
        }
        if state.guild_id.is_none() {
            state.guild_id = value
                .get("guild_id")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
                .or(fallback_guild_id);
        }
    }

    fn apply_message_event(
        &mut self,
        value: &Value,
        is_update: bool,
    ) -> Option<DiscordChannelSnapshot> {
        let channel_id = value
            .get("channel_id")
            .and_then(|v| v.as_str())?
            .to_string();
        let message_id = value.get("id").and_then(|v| v.as_str())?.to_string();
        let body = discord_message_body(value);
        let timestamp_ms = value
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(parse_discord_timestamp_ms)
            .unwrap_or_else(chrono_now_millis);
        let guild_id = value
            .get("guild_id")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);
        let author_id = value
            .get("author")
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);
        let author = discord_author_label(value);
        let source_ref = discord_message_permalink(value, &channel_id, &message_id);

        let state = self.channels.entry(channel_id.clone()).or_default();
        if let Some(name) = channel_label(value) {
            state.name = Some(name);
        }
        if state.guild_id.is_none() {
            state.guild_id = guild_id;
        }

        if let Some(existing) = state.messages.iter_mut().find(|m| m.id == message_id) {
            if discord_message_body_should_replace(value) {
                if let Some(next_body) = body {
                    existing.body = next_body;
                }
            } else if !is_update && body.is_none() {
                return None;
            } else if body.is_none() && discord_message_body_fields_present(value) {
                log::warn!(
                    "[discord][{}] message update omitted transcript body fields for id={}",
                    channel_id,
                    message_id
                );
            }
            if let Some(next_author_id) = author_id {
                existing.author_id = next_author_id;
                if !author.is_empty() && author != "?" {
                    existing.author = author;
                }
            }
            if value.get("timestamp").is_some() {
                existing.timestamp_ms = timestamp_ms;
            }
            existing.source_ref = source_ref;
        } else {
            let next = DiscordPersistMessage {
                id: message_id.clone(),
                author: if author.is_empty() {
                    "?".to_string()
                } else {
                    author
                },
                author_id: author_id.unwrap_or_default(),
                body: body?,
                timestamp_ms,
                source_ref,
            };
            state.messages.push(next);
        }
        state
            .messages
            .sort_by_key(|m| (m.timestamp_ms, m.id.clone()));
        if state.messages.len() > MAX_CHANNEL_MESSAGES {
            let drop_n = state.messages.len() - MAX_CHANNEL_MESSAGES;
            state.messages.drain(0..drop_n);
        }

        Some(DiscordChannelSnapshot {
            channel_id: channel_id.clone(),
            channel_name: state
                .name
                .clone()
                .unwrap_or_else(|| format!("channel-{channel_id}")),
            guild_id: state.guild_id.clone(),
            messages: state.messages.clone(),
        })
    }

    fn snapshot_for_channel(&self, channel_id: &str) -> Option<DiscordChannelSnapshot> {
        let state = self.channels.get(channel_id)?;
        if state.messages.is_empty() {
            return None;
        }
        Some(DiscordChannelSnapshot {
            channel_id: channel_id.to_string(),
            channel_name: state
                .name
                .clone()
                .unwrap_or_else(|| format!("channel-{channel_id}")),
            guild_id: state.guild_id.clone(),
            messages: state.messages.clone(),
        })
    }
}

/// Spawn the per-account MITM task. Idempotent at call site — caller guards
/// double-spawn via `ScannerRegistry::ensure_scanner`.
pub fn spawn_scanner<R: Runtime>(
    app: AppHandle<R>,
    account_id: String,
    url_prefix: String,
) -> Vec<AbortHandle> {
    let mut handles = Vec::with_capacity(2);
    handles.push(spawn_dom_poll(
        app.clone(),
        account_id.clone(),
        url_prefix.clone(),
    ));
    let task = tokio::spawn(async move {
        let fragment = crate::cdp::target_url_fragment(&account_id);
        log::info!(
            "[discord][{}] mitm up url_prefix={} fragment={} (in-process CDP)",
            account_id,
            url_prefix,
            fragment,
        );
        // Let Discord's bootstrap (auth + gateway handshake) settle before
        // we attach — `Network.enable` issued during the cold-start burst
        // tends to race with the renderer's own initialization and we miss
        // the first few frames anyway.
        sleep(Duration::from_secs(4)).await;
        // Lock onto the page target once a strict fragment match succeeds, so
        // re-attaches after a reload survive Discord stripping the URL hash
        // (see `attach_account_target`). Persists across reconnects.
        let mut pinned_target_id: Option<String> = None;
        loop {
            match run_mitm_session(
                &app,
                &account_id,
                &url_prefix,
                &fragment,
                &mut pinned_target_id,
            )
            .await
            {
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
    handles.push(task.abort_handle());
    handles
}

/// Run one CDP attach → enable → stream-events lifecycle. Returns when the
/// in-process transport closes (webview torn down) or when the pump's idle
/// watchdog trips after `PUMP_IDLE_TIMEOUT` of no frames — i.e. the attached
/// page target went stale (Discord reload, renderer crash, hard navigation).
/// The caller's outer loop then re-attaches. `pinned_target_id` carries the
/// pin/strict/relaxed resolution state across reconnects (see
/// [`attach_account_target`]).
async fn run_mitm_session<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    url_prefix: &str,
    url_fragment: &str,
    pinned_target_id: &mut Option<String>,
) -> Result<(), String> {
    let (mut cdp, session_id) =
        attach_account_target(app, account_id, url_prefix, url_fragment, pinned_target_id)
            .await
            .map_err(|e| format!("attach: {e}"))?;
    log::info!(
        "[discord][{}] attached label={} session={}",
        account_id,
        cdp.label(),
        session_id
    );

    // Enable the Network domain on the page session — this is what unlocks
    // the `requestWillBeSent` / `webSocketFrame*` event stream we care about.
    cdp.call("Network.enable", json!({}), Some(&session_id))
        .await?;
    log::info!(
        "[discord][{}] Network.enable ok session={}",
        account_id,
        session_id
    );

    // Drop into the event read loop. It returns when the in-process transport
    // closes (webview gone) OR when the idle watchdog fires after
    // `PUMP_IDLE_TIMEOUT` of no frames (stale/destroyed page target) — either
    // way the outer loop re-attaches. The resilient pump also buffers bursts
    // into an unbounded queue so a flood that overflows the broadcast ring
    // isn't silently dropped. V1 doesn't issue any in-stream calls (responses
    // table from the previous TCP impl is gone — re-introduce a
    // request/response API here when V1.5 backfills `Network.getResponseBody`).
    log::info!("[discord][{}] event pump started", account_id);
    let mut ingest_state = DiscordIngestState::default();
    let pump_result = cdp
        .pump_events_resilient(&session_id, PUMP_IDLE_TIMEOUT, |method, params| {
            dispatch_event(app, account_id, method, params, &mut ingest_state);
        })
        .await;
    // Detach the now-stale session before the outer loop re-attaches, so idle /
    // lag-forced reconnects don't accumulate orphaned CDP sessions on the
    // transport (mirrors the DOM-scan cleanup).
    crate::cdp::detach_session(&mut cdp, &session_id).await;
    pump_result
}

/// Pure pin → strict → relaxed target-selection core of
/// [`attach_account_target`]. Returns the chosen page target and whether it was
/// a strict fragment match (the caller pins only on `true`). Split out so the
/// resolution hierarchy is unit-testable without a live CDP transport.
fn resolve_page_target<'a>(
    targets: &'a [crate::cdp::target::CdpTarget],
    url_prefix: &str,
    url_fragment: &str,
    pinned_target_id: Option<&str>,
) -> Option<(&'a crate::cdp::target::CdpTarget, bool)> {
    // 1. Pinned id (locked on a prior strict match) — survives the hash strip.
    //    Still require the prefix: a pinned tab can navigate off Discord while
    //    keeping its target id, and we must not keep scanning an off-prefix page.
    if let Some(pid) = pinned_target_id {
        if let Some(t) = targets
            .iter()
            .find(|t| t.id == pid && t.kind == "page" && t.url.starts_with(url_prefix))
        {
            return Some((t, false));
        }
    }
    // 2. Strict fragment match — the only result that proves account ownership.
    if let Some(t) = targets.iter().find(|t| {
        t.kind == "page" && t.url.starts_with(url_prefix) && t.url.ends_with(url_fragment)
    }) {
        return Some((t, true));
    }
    // 3. Relaxed prefix-only — last resort; safe under per-account data-dir isolation.
    targets
        .iter()
        .find(|t| t.kind == "page" && t.url.starts_with(url_prefix))
        .map(|t| (t, false))
}

/// Resolve this account's page target, attach, and return the live
/// [`CdpConn`](crate::cdp::CdpConn) plus session id.
///
/// Discord's web client `replaceState`s to its canonical `/channels/...` URL
/// on boot, stripping the `#openhuman-account-<id>` fragment the webview was
/// opened with — so a strict `ends_with(fragment)` match only holds for the
/// first instant after navigation and fails forever after (the 4s settle delay
/// alone guarantees we attach *after* the strip). Mirrors the Slack scanner's
/// resolution hierarchy (`slack_scanner::scan_once`) via [`resolve_page_target`]:
///
///   1. **Pinned target id** — once a strict match locked the id, prefer it
///      (still constrained to `url_prefix`). Survives the fragment strip and
///      keeps multi-account sessions from cross-wiring scanner A onto B's tab.
///   2. **Strict fragment match** (`url_prefix` + `#openhuman-account-<id>`).
///      On hit, (re)pin the id into `pinned_target_id`.
///   3. **Relaxed prefix-only match** — last resort. Per-account
///      `data_directory` isolation makes this safe for single-account setups;
///      never persisted into the pin (only a strict match proves ownership).
async fn attach_account_target<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    url_prefix: &str,
    url_fragment: &str,
    pinned_target_id: &mut Option<String>,
) -> Result<(crate::cdp::CdpConn, String), String> {
    let mut cdp = crate::cdp::target::conn_for_account(app, account_id)?;
    let targets_v = cdp.call("Target.getTargets", json!({}), None).await?;
    let targets = crate::cdp::target::parse_targets(&targets_v);

    let (page_target, is_strict) = resolve_page_target(
        &targets,
        url_prefix,
        url_fragment,
        pinned_target_id.as_deref(),
    )
    .ok_or_else(|| format!("no page target matching {url_prefix} fragment={url_fragment}"))?;

    // (Re)pin on every live strict-fragment match — the one signal that proves
    // this target is *this* account's. Refreshing (not just setting-once) lets a
    // stale pin recover: after a renderer swap gives a new target id, the next
    // strict match re-pins instead of being stuck on relaxed forever. Relaxed
    // matches never feed the pin.
    if is_strict && pinned_target_id.as_deref() != Some(page_target.id.as_str()) {
        log::info!(
            "[discord][{}] pinned to target_id={} (strict fragment match)",
            account_id,
            page_target.id
        );
        *pinned_target_id = Some(page_target.id.clone());
    }

    let target_id = page_target.id.clone();
    let attach = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
        )
        .await?;
    let session = attach
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "page attach missing sessionId".to_string())?
        .to_string();
    Ok((cdp, session))
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
fn dispatch_event<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    method: &str,
    params: &Value,
    ingest_state: &mut DiscordIngestState,
) {
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
            // TODO: fetch response bodies with `Network.getResponseBody` if
            // we need backfill beyond what the live gateway stream gives us.
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
            if direction == "received" && opcode == 1 {
                for snapshot in ingest_state.apply_gateway_payload(&payload) {
                    emit_channel_transcript(app, account_id, snapshot);
                }
            }
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

fn parse_discord_timestamp_ms(raw: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|ts| ts.with_timezone(&Utc).timestamp_millis())
}

fn discord_author_label(value: &Value) -> String {
    value
        .get("member")
        .and_then(|v| v.get("nick"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            value
                .get("author")
                .and_then(|v| v.get("global_name"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            value
                .get("author")
                .and_then(|v| v.get("username"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            value
                .get("author")
                .and_then(|v| v.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("?")
        .to_string()
}

fn discord_message_body(value: &Value) -> Option<String> {
    let content = value
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if !content.is_empty() {
        return Some(content);
    }

    let attachment_names = value
        .get("attachments")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.get("filename").and_then(|v| v.as_str()))
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !attachment_names.is_empty() {
        return Some(format!("[attachments] {}", attachment_names.join(", ")));
    }

    let embed_titles = value
        .get("embeds")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.get("title")
                        .and_then(|v| v.as_str())
                        .or_else(|| item.get("description").and_then(|v| v.as_str()))
                })
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !embed_titles.is_empty() {
        return Some(format!("[embed] {}", embed_titles.join(" | ")));
    }

    None
}

fn discord_message_body_fields_present(value: &Value) -> bool {
    value.get("content").is_some()
        || value.get("attachments").is_some()
        || value.get("embeds").is_some()
}

fn discord_message_body_should_replace(value: &Value) -> bool {
    value.get("content").is_some() || value.get("attachments").is_some()
}

fn channel_label(value: &Value) -> Option<String> {
    let direct = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if !direct.is_empty() {
        return Some(direct.to_string());
    }
    let recipients = value
        .get("recipients")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|user| {
                    user.get("global_name")
                        .and_then(|v| v.as_str())
                        .or_else(|| user.get("username").and_then(|v| v.as_str()))
                })
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if recipients.is_empty() {
        None
    } else {
        Some(recipients.join(", "))
    }
}

fn discord_message_permalink(value: &Value, channel_id: &str, message_id: &str) -> String {
    let guild_or_me = value
        .get("guild_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("@me");
    format!("https://discord.com/channels/{guild_or_me}/{channel_id}/{message_id}")
}

fn discord_memory_payload(snapshot: &DiscordChannelSnapshot) -> Value {
    let messages = snapshot
        .messages
        .iter()
        .map(|message| {
            json!({
                "id": message.id,
                "sender": message.author,
                "sender_id": message.author_id,
                "body": message.body,
                "date": message.timestamp_ms.div_euclid(1000),
                "source_ref": message.source_ref,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "provider": "discord",
        "source": "cdp-gateway-chat",
        "channelId": snapshot.channel_id,
        "channelName": snapshot.channel_name,
        "guildId": snapshot.guild_id,
        "messages": messages,
    })
}

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

async fn post_memory_doc_ingest(account_id: &str, ingest: &Value) -> Result<(), String> {
    let channel_id = ingest
        .get("channelId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let channel_name = ingest
        .get("channelName")
        .and_then(|v| v.as_str())
        .unwrap_or(channel_id);
    let guild_id = ingest
        .get("guildId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let empty: Vec<Value> = Vec::new();
    let messages = ingest
        .get("messages")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    if channel_id.is_empty() || messages.is_empty() {
        return Ok(());
    }

    let mut sorted: Vec<&Value> = messages.iter().collect();
    sorted.sort_by_key(|m| {
        (
            m.get("date").and_then(|v| v.as_i64()).unwrap_or(0),
            m.get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        )
    });

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
    let transcript = sorted
        .iter()
        .map(|m| {
            let ts = m.get("date").and_then(|v| v.as_i64()).unwrap_or(0);
            let stamp = if ts > 0 {
                let day = seconds_to_ymd(ts);
                let secs_of_day = ts.rem_euclid(86_400) as u32;
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
        "# Discord — {channel}\nchannel_id: {channel_id}\nguild_id: {guild_id}\naccount_id: {account_id}\nmessages: {count}\nrange: {first_day} → {last_day}\n\n",
        channel = channel_name,
        channel_id = channel_id,
        guild_id = if guild_id.is_empty() { "@me" } else { guild_id },
        account_id = account_id,
        count = sorted.len(),
        first_day = first_day,
        last_day = last_day,
    );
    let doc_key = discord_channel_doc_key(guild_id, channel_id);
    let params = json!({
        "namespace": format!("discord-web:{account_id}"),
        "key": doc_key,
        "title": format!("Discord · {channel_name}"),
        "content": format!("{header}{transcript}"),
        "source_type": "discord-web",
        "priority": "medium",
        "tags": ["discord", "channel-transcript"],
        "metadata": {
            "provider": "discord",
            "account_id": account_id,
            "channel_id": channel_id,
            "channel_name": channel_name,
            "guild_id": guild_id,
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
    let url = crate::core_rpc::core_rpc_url_value();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let req = crate::core_rpc::apply_auth(client.post(&url))
        .map_err(|e| format!("prepare {url}: {e}"))?;
    let resp = req
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
        "[discord][{}] memory upsert ok channel={} key={} msgs={} range={}→{}",
        account_id,
        channel_id,
        discord_channel_doc_key(guild_id, channel_id),
        sorted.len(),
        first_day,
        last_day,
    );
    Ok(())
}

fn discord_channel_doc_key(guild_id: &str, channel_id: &str) -> String {
    if guild_id.is_empty() {
        format!("@me:{channel_id}")
    } else {
        format!("{guild_id}:{channel_id}")
    }
}

fn queue_memory_doc_ingest(account_id: String, payload: Value) {
    let worker_key = memory_worker_key(&account_id, &payload);
    let registry = MEMORY_UPSERT_REGISTRY.get_or_init(MemoryUpsertRegistry::default);
    let sender = {
        let mut workers = registry.workers.lock();
        if let Some(existing) = workers.get(&worker_key) {
            existing.clone()
        } else {
            let (tx, mut rx) = watch::channel(payload.clone());
            let worker_key_for_task = worker_key.clone();
            let account_id_for_task = account_id.clone();
            tokio::spawn(async move {
                let mut first = true;
                loop {
                    if !first && rx.changed().await.is_err() {
                        break;
                    }
                    first = false;
                    let next_payload = rx.borrow().clone();
                    if let Err(e) =
                        post_memory_doc_ingest(&account_id_for_task, &next_payload).await
                    {
                        log::warn!(
                            "[discord][{}] memory write failed worker={} err={}",
                            account_id_for_task,
                            worker_key_for_task,
                            e
                        );
                    }
                }
            });
            workers.insert(worker_key.clone(), tx.clone());
            tx
        }
    };
    if let Err(e) = sender.send(payload) {
        log::warn!(
            "[discord][{}] memory ingest queue send failed worker={} err={}",
            account_id,
            worker_key,
            e
        );
    }
}

fn memory_worker_key(account_id: &str, payload: &Value) -> String {
    let channel_id = payload
        .get("channelId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let guild_id = payload
        .get("guildId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    format!(
        "{account_id}:{}",
        discord_channel_doc_key(guild_id, channel_id)
    )
}

fn emit_channel_transcript<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    snapshot: DiscordChannelSnapshot,
) {
    let payload = discord_memory_payload(&snapshot);
    let envelope = json!({
        "account_id": account_id,
        "provider": "discord",
        "kind": "discord_memory_ingest",
        "payload": payload.clone(),
        "ts": chrono_now_millis(),
    });
    if let Err(e) = app.emit("webview:event", &envelope) {
        log::warn!("[discord][{}] memory ingest emit failed: {}", account_id, e);
    }
    queue_memory_doc_ingest(account_id.to_string(), payload);
}

// ---------- DOM chat-list poll ----------------------------------------------

const DOM_POLL_INTERVAL: Duration = Duration::from_secs(2);

fn spawn_dom_poll<R: Runtime>(
    app: AppHandle<R>,
    account_id: String,
    url_prefix: String,
) -> AbortHandle {
    let task = tokio::spawn(async move {
        let fragment = crate::cdp::target_url_fragment(&account_id);
        sleep(Duration::from_secs(6)).await;
        let mut last_hash: Option<u64> = None;
        let mut pinned_target_id: Option<String> = None;
        loop {
            match dom_scan_once(
                &app,
                &account_id,
                &url_prefix,
                &fragment,
                &mut pinned_target_id,
            )
            .await
            {
                Ok(scan) => {
                    if Some(scan.hash) != last_hash {
                        log::info!(
                            "[discord][{}] dom scan rows={} unread={} hash={:x}",
                            account_id,
                            scan.rows.len(),
                            scan.total_unread,
                            scan.hash
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
                }
                Err(e) => log::debug!("[discord][{}] dom scan: {}", account_id, e),
            }
            sleep(DOM_POLL_INTERVAL).await;
        }
    });
    task.abort_handle()
}

async fn dom_scan_once<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    url_prefix: &str,
    url_fragment: &str,
    pinned_target_id: &mut Option<String>,
) -> Result<dom_snapshot::DomScan, String> {
    let (mut cdp, session) =
        attach_account_target(app, account_id, url_prefix, url_fragment, pinned_target_id).await?;
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
    started: Mutex<HashMap<String, Vec<AbortHandle>>>,
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
            log::debug!("[discord] mitm already running for {}", account_id);
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
            log::info!(
                "[discord] aborted {} scanner task(s) for {}",
                count,
                account_id
            );
        }
    }

    pub fn forget_all(&self) -> usize {
        let entries: Vec<_> = self.started.lock().drain().collect();
        let task_count = entries.iter().map(|(_, handles)| handles.len()).sum();
        for (account_id, handles) in entries {
            for handle in handles {
                handle.abort();
            }
            log::debug!("[discord] aborted scanner tasks for {}", account_id);
        }
        if task_count > 0 {
            log::info!("[discord] aborted {} scanner task(s)", task_count);
        }
        task_count
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
