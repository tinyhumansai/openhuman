//! [`CdpConn`] — per-attach handle on top of the in-process CDP transport.
//!
//! Wraps an [`Arc<WebviewCdpTransport>`](super::in_process::WebviewCdpTransport)
//! with the same `call` / `pump_events` surface scanners and the per-account
//! session opener use. All attaches for a given webview share the same
//! in-process channel, and a [`CdpConn`] is just a cheap session-scoped
//! view.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc};

use super::in_process::{EventFrame, WebviewCdpTransport};

/// Per-attach CDP handle. Wraps an `Arc<WebviewCdpTransport>` and
/// filters incoming events by `session_id` so concurrent attachers
/// don't see each other's events.
pub struct CdpConn {
    transport: Arc<WebviewCdpTransport>,
    label: String,
}

impl CdpConn {
    /// Wrap an already-installed in-process transport. Callers obtain
    /// the transport from the per-app [`super::CdpRegistry`]
    /// (`app.state()`) — typically via [`super::conn_for_account`] or
    /// [`super::conn_for_label`].
    pub fn new(transport: Arc<WebviewCdpTransport>) -> Self {
        let label = transport.label().to_string();
        Self { transport, label }
    }

    /// Setup-phase request/response: sends a JSON-RPC call and awaits
    /// the matching response. `session_id`, when supplied, is inlined
    /// into the envelope so the call routes to a previously-attached
    /// child target (via `Target.attachToTarget`).
    pub async fn call(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        self.transport.call(method, params, session_id).await
    }

    /// Same as [`call`](Self::call) but with a caller-supplied response
    /// timeout. Slack's IDB batch serialisation can run past the default
    /// 35s; other callers should stick with `call`.
    pub async fn call_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.transport
            .call_with_timeout(method, params, session_id, timeout)
            .await
    }

    /// Subscribe to the transport's event stream and dispatch every
    /// inbound CDP event via the supplied callback until the channel
    /// signals it cannot keep up.
    ///
    /// `session_id` filters incoming events — CDP multiplexes all
    /// sessions through the same transport when `flatten: true` is set,
    /// so we drop events belonging to other sessions.
    ///
    /// Returns when the channel closes (the transport has been
    /// forgotten) or on an unrecoverable error. `Lagged` is logged and
    /// treated as a continuation signal: the pump keeps draining rather
    /// than tearing down the session, so a burst that overflows
    /// `EVENT_CHANNEL_CAP` drops the skipped frames without re-syncing.
    /// This plain pump has no idle watchdog, so it only returns once the
    /// whole transport is dropped — a stale/destroyed page target leaves it
    /// awaiting forever. Long-lived consumers that must self-heal from a
    /// dead page target and never drop frames should use
    /// [`pump_events_resilient`](Self::pump_events_resilient) instead.
    pub async fn pump_events<F>(&mut self, session_id: &str, mut on_event: F) -> Result<(), String>
    where
        F: FnMut(&str, &Value),
    {
        let mut rx = self.transport.subscribe_events();
        loop {
            match rx.recv().await {
                Ok(EventFrame {
                    method,
                    params,
                    session_id: evt_session,
                }) => {
                    if !evt_session.is_empty() && evt_session != session_id {
                        continue;
                    }
                    on_event(&method, &params);
                }
                Err(RecvError::Lagged(skipped)) => {
                    log::warn!(
                        "[cdp][{}] event channel lagged skipped={} session_id={}",
                        self.label,
                        skipped,
                        session_id
                    );
                    continue;
                }
                Err(RecvError::Closed) => return Ok(()),
            }
        }
    }

    /// Like [`pump_events`](Self::pump_events) but adds the two robustness
    /// properties a long-lived consumer (the Discord scanner) needs:
    ///
    ///  * **Idle watchdog** — returns `Ok(())` once `idle_timeout` elapses
    ///    with no inbound frame. A live session emits frames well within that
    ///    window (Discord's gateway heartbeats roughly every 41s), so a longer
    ///    silence means the page target is stale/destroyed (reload, renderer
    ///    crash, hard navigation) — the plain pump would await it forever.
    ///    Returning lets the caller's outer loop re-attach. `idle_timeout`
    ///    MUST be larger than the consumer's heartbeat cadence.
    ///  * **Non-lossy delivery** — a background task drains frames off the
    ///    fixed-capacity broadcast ring into an unbounded queue, so a burst
    ///    that overflows `EVENT_CHANNEL_CAP` is buffered instead of silently
    ///    dropped (the slow per-frame work runs on the consumer side, not on
    ///    the broadcast receiver).
    ///
    /// Like `pump_events`, returns `Ok(())` when the transport closes.
    pub async fn pump_events_resilient<F>(
        &mut self,
        session_id: &str,
        idle_timeout: Duration,
        on_event: F,
    ) -> Result<(), String>
    where
        F: FnMut(&str, &Value),
    {
        pump_resilient_core(
            self.transport.subscribe_events(),
            session_id,
            idle_timeout,
            &self.label,
            on_event,
        )
        .await
    }

    /// Diagnostic helper — webview label this connection is bound to.
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// Core of [`CdpConn::pump_events_resilient`], split out over raw primitives
/// (a [`broadcast::Receiver`] plus the policy params) so it can be unit-tested
/// without constructing a full [`WebviewCdpTransport`].
///
/// A tiny drain task forwards every session-matched frame from the fixed-size
/// broadcast ring into an unbounded queue, decoupling the slow per-frame
/// `on_event` work from the broadcast consumer. The main loop reads that queue
/// under an idle-timeout watchdog. Returns `Ok(())` when the transport closes
/// (drain task ends) or when `idle_timeout` elapses with no frame.
async fn pump_resilient_core<F>(
    mut rx: broadcast::Receiver<EventFrame>,
    session_id: &str,
    idle_timeout: Duration,
    label: &str,
    mut on_event: F,
) -> Result<(), String>
where
    F: FnMut(&str, &Value),
{
    let (tx, mut rx_u) = mpsc::unbounded_channel::<EventFrame>();
    let session_owned = session_id.to_string();
    let drain_label = label.to_string();
    let drain = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(frame) => {
                    // Session filter only — content filtering stays in the
                    // caller's `on_event`, so the watchdog still observes
                    // liveness traffic (e.g. gateway heartbeat WS frames) on
                    // any session we're attached to.
                    if !frame.session_id.is_empty() && frame.session_id != session_owned {
                        continue;
                    }
                    if tx.send(frame).is_err() {
                        break; // consumer dropped — nothing left to drain into
                    }
                }
                Err(RecvError::Lagged(skipped)) => {
                    // The drain does ~zero work per frame, so it practically
                    // never falls behind; log if it ever does.
                    log::warn!(
                        "[cdp][{}] drain lagged skipped={} session_id={}",
                        drain_label,
                        skipped,
                        session_owned
                    );
                    continue;
                }
                Err(RecvError::Closed) => break,
            }
        }
    });

    let result = loop {
        match tokio::time::timeout(idle_timeout, rx_u.recv()).await {
            Ok(Some(frame)) => on_event(&frame.method, &frame.params),
            Ok(None) => break Ok(()), // transport closed / webview forgotten
            Err(_elapsed) => {
                log::info!(
                    "[cdp][{}] event pump idle for {:?}, forcing re-attach session_id={}",
                    label,
                    idle_timeout,
                    session_id
                );
                break Ok(());
            }
        }
    };
    drain.abort();
    result
}

