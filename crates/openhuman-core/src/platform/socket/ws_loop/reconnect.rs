//! The reconnect loop: [`ws_loop`], its failure-escalation logging, the
//! invalid-token retry decision, and the emit-queue drain used on shutdown.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use parking_lot::Mutex;

use tokio::sync::{mpsc, watch};
use tokio::time::{Duration, Instant};

use crate::platform::socket::models::ConnectionStatus;

use super::connect::{run_connection, ReconnectContext};
use crate::platform::socket::manager::{emit_state_change, SharedState};
use crate::platform::socket::token_provider::{is_invalid_token_error, TokenProvider};
use crate::platform::socket::types::ConnectionOutcome;

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
pub(super) const FAIL_ESCALATE_THRESHOLD: u32 = 5;

/// Clears `SharedState::loop_active` when the loop task ends, whatever the
/// exit path — a `return`, a `break`, or an abort from `terminate_loop`
/// (dropping the future drops this guard too).
struct LoopActiveGuard<'a>(&'a SharedState);

impl Drop for LoopActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.loop_active.store(false, Ordering::Release);
    }
}

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
pub(crate) async fn ws_loop(
    url: String,
    token_provider: TokenProvider,
    shared: Arc<SharedState>,
    mut emit_rx: mpsc::UnboundedReceiver<String>,
    mut shutdown_rx: watch::Receiver<bool>,
    internal_tx: mpsc::UnboundedSender<String>,
    emit_ready: Arc<Mutex<bool>>,
) {
    // `spawn_loop` raises the flag before the task starts; a direct caller
    // (tests) gets the same contract from here. The guard lowers it on exit.
    shared.loop_active.store(true, Ordering::Release);
    shared
        .loop_stopped_on_failure
        .store(false, Ordering::Release);
    let _loop_active = LoopActiveGuard(&shared);

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
    let mut ws_url = crate::platform::socket::url::websocket_url(&url);
    // What the next attempt is recovering from, so the handshake can log how
    // long the socket was down and how many attempts it took (#6256).
    let mut reconnect = ReconnectContext::default();

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
                    shared
                        .loop_stopped_on_failure
                        .store(true, Ordering::Release);
                    *shared.error.write() =
                        Some("session expired — please sign in again".to_string());
                    *shared.status.write() = ConnectionStatus::Disconnected;
                    *shared.socket_id.write() = None;
                    emit_state_change(&shared);
                    return;
                }
                Err(e) => {
                    log::warn!("[socket] ws_loop: token provider failed — stopping: {e}");
                    shared
                        .loop_stopped_on_failure
                        .store(true, Ordering::Release);
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
        shared.set_connection_snapshot(
            Some((url.clone(), token.clone())),
            ConnectionStatus::Connecting,
        );
        emit_state_change(&shared);

        // Taken before the attempt so a stalled first dial counts toward the
        // outage it starts: `Failed` lands only after the connect deadline has
        // run out, and dating the outage from then would hide that whole wait
        // from the `Connected after …` line (Codex review on #6270).
        let attempt_started = Instant::now();
        let outcome = run_connection(
            &mut ws_url,
            &token,
            &shared,
            &mut emit_rx,
            &mut shutdown_rx,
            &internal_tx,
            &emit_ready,
            reconnect,
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

        match &outcome {
            ConnectionOutcome::Lost(_) => {
                reconnect = ReconnectContext {
                    outage_started: Some(Instant::now()),
                    lost_previous: true,
                    failed_attempts: 0,
                    escalated: false,
                };
            }
            ConnectionOutcome::Failed(_) => {
                if reconnect.outage_started.is_none() {
                    reconnect.outage_started = Some(attempt_started);
                }
                reconnect.failed_attempts = reconnect.failed_attempts.saturating_add(1);
            }
            ConnectionOutcome::Shutdown => {}
        }

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
                            reconnect.escalated |= log_connection_failure(
                                consecutive_failures,
                                reconnect.outage_started.map(|t| t.elapsed()),
                                &reason,
                            );
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
                        shared
                            .loop_stopped_on_failure
                            .store(true, Ordering::Release);
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
                reconnect.escalated |= log_connection_failure(
                    consecutive_failures,
                    reconnect.outage_started.map(|t| t.elapsed()),
                    &reason,
                );
                // keep growing backoff
            }
        }

        // Between attempts the loop is alive and will retry, so report
        // `Reconnecting` rather than `Disconnected`: `Disconnected` is what a
        // stopped loop reports (signed out, session expired, shutdown), and
        // the frontend's connectivity chip has to tell the two apart — a link
        // that is down but being retried is an outage worth showing, a loop
        // that was never started is not (#6256).
        *shared.status.write() = ConnectionStatus::Reconnecting;
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
/// Returns whether this call fired the one-shot escalation, so the caller can
/// remember that the outage paged and report its duration on recovery (#6417).
///
/// Extracted as a pure function so it can be unit-tested without running an
/// async event loop or touching the WS stack.
pub(super) fn log_connection_failure(
    consecutive: u32,
    outage: Option<Duration>,
    reason: &str,
) -> bool {
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
        // Report the outage as "paged" only if the classifier actually let the
        // event through. An offline user (`Network is unreachable`, airplane
        // mode — OPENHUMAN-TAURI-BH) is demoted to a warn breadcrumb here, and
        // claiming otherwise would make the recovery report in
        // `run_connection` fire a Sentry event on the way back up — the same
        // noise, moved one step later. Pairing on the *reported* escalation
        // keeps it at exactly two events for a genuine outage and zero for a
        // user who simply closed their laptop lid.
        crate::core::observability::expected_error_kind(&detailed).is_none()
    } else {
        // Below threshold (transient blips) or above threshold (already fired
        // the one-shot error): stay at `warn` so subsequent retries don't pile
        // up additional Sentry events.
        log::warn!("{}", render_attempt_line(consecutive, outage, reason));
        false
    }
}

