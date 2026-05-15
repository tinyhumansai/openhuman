//! WebSocket Engine.IO / Socket.IO connection loop with automatic reconnection.

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, watch};
use tokio::time::{Duration, Instant};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{http::StatusCode, Error as WsError, Message as WsMessage},
};

use crate::api::models::socket::ConnectionStatus;

use super::event_handlers::{handle_sio_event, parse_sio_event};
use super::manager::{emit_state_change, SharedState};
use super::types::{ConnectionOutcome, WsStream};

/// Maximum HTTP redirect hops to follow during a single WebSocket connect attempt.
///
/// Cloudflare and similar edges return a single 301 (e.g. when the configured
/// `BACKEND_URL` is `http://...` and the server only serves the upgrade over TLS)
/// before the upgrade succeeds. Three hops is enough headroom for chained
/// redirects while still bounding pathological loops.
const MAX_REDIRECT_HOPS: u8 = 3;

// ---------------------------------------------------------------------------
// Background loop
// ---------------------------------------------------------------------------

/// Number of consecutive `ConnectionOutcome::Failed` attempts at which the
/// loop fires exactly one `error`-level log (and therefore one Sentry event).
/// Below the threshold, repeated transient failures (gateway 5xx, TLS
/// handshake resets, DNS blips) stay at `warn` and don't reach the Sentry
/// tracing layer. Above the threshold, subsequent retries return to `warn` —
/// the one-shot `error` at the threshold is sufficient to page on a sustained
/// outage without generating unbounded events.
///
/// The value is 5 intentionally: the Sentry event fires on the **5th
/// consecutive failed attempt**, which corresponds to ~15 seconds of
/// accumulated backoff sleep (1 s + 2 s + 4 s + 8 s before the 5th try).
/// Transient blips that recover within 4 attempts produce zero Sentry noise;
/// sustained outages produce exactly one event per affected client.
///
/// See OPENHUMAN-TAURI-8M — a single gateway 503 incident generated 549
/// Sentry events because every retry was logged at `error`.
const FAIL_ESCALATE_THRESHOLD: u32 = 5;

/// Background loop that manages the WebSocket connection and reconnection.
pub(super) async fn ws_loop(
    url: String,
    token: String,
    shared: Arc<SharedState>,
    mut emit_rx: mpsc::UnboundedReceiver<String>,
    mut shutdown_rx: watch::Receiver<bool>,
    internal_tx: mpsc::UnboundedSender<String>,
) {
    // Defense in depth: `SocketManager::connect` is the primary guard and
    // will reject an empty token before spawning this task. This check
    // covers any future caller that invokes `ws_loop` directly (e.g. tests
    // or internal plumbing bypassing the manager). Without it, every attempt
    // would either 401 at the SIO CONNECT step or fail upstream at the
    // gateway, producing the retry-storm Sentry noise this module is designed
    // to suppress.
    if token.trim().is_empty() {
        log::error!("[socket] ws_loop: refusing to start — empty session token");
        *shared.status.write() = ConnectionStatus::Disconnected;
        *shared.socket_id.write() = None;
        emit_state_change(&shared);
        return;
    }

    let mut backoff = Duration::from_millis(1000);
    let max_backoff = Duration::from_secs(30);
    let mut consecutive_failures: u32 = 0;

    // `ws_url` is the *resolved* socket URL we're currently connecting to.
    // If the backend responds with an HTTP 3xx during the upgrade (typical when
    // BACKEND_URL is configured as `http://` and the edge forces TLS), we
    // follow the Location header and pin the resolved URL here so subsequent
    // reconnects skip the redirect round-trip entirely.
    let mut ws_url = crate::api::socket::websocket_url(&url);

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        log::info!("[socket] Attempting connection...");
        *shared.status.write() = ConnectionStatus::Connecting;
        emit_state_change(&shared);

        let outcome = run_connection(
            &mut ws_url,
            &token,
            &shared,
            &mut emit_rx,
            &mut shutdown_rx,
            &internal_tx,
        )
        .await;

        match outcome {
            ConnectionOutcome::Shutdown => {
                log::info!("[socket] Clean shutdown");
                break;
            }
            ConnectionOutcome::Lost(reason) => {
                // `Lost` is only returned after a successful SIO CONNECT ACK
                // (see `run_connection`), so reaching this arm proves the
                // backend is reachable and the token is valid. Reset both
                // the backoff and the failure streak.
                if consecutive_failures > 0 {
                    log::debug!(
                        "[socket] Connection re-established; resetting failure streak ({} cleared)",
                        consecutive_failures
                    );
                }
                consecutive_failures = 0;
                log::warn!("[socket] Connection lost: {}", reason);
                backoff = Duration::from_millis(1000);
            }
            ConnectionOutcome::Failed(reason) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                log_connection_failure(consecutive_failures, &reason);
                // keep growing backoff
            }
        }

        *shared.status.write() = ConnectionStatus::Disconnected;
        *shared.socket_id.write() = None;
        emit_state_change(&shared);

        if *shutdown_rx.borrow() {
            break;
        }

        log::info!("[socket] Reconnecting in {:?}...", backoff);
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() { break; }
            }
        }
        backoff = (backoff * 2).min(max_backoff);
    }

    log::info!("[socket] WebSocket loop exiting");
    *shared.status.write() = ConnectionStatus::Disconnected;
    *shared.socket_id.write() = None;
    emit_state_change(&shared);
}