#[cfg(test)]
mod resilient_pump_tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn frame(method: &str, session: &str) -> EventFrame {
        EventFrame {
            method: method.to_string(),
            params: serde_json::json!({}),
            session_id: session.to_string(),
        }
    }

    /// No frames within the idle window → the watchdog returns `Ok(())` so the
    /// caller can re-attach, instead of awaiting a dead session forever.
    #[tokio::test(start_paused = true)]
    async fn returns_on_idle_timeout() {
        let (tx, rx) = broadcast::channel::<EventFrame>(8);
        let calls = Rc::new(RefCell::new(0usize));
        let c = calls.clone();
        let res = pump_resilient_core(rx, "sess", Duration::from_secs(30), "test", |_m, _p| {
            *c.borrow_mut() += 1;
        })
        .await;
        assert!(res.is_ok());
        assert_eq!(*calls.borrow(), 0);
        drop(tx); // sender kept alive across the run so the idle path is exercised
    }

    /// When the transport closes, frames already buffered are flushed to the
    /// consumer before the pump returns.
    #[tokio::test(start_paused = true)]
    async fn flushes_buffered_then_returns_on_close() {
        let (tx, rx) = broadcast::channel::<EventFrame>(64);
        for i in 0..10 {
            tx.send(frame(&format!("m{i}"), "sess")).unwrap();
        }
        drop(tx); // close after queuing
        let got = Rc::new(RefCell::new(Vec::new()));
        let g = got.clone();
        let res = pump_resilient_core(rx, "sess", Duration::from_secs(30), "test", |m, _p| {
            g.borrow_mut().push(m.to_string());
        })
        .await;
        assert!(res.is_ok());
        assert_eq!(got.borrow().len(), 10);
    }

    /// A burst far larger than `EVENT_CHANNEL_CAP` (here a tiny ring of 4) is
    /// delivered without loss: the drain task forwards each frame into the
    /// unbounded queue faster than it can overflow the ring.
    #[tokio::test(start_paused = true)]
    async fn is_non_lossy_beyond_broadcast_cap() {
        let (tx, rx) = broadcast::channel::<EventFrame>(4);
        let got = Rc::new(RefCell::new(0usize));
        let g = got.clone();
        let producer = tokio::spawn(async move {
            for i in 0..200 {
                let _ = tx.send(frame(&format!("m{i}"), "sess"));
                // Yield so the drain task pulls each frame before the next send
                // can overflow the 4-slot ring.
                tokio::task::yield_now().await;
            }
            // tx dropped here → channel closes once the burst is drained
        });
        let res = pump_resilient_core(rx, "sess", Duration::from_secs(3600), "test", |_m, _p| {
            *g.borrow_mut() += 1;
        })
        .await;
        producer.await.unwrap();
        assert!(res.is_ok());
        assert_eq!(*got.borrow(), 200);
    }

    /// Frames addressed to another session are dropped; the empty session id
    /// (broadcast-to-all) and our own session id are delivered.
    #[tokio::test(start_paused = true)]
    async fn filters_by_session() {
        let (tx, rx) = broadcast::channel::<EventFrame>(64);
        tx.send(frame("a", "sess")).unwrap();
        tx.send(frame("b", "other")).unwrap();
        tx.send(frame("c", "")).unwrap();
        tx.send(frame("d", "sess")).unwrap();
        drop(tx);
        let got = Rc::new(RefCell::new(Vec::new()));
        let g = got.clone();
        let res = pump_resilient_core(rx, "sess", Duration::from_secs(30), "test", |m, _p| {
            g.borrow_mut().push(m.to_string());
        })
        .await;
        assert!(res.is_ok());
        assert_eq!(*got.borrow(), vec!["a", "c", "d"]);
    }
}