/// Render the `warn`-level line for a failed attempt.
///
/// `FAIL_ESCALATE_THRESHOLD` is the point at which the loop *pages*, not a cap
/// on retries — the loop retries forever by design. Printing it as the
/// denominator of every attempt is what produced `attempt 32/5` in #6417: a
/// counter apparently past its own maximum, which reads as a bug in the loop
/// rather than the intended behaviour.
///
/// So the denominator is only shown while it is still true. Past the threshold
/// the line drops it and reports what actually matters during a long outage —
/// how long the socket has been down — which is also what the reporter asked
/// for. Duration is in seconds to match the `Connected after …` line that
/// closes the outage out in `run_connection`.
///
/// A pure `String` so the wording is directly assertable without a tracing
/// capture layer.
pub(super) fn render_attempt_line(
    consecutive: u32,
    outage: Option<Duration>,
    reason: &str,
) -> String {
    if consecutive <= FAIL_ESCALATE_THRESHOLD {
        format!("[socket] Connection failed (attempt {consecutive}/{FAIL_ESCALATE_THRESHOLD}): {reason}")
    } else {
        match outage {
            Some(down) => format!(
                "[socket] Connection failed (attempt {consecutive}, still retrying after {:.0}s down): {reason}",
                down.as_secs_f64()
            ),
            None => format!(
                "[socket] Connection failed (attempt {consecutive}, still retrying): {reason}"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Invalid-token decision helper
// ---------------------------------------------------------------------------

/// Action the reconnect loop should take after receiving an "Invalid token"
/// rejection from the server.
pub(super) enum InvalidTokenAction {
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
pub(super) fn decide_after_invalid_token(
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
pub(super) fn drain_pending_emits(rx: &mut mpsc::UnboundedReceiver<String>) -> usize {
    let mut dropped = 0usize;
    while rx.try_recv().is_ok() {
        dropped += 1;
    }
    dropped
}

// ---------------------------------------------------------------------------
// Single connection attempt
// ---------------------------------------------------------------------------