// ---------------------------------------------------------------------------
// Failure logging
// ---------------------------------------------------------------------------

/// Log a connection failure at the appropriate level based on how many
/// consecutive failures have occurred.
///
/// - Below `FAIL_ESCALATE_THRESHOLD`: `warn` — transient blips (DNS, gateway
///   5xx, TLS resets) stay out of Sentry.
/// - Exactly at the threshold: routed through
///   [`crate::core::observability::report_error_or_expected`] so transport-
///   level user-environment shapes (`network is unreachable`, `dns error`,
///   `connection refused/reset`, `tls handshake`) demote to a `warn`
///   breadcrumb while genuine outages (gateway 5xx, server-side WebSocket
///   close, malformed handshake) fire exactly one Sentry event per affected
///   client.
/// - Above the threshold: `warn` — already paged once; avoid unbounded events
///   during a long outage.
///
/// Extracted as a pure function so it can be unit-tested without running an
/// async event loop or touching the WS stack.
fn log_connection_failure(consecutive: u32, reason: &str) {
    if consecutive == FAIL_ESCALATE_THRESHOLD {
        // Route the one-shot sustained-outage escalation through the
        // observability classifier so an offline user (no wifi / airplane mode
        // / `Network is unreachable (os error 51)` — see OPENHUMAN-TAURI-BH)
        // does not page on every affected client. Sentry has no signal to act
        // on a user being offline — no status, no trace, no payload — so the
        // event was pure noise. Genuine outage shapes (gateway 5xx, malformed
        // handshake, …) don't match the classifier and still fire one Sentry
        // event per affected client, preserving the OPENHUMAN-TAURI-8M intent.
        let detailed = format!(
            "[socket] Connection failed (sustained outage after {consecutive} attempts): {reason}"
        );
        let attempts = consecutive.to_string();
        crate::core::observability::report_error_or_expected(
            detailed.as_str(),
            "socket",
            "ws_connect",
            &[("attempts", attempts.as_str())],
        );
    } else {
        // Below threshold (transient blips) or above threshold (already fired
        // the one-shot error): stay at `warn` so subsequent retries don't pile
        // up additional Sentry events.
        log::warn!(
            "[socket] Connection failed (attempt {}/{}): {}",
            consecutive,
            FAIL_ESCALATE_THRESHOLD,
            reason
        );
    }
}

// ---------------------------------------------------------------------------
// Single connection attempt
// ---------------------------------------------------------------------------

