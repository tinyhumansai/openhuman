use std::sync::Arc;

use parking_lot::Mutex;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, watch};
use tokio::time::{Duration, Instant};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{http::StatusCode, Error as WsError, Message as WsMessage},
};

use crate::api::models::socket::ConnectionStatus;
use crate::openhuman::util::utf8_safe_prefix_at_byte_boundary;

use super::event_handlers::{handle_sio_event, parse_sio_event};
#[cfg(test)]
use super::manager::AckRegistry;
use super::manager::{emit_state_change, SharedState};
use super::token_provider::{is_invalid_token_error, TokenProvider};
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
///
/// `token_provider` is called before **each** connection attempt, so a
/// token that was refreshed or re-stored on disk (e.g. after the user
/// re-logged-in while the loop was sleeping) is picked up automatically.
///
/// On a `"Socket.IO connect error: Invalid token"` rejection the loop
/// performs one extra provider call to check whether a fresher token
/// became available. If the token is unchanged (or the second attempt also
/// fails with "Invalid token") the loop escalates immediately — it does
/// **not** waste the remaining back-off attempts on a provably dead token.
/// This is the fix for TAURI-RUST-9C (#2892).
pub(super) async fn ws_loop(
    url: String,
    token_provider: TokenProvider,
    shared: Arc<SharedState>,
    mut emit_rx: mpsc::UnboundedReceiver<String>,
    mut shutdown_rx: watch::Receiver<bool>,
    internal_tx: mpsc::UnboundedSender<String>,
    emit_ready: Arc<Mutex<bool>>,
) {
    let mut backoff = Duration::from_millis(1000);
    let max_backoff = Duration::from_secs(30);
    let mut consecutive_failures: u32 = 0;
    // How many `RetryImmediately` short-circuits we've taken in the **current**
    // "fresh-token cycle" (i.e. since the last successful connection or
    // non-token-related failure). Bounded to 1 so a buggy / non-deterministic
    // provider that returns a *different* non-empty token on every call cannot
    // hot-loop: connect → Invalid token → fresh token → connect → Invalid token
    // → fresh token → ... with no sleep and no escalation. After one immediate
    // shot we fall through to the normal backoff + escalation path. See the
    // CodeRabbit Major on PR #2905.
    let mut fresh_token_retries: u32 = 0;
    // Fresh token carried forward from a `RetryImmediately` decision. When
    // `decide_after_invalid_token` re-fetches the provider and finds a
    // genuinely different value, we stash it here so the next loop iteration
    // skips the redundant top-of-loop provider call and uses **exactly** the
    // token that was validated by the decision step — avoiding a redundant
    // lock + disk read and the case where the logged fresh-token length
    // drifts from what's actually sent over the wire. See the CodeRabbit
    // Minor on PR #2905.
    let mut pending_token: Option<String> = None;

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

        // If a fresh token was carried forward from the previous iteration's
        // `RetryImmediately` decision, consume it now and skip the redundant
        // provider call — `decide_after_invalid_token` already validated that
        // it is non-empty and distinct from the previously-rejected token, so
        // re-reading the provider here would only re-acquire the same lock /
        // re-hit the same disk file. Otherwise fetch the latest token
        // afresh. If the provider returns an error (no token stored → user
        // is logged out, profile corrupt), there is nothing useful to retry
        // with — surface the error and exit cleanly rather than spamming
        // the server.
        let token = match pending_token.take() {
            Some(t) => t,
            None => match token_provider() {
                Ok(t) if !t.trim().is_empty() => t,
                Ok(_) => {
                    log::warn!("[socket] ws_loop: token provider returned empty token — stopping");
                    *shared.error.write() =
                        Some("session expired — please sign in again".to_string());
                    *shared.status.write() = ConnectionStatus::Disconnected;
                    *shared.socket_id.write() = None;
                    emit_state_change(&shared);
                    return;
                }
                Err(e) => {
                    log::warn!("[socket] ws_loop: token provider failed — stopping: {e}");
                    *shared.error.write() =
                        Some("session expired — please sign in again".to_string());
                    *shared.status.write() = ConnectionStatus::Disconnected;
                    *shared.socket_id.write() = None;
                    emit_state_change(&shared);
                    return;
                }
            },
        };

        log::info!(
            "[socket] Attempting connection (token_len={})...",
            token.len()
        );
        // Record the credential this attempt actually authenticates with. The
        // provider is re-read every iteration, so a session refreshed mid-loop
        // would otherwise leave `SocketManager::is_live_for` comparing against
        // the token this loop was spawned with and tearing down a healthy socket
        // on the next connect (#6181).
        *shared.connection_identity.write() = Some((url.clone(), token.clone()));
        *shared.status.write() = ConnectionStatus::Connecting;
        emit_state_change(&shared);

        let outcome = run_connection(
            &mut ws_url,
            &token,
            &shared,
            &mut emit_rx,
            &mut shutdown_rx,
            &internal_tx,
            &emit_ready,
        )
        .await;

        // The connection is over. Clear this connection's readiness flag and
        // drain whatever is still queued, both under the `emit_ready` lock so a
        // concurrent `emit` cannot slip between them: without the shared lock,
        // `emit` could observe `ready == true`, this teardown could clear+drain,
        // and `emit`'s `tx.send` could then land a message in the just-emptied
        // channel — where the next reconnect (a fresh sid, whose roster the
        // backend has cleared) would forward it. `emit` takes the same lock
        // across its check+send, so that interleaving cannot occur.
        //
        // Clearing also covers a connection that never handshook (`run_connection`
        // only sets `true` after the CONNECT ACK), and draining is the backstop
        // for anything already queued: `emit_rx` is owned by this loop, not by
        // the connection, so a leftover message would otherwise be flushed onto
        // the next socket. It is a no-op on a `Failed` attempt that never flipped
        // the flag or queued anything.
        let dropped = {
            let mut ready = emit_ready.lock();
            *ready = false;
            drain_pending_emits(&mut emit_rx)
        };
        if dropped > 0 {
            log::warn!("[socket] Dropped {dropped} queued emit(s) on disconnect");
        }

        // The connection attempt has ended (lost, failed, or shutdown), so any
        // in-flight `emit_with_ack` waiter can never receive its ACK now. Cancel
        // them here — covering server-driven disconnects (`Lost`) and the
        // session-expired escalation below — not just explicit
        // `SocketManager::disconnect()` (CodeRabbit #4355).
        shared.ack_registry.cancel_all();
        super::medulla::workflows::end_connection_generation();

        match outcome {
            ConnectionOutcome::Shutdown => {
                log::info!("[socket] Clean shutdown");
                break;
            }
            ConnectionOutcome::Lost(reason) => {
                // `Lost` is only returned after a successful SIO CONNECT ACK
                // (see `run_connection`), so reaching this arm proves the
                // backend is reachable and the token is valid. Reset both
                // the backoff and the failure streak — and clear the
                // fresh-token retry counter so a long-lived session that
                // accumulated a RetryImmediately on a prior reconnect cycle
                // doesn't carry that dead state into the next one.
                if consecutive_failures > 0 {
                    log::debug!(
                        "[socket] Connection re-established; resetting failure streak ({} cleared)",
                        consecutive_failures
                    );
                }
                consecutive_failures = 0;
                fresh_token_retries = 0;
                log::warn!("[socket] Connection lost: {}", reason);
                backoff = Duration::from_millis(1000);
            }
            ConnectionOutcome::Failed(reason) if is_invalid_token_error(&reason) => {
                // The server rejected our token explicitly. Try one more
                // provider call — in case the token was refreshed on disk
                // since we fetched it moments ago (e.g. another code path
                // rotated it). If the provider returns a genuinely different
                // token, we can give it one more shot without consuming the
                // normal backoff budget.
                log::warn!(
                    "[socket] Invalid token on attempt — checking for fresh token (current_len={})",
                    token.len()
                );
                match decide_after_invalid_token(&token, &token_provider) {
                    InvalidTokenAction::RetryImmediately { token: fresh } => {
                        let fresh_len = fresh.len();
                        fresh_token_retries = fresh_token_retries.saturating_add(1);
                        if fresh_token_retries > 1 {
                            // We already gave the fresh-token cycle one
                            // immediate shot. A provider that keeps returning
                            // *different* non-empty tokens (rapid server-side
                            // rotation, non-deterministic source, buggy impl)
                            // could otherwise hot-loop forever with no sleep
                            // and no escalation — arguably worse than the
                            // 5-retry storm this PR was originally fixing. Fall
                            // through to the normal failure path so backoff
                            // sleeps and `consecutive_failures` escalation
                            // converge on a definitive outcome.
                            log::warn!(
                                "[socket] Fresh token available (len={fresh_len}) but already \
                                 retried once this cycle — escalating to normal backoff path"
                            );
                            consecutive_failures = consecutive_failures.saturating_add(1);
                            log_connection_failure(consecutive_failures, &reason);
                            // Fall through to the backoff sleep below.
                            // Intentionally drop `fresh` here: the bounded
                            // path now demands a backoff sleep, after which
                            // the next loop iteration will re-fetch the
                            // provider afresh (the token we just got may
                            // itself be stale by the time the sleep
                            // completes — this is the only knowable-correct
                            // policy for a rotating-source provider).
                        } else {
                            // We have a genuinely different token — try once
                            // immediately (no backoff sleep) with the **exact**
                            // token the decision step validated. Stash it for
                            // the next iteration so the top-of-loop provider
                            // call is skipped and we don't re-acquire the
                            // session-store lock or re-read from disk just to
                            // get the same value back. If this attempt also
                            // fails we will go through the normal escalation
                            // path on the next loop iteration (either
                            // same-token Escalate or the bounded fall-through
                            // above).
                            log::info!(
                                "[socket] Fresh token available (len={fresh_len}), retrying immediately"
                            );
                            pending_token = Some(fresh);
                            // Don't increment consecutive_failures for an attempt we
                            // couldn't have avoided — the token we used was already
                            // stale at fetch time.
                            continue;
                        }
                    }
                    InvalidTokenAction::Escalate { reason } => {
                        // No fresh token — the session is definitively expired.
                        // Escalate immediately instead of wasting more attempts
                        // on what is provably a dead token. This is the core fix
                        // for TAURI-RUST-9C (#2892).
                        log::warn!("[socket] Session expired ({reason}) — stopping reconnect loop");
                        *shared.error.write() =
                            Some("session expired — please sign in again".to_string());
                        *shared.status.write() = ConnectionStatus::Disconnected;
                        *shared.socket_id.write() = None;
                        emit_state_change(&shared);
                        return;
                    }
                }
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
// Invalid-token decision helper
// ---------------------------------------------------------------------------

/// Action the reconnect loop should take after receiving an "Invalid token"
/// rejection from the server.
enum InvalidTokenAction {
    /// A genuinely different token is available — retry the connection
    /// immediately (no backoff sleep). The fresh token is carried forward so
    /// the next `run_connection` uses **exactly** the validated value, not
    /// whatever a subsequent re-read of the provider returns — avoiding a
    /// redundant lock + disk I/O and the case where the logged `fresh_len`
    /// drifts from the token actually sent over the wire.
    RetryImmediately { token: String },
    /// No fresh token is available; the session is definitively expired.
    /// `reason` is a short diagnostic string for the log line.
    Escalate { reason: String },
}

/// Pure decision function: given the token that was just rejected and the
/// provider that may have a fresher one, decide what the reconnect loop
/// should do next.
///
/// Calls `provider()` exactly once. No socket I/O.
///
/// - Provider returns a **different, non-empty** token → `RetryImmediately`
///   carrying the fresh token for the caller to reuse on the next attempt.
/// - Provider returns the **same** token → `Escalate` (no point retrying).
/// - Provider returns an **empty** token → `Escalate` (treat as no session).
/// - Provider returns `Err` → `Escalate` with the provider error as reason.
fn decide_after_invalid_token(
    previous_token: &str,
    provider: &TokenProvider,
) -> InvalidTokenAction {
    match provider() {
        Ok(fresh) if !fresh.trim().is_empty() && fresh != previous_token => {
            InvalidTokenAction::RetryImmediately { token: fresh }
        }
        Ok(same) if same == previous_token => InvalidTokenAction::Escalate {
            reason: "token unchanged after provider re-fetch".to_string(),
        },
        Ok(_) => InvalidTokenAction::Escalate {
            reason: "provider returned empty token".to_string(),
        },
        Err(e) => InvalidTokenAction::Escalate {
            reason: format!("provider error: {e}"),
        },
    }
}

// ---------------------------------------------------------------------------
// Emit-channel draining
// ---------------------------------------------------------------------------

/// Discard whatever is still queued on the emit channel, reporting how much.
///
/// Called when a connection ends. The channel outlives the socket — it is
/// created once per `SocketManager::connect` and re-used by every reconnect
/// attempt — so anything left in it would otherwise be delivered on the next
/// connection, where nothing is waiting for it.
///
/// A free function rather than an inline loop so the behaviour is directly
/// testable without standing up a socket.
fn drain_pending_emits(rx: &mut mpsc::UnboundedReceiver<String>) -> usize {
    let mut dropped = 0usize;
    while rx.try_recv().is_ok() {
        dropped += 1;
    }
    dropped
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
    emit_ready: &Mutex<bool>,
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
    if let Err(e) = ws_write.send(WsMessage::Text(connect_msg.into())).await {
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

    // 6. Update state to Connected and mark this connection ready to emit.
    // The readiness flag is the emit gate (see `SocketManager::emit`): only now,
    // past a completed Socket.IO CONNECT ACK, is a queued message guaranteed to
    // ride *this* live socket rather than be dropped by `drain_pending_emits`.
    // A later server `error` EVENT flips `status` to `Error` for the UI but does
    // not return from this function, so the socket stays live and this flag
    // stays set — emits keep flowing until the connection is actually torn down.
    *shared.status.write() = ConnectionStatus::Connected;
    *shared.socket_id.write() = sio_sid;
    *emit_ready.lock() = true;
    emit_state_change(shared);

    // 7. Main event loop
    // Deadline = pingInterval + pingTimeout + 5 s grace so minor server-side
    // jitter doesn't cause a spurious reconnect on a healthy connection.
    let timeout_ms = ping_interval + ping_timeout_ms + 5_000;
    let timeout_duration = Duration::from_millis(timeout_ms);
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
                        if let Err(e) = ws_write.send(WsMessage::Text(msg.into())).await {
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
                    "[socket] No server ping received within {}ms (interval={}ms + timeout={}ms + 5s grace); reconnecting",
                    timeout_ms,
                    ping_interval,
                    ping_timeout_ms,
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
                    utf8_safe_prefix_at_byte_boundary(s, 40)
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
                    utf8_safe_prefix_at_byte_boundary(s, 40)
                );
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(format!("WS error during SIO handshake: {e}")),
            None => return Err("WebSocket closed before CONNECT ACK".into()),
        }
    }
}
