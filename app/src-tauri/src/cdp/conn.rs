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
    /// Long-lived consumers (e.g. the Discord scanner) do not yet have an
    /// idle watchdog to time out a stalled/destroyed page target and force
    /// a re-attach — that self-healing path is tracked as a fast-follow in
    /// #3693.
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

    /// Diagnostic helper — webview label this connection is bound to.
    pub fn label(&self) -> &str {
        &self.label
    }
}