/// Run a single WebSocket connection through handshake and event loop.
///
/// `ws_url` is taken by mutable reference so that any HTTP redirect we follow
/// during the upgrade (see `connect_with_redirects`) is pinned for the next
/// reconnect attempt — we don't want to re-hit the redirect every time the
/// loop backs off and retries.
async fn run_connection(
    ws_url: &mut String,
    token: &str,
    shared: &Arc<SharedState>,
    emit_rx: &mut mpsc::UnboundedReceiver<String>,
    shutdown_rx: &mut watch::Receiver<bool>,
    internal_tx: &mpsc::UnboundedSender<String>,
) -> ConnectionOutcome {
    log::info!("[socket] WS URL: {}", ws_url);

    // 2. Connect via WebSocket (uses rustls TLS for wss://). Follow HTTP 3xx
    //    redirects up to MAX_REDIRECT_HOPS so a `http://` config behind a
    //    Cloudflare-style edge that 301s to `https://` connects cleanly
    //    instead of looping at error-level forever.
    let ws_stream = match connect_with_redirects(ws_url, shared).await {
        Ok(stream) => stream,
        Err(e) => return ConnectionOutcome::Failed(format!("WebSocket connect: {e}")),
    };

    log::info!("[socket] WebSocket connected, starting handshake");
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // 3. Read Engine.IO OPEN packet (type 0)
    let open_data =
        match tokio::time::timeout(Duration::from_secs(10), read_eio_open(&mut ws_read)).await {
            Ok(Ok(data)) => data,
            Ok(Err(e)) => return ConnectionOutcome::Failed(format!("EIO OPEN: {e}")),
            Err(_) => return ConnectionOutcome::Failed("Timeout waiting for EIO OPEN".into()),
        };

    let ping_interval = open_data
        .get("pingInterval")
        .and_then(|v| v.as_u64())
        .unwrap_or(25000);
    let ping_timeout_ms = open_data
        .get("pingTimeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(20000);
    let eio_sid = open_data.get("sid").and_then(|v| v.as_str()).unwrap_or("?");
    log::info!(
        "[socket] EIO OPEN: sid={}, ping={}ms, timeout={}ms",
        eio_sid,
        ping_interval,
        ping_timeout_ms
    );

    // 4. Send Socket.IO CONNECT with auth token
    let connect_payload = json!({"token": token});
    let connect_msg = format!("40{}", serde_json::to_string(&connect_payload).unwrap());
    if let Err(e) = ws_write.send(WsMessage::Text(connect_msg)).await {
        return ConnectionOutcome::Failed(format!("Send SIO CONNECT: {e}"));
    }

    // 5. Read Socket.IO CONNECT ACK (type 40)
    let ack_data =
        match tokio::time::timeout(Duration::from_secs(10), read_sio_connect_ack(&mut ws_read))
            .await
        {
            Ok(Ok(data)) => data,
            Ok(Err(e)) => return ConnectionOutcome::Failed(format!("SIO CONNECT: {e}")),
            Err(_) => {
                return ConnectionOutcome::Failed("Timeout waiting for SIO CONNECT ACK".into())
            }
        };

    let sio_sid = ack_data
        .get("sid")
        .and_then(|v| v.as_str())
        .map(String::from);
    log::info!("[socket] SIO CONNECT ACK: sid={:?}", sio_sid);

    // 6. Update state to Connected
    *shared.status.write() = ConnectionStatus::Connected;
    *shared.socket_id.write() = sio_sid;
    emit_state_change(shared);

    // 7. Main event loop
    let timeout_duration = Duration::from_millis(ping_interval + ping_timeout_ms + 5000);
    let mut deadline = Instant::now() + timeout_duration;

    loop {
        tokio::select! {
            msg = ws_read.next() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        deadline = Instant::now() + timeout_duration;
                        handle_eio_message(&text, internal_tx, shared);
                    }
                    Some(Ok(WsMessage::Ping(data))) => {
                        let _ = ws_write.send(WsMessage::Pong(data)).await;
                    }
                    Some(Ok(WsMessage::Close(_))) => {
                        log::info!("[socket] Server closed WebSocket");
                        return ConnectionOutcome::Lost("Server closed connection".into());
                    }
                    Some(Err(e)) => {
                        return ConnectionOutcome::Lost(format!("WebSocket error: {e}"));
                    }
                    None => {
                        return ConnectionOutcome::Lost("WebSocket stream ended".into());
                    }
                    _ => {} // Binary, Pong, Frame
                }
            }
            outgoing = emit_rx.recv() => {
                match outgoing {
                    Some(msg) => {
                        if let Err(e) = ws_write.send(WsMessage::Text(msg)).await {
                            return ConnectionOutcome::Lost(format!("Send failed: {e}"));
                        }
                    }
                    None => {
                        let _ = ws_write.send(WsMessage::Close(None)).await;
                        return ConnectionOutcome::Shutdown;
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                log::warn!(
                    "[socket] Ping timeout ({}ms)",
                    ping_interval + ping_timeout_ms + 5000
                );
                return ConnectionOutcome::Lost("Ping timeout".into());
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    log::info!("[socket] Shutdown signal received");
                    let _ = ws_write.send(WsMessage::Close(None)).await;
                    return ConnectionOutcome::Shutdown;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Handshake helpers
// ---------------------------------------------------------------------------

/// Read the Engine.IO OPEN packet (type 0) from the WebSocket.
///
/// Format: `0{"sid":"...","upgrades":[],"pingInterval":25000,"pingTimeout":20000}`
async fn read_eio_open(
    ws_read: &mut futures_util::stream::SplitStream<WsStream>,
) -> Result<serde_json::Value, String> {
    loop {
        match ws_read.next().await {
            Some(Ok(WsMessage::Text(text))) => {
                let s: &str = &text;
                if let Some(json_str) = s.strip_prefix('0') {
                    return serde_json::from_str(json_str)
                        .map_err(|e| format!("Parse EIO OPEN JSON: {e}"));
                }
                log::debug!(
                    "[socket] Skipping non-OPEN packet: {}",
                    &s[..s.len().min(40)]
                );
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(format!("WS error during handshake: {e}")),
            None => return Err("WebSocket closed before OPEN".into()),
        }
    }
}

/// Read the Socket.IO CONNECT ACK (type 40) from the WebSocket.
///
/// Format: `40{"sid":"..."}` or `44{"message":"error"}` for connect error.
async fn read_sio_connect_ack(
    ws_read: &mut futures_util::stream::SplitStream<WsStream>,
) -> Result<serde_json::Value, String> {
    loop {
        match ws_read.next().await {
            Some(Ok(WsMessage::Text(text))) => {
                let s: &str = &text;
                // Engine.IO MESSAGE (4) + Socket.IO CONNECT (0)
                if let Some(json_str) = s.strip_prefix("40") {
                    if json_str.is_empty() {
                        return Ok(json!({}));
                    }
                    return serde_json::from_str(json_str)
                        .map_err(|e| format!("Parse CONNECT ACK: {e}"));
                }
                // Engine.IO MESSAGE (4) + Socket.IO CONNECT_ERROR (4)
                if let Some(json_str) = s.strip_prefix("44") {
                    let err: serde_json::Value =
                        serde_json::from_str(json_str).unwrap_or(json!({"message": "unknown"}));
                    let msg = err
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Connect error");
                    return Err(format!("Socket.IO connect error: {msg}"));
                }
                // Engine.IO PING (2) — respond via log, can't write from here
                if s.starts_with('2') {
                    log::debug!("[socket] EIO ping during handshake (will respond after)");
                    continue;
                }
                log::debug!(
                    "[socket] Skipping packet during SIO handshake: {}",
                    &s[..s.len().min(40)]
                );
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(format!("WS error during SIO handshake: {e}")),
            None => return Err("WebSocket closed before CONNECT ACK".into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Message handling
// ---------------------------------------------------------------------------

/// Handle an incoming Engine.IO text message by its type prefix.
fn handle_eio_message(
    text: &str,
    emit_tx: &mpsc::UnboundedSender<String>,
    shared: &Arc<SharedState>,
) {
    if text.is_empty() {
        return;
    }

    match text.as_bytes()[0] {
        b'2' => {
            // Engine.IO PING → respond with PONG
            let _ = emit_tx.send("3".to_string());
        }
        b'3' => {
            // Engine.IO PONG — ignore (server responding to our ping)
        }
        b'4' => {
            // Engine.IO MESSAGE → contains Socket.IO packet
            if text.len() > 1 {
                handle_sio_packet(&text[1..], emit_tx, shared);
            }
        }
        b'1' => {
            log::info!("[socket] Engine.IO CLOSE from server");
        }
        b'6' => {
            // Engine.IO NOOP
        }
        _ => {
            log::debug!(
                "[socket] Unknown EIO packet: {}",
                &text[..text.len().min(30)]
            );
        }
    }
}

/// Handle a Socket.IO packet (after stripping the Engine.IO '4' prefix).
fn handle_sio_packet(
    text: &str,
    emit_tx: &mpsc::UnboundedSender<String>,
    shared: &Arc<SharedState>,
) {
    if text.is_empty() {
        return;
    }

    match text.as_bytes()[0] {
        b'2' => {
            // Socket.IO EVENT: 2["eventName", data]
            if let Some((event_name, data)) = parse_sio_event(&text[1..]) {
                handle_sio_event(&event_name, data, emit_tx, shared);
            } else {
                log::warn!(
                    "[socket] Failed to parse SIO EVENT: {}",
                    &text[..text.len().min(80)]
                );
            }
        }
        b'0' => {
            // Socket.IO CONNECT (re-ack during reconnection) — update sid
            log::debug!("[socket] SIO CONNECT re-ack");
            if text.len() > 1 {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text[1..]) {
                    if let Some(sid) = data.get("sid").and_then(|v| v.as_str()) {
                        *shared.socket_id.write() = Some(sid.to_string());
                        emit_state_change(shared);
                    }
                }
            }
        }
        b'1' => {
            // Socket.IO DISCONNECT
            log::info!("[socket] SIO DISCONNECT from server");
            *shared.status.write() = ConnectionStatus::Disconnected;
            *shared.socket_id.write() = None;
            emit_state_change(shared);
        }
        b'4' => {
            // Socket.IO CONNECT_ERROR
            let error_str = if text.len() > 1 {
                &text[1..]
            } else {
                "unknown"
            };
            log::error!("[socket] SIO CONNECT_ERROR: {}", error_str);
        }
        _ => {
            log::debug!(
                "[socket] Unknown SIO packet type: {}",
                &text[..text.len().min(30)]
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Redirect-following connect
// ---------------------------------------------------------------------------

/// Connect to `ws_url`, following HTTP 3xx redirects up to `MAX_REDIRECT_HOPS`.
///
/// Plain `connect_async` returns an error on any non-`101 Switching Protocols`
/// response, so a Cloudflare-style `http://… → https://…` 301 (which happens
/// whenever `BACKEND_URL` is configured without TLS) used to be fatal — the
/// reconnect loop would hammer the same dead URL forever at error level.
///
/// On each redirect we:
///   1. resolve the `Location` header against the current URL (handles relative
///      Location values),
///   2. upgrade the scheme so the next attempt is still a WebSocket
///      (`http` → `ws`, `https` → `wss`; `ws`/`wss` pass through),
///   3. mutate `ws_url` in place so the redirect target is pinned for
///      subsequent reconnects (no need to re-hit the redirect every retry),
///   4. record a one-shot warning in `SharedState.error` the first time we
///      follow a redirect so the UI can surface "your `BACKEND_URL` is stale".
///
/// On non-redirect failures the original error is returned and the caller
/// counts it toward the exponential backoff like before.
async fn connect_with_redirects(
    ws_url: &mut String,
    shared: &Arc<SharedState>,
) -> Result<WsStream, WsError> {
    let original = ws_url.clone();
    for hop in 0..=MAX_REDIRECT_HOPS {
        match connect_async(ws_url.as_str()).await {
            Ok((stream, _response)) => return Ok(stream),
            Err(WsError::Http(response)) if is_redirect_status(response.status()) => {
                if hop == MAX_REDIRECT_HOPS {
                    log::error!(
                        "[socket] Exceeded {MAX_REDIRECT_HOPS} redirect hops starting from {original}; giving up"
                    );
                    return Err(WsError::Http(response));
                }
                let location = match extract_location_header(&response) {
                    Some(loc) => loc,
                    None => {
                        log::error!(
                            "[socket] Redirect {} from {ws_url} missing Location header",
                            response.status()
                        );
                        return Err(WsError::Http(response));
                    }
                };
                let next_url = match resolve_redirect_target(ws_url, &location) {
                    Ok(url) => url,
                    Err(e) => {
                        log::error!(
                            "[socket] Cannot follow redirect to {location} from {ws_url}: {e}"
                        );
                        return Err(WsError::Http(response));
                    }
                };
                log::warn!(
                    "[socket] Server redirected ({}) {} → {}",
                    response.status(),
                    ws_url,
                    next_url
                );
                // Only persist a stale-BACKEND_URL warning for permanent
                // redirects (301 / 308). Temporary redirects (302 / 307) say
                // "this time, go elsewhere" — the configured BACKEND_URL is
                // still correct, and surfacing a "please update config" hint
                // for a transient hop would be misleading. Per CodeRabbit
                // review on PR #1547.
                if matches!(
                    response.status(),
                    StatusCode::MOVED_PERMANENTLY | StatusCode::PERMANENT_REDIRECT
                ) {
                    record_redirect_warning(shared, &original, &next_url);
                }
                *ws_url = next_url;
            }
            Err(e) => return Err(e),
        }
    }
    // Unreachable: the loop either returns Ok, returns the redirect error after
    // exhausting hops, or returns a non-redirect Err.
    unreachable!("connect_with_redirects exited loop without returning")
}

/// Statuses we treat as "follow the Location and retry".
///
/// 308 (Permanent Redirect) and 307 (Temporary Redirect) explicitly preserve
/// the method; 301/302 historically do too for upgrade requests in practice.
/// Anything else (300, 304, ...) stays an error.
fn is_redirect_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

fn extract_location_header(
    response: &tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
) -> Option<String> {
    response
        .headers()
        .get(tokio_tungstenite::tungstenite::http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// Resolve `location` against `current_ws_url` and rewrite the scheme so the
/// result is still a valid WebSocket URL.
///
/// `location` may be absolute (`https://host/path?q=1`) or relative
/// (`/socket.io/?EIO=4`). We use the `url` crate's relative-URL parser to do
/// the join the same way browsers do, then map `http`→`ws` / `https`→`wss`.
fn resolve_redirect_target(current_ws_url: &str, location: &str) -> Result<String, String> {
    let base = url::Url::parse(current_ws_url).map_err(|e| format!("invalid current URL: {e}"))?;
    let resolved = base
        .join(location)
        .map_err(|e| format!("invalid Location {location:?}: {e}"))?;

    let upgraded_scheme = match resolved.scheme() {
        "http" => "ws",
        "https" => "wss",
        "ws" | "wss" => resolved.scheme(),
        other => return Err(format!("unsupported scheme in Location: {other}")),
    };

    let mut next = resolved.clone();
    next.set_scheme(upgraded_scheme)
        .map_err(|_| format!("failed to set scheme {upgraded_scheme} on {resolved}"))?;
    Ok(next.to_string())
}

/// Persist a one-shot, user-visible warning that the backend redirected the
/// configured socket URL. Subsequent redirects in the same connect attempt
/// don't overwrite — the first hop carries the actionable signal.
fn record_redirect_warning(shared: &Arc<SharedState>, original: &str, resolved: &str) {
    let mut slot = shared.error.write();
    if slot.is_some() {
        return;
    }
    *slot = Some(format!(
        "Backend redirected {original} → {resolved}. Update BACKEND_URL to the resolved URL to avoid the extra hop."
    ));
}

#[cfg(test)]
#[path = "ws_loop_tests.rs"]
mod tests;
