//! In-process Chrome DevTools Protocol transport built on
//! [`tauri::Webview::send_dev_tools_message`] +
//! [`tauri::Webview::on_dev_tools_protocol`].
//!
//! Replaces the legacy WebSocket-to-loopback transport that required
//! Chromium to be spawned with `--remote-debugging-port=19222`. The old
//! transport opened an unauthenticated TCP listener that any same-UID
//! local process could attach to. The in-process path stays entirely
//! within our process boundary — there is no listener for an external
//! attacker to reach.
//!
//! # Architecture
//!
//! - One [`WebviewCdpTransport`] per CEF webview. Installed at the
//!   moment a webview is created by `webview_accounts` (and similar
//!   creation sites). The webview-creator calls
//!   [`install_for_webview`] with its concrete `Webview<tauri::Cef>`
//!   handle.
//! - The bootstrapped transport is registered in a process-global
//!   [`CdpRegistry`] (`app.state::<CdpRegistry>()`) keyed by account id
//!   so scanners that are generic over `Runtime: tauri::Runtime` never
//!   need to obtain a typed `Webview<Cef>` themselves.
//! - The transport registers a single
//!   [`tauri::Webview::on_dev_tools_protocol`] callback at install
//!   time. The CEF runtime keeps the callback alive for the webview's
//!   whole lifetime; there is no un-register path, so the install must
//!   be a one-shot keyed by webview label.
//! - Every outbound request gets a unique numeric `id`. A pending-map
//!   ([`PendingMap`]) routes responses back to the awaiting
//!   `tokio::sync::oneshot`.
//! - Events (CDP frames with no `id`) are fanned out through a
//!   `tokio::sync::broadcast` channel. Each subscriber sees its own
//!   queue.
//!
//! # Wire format
//!
//! Tauri-CEF delivers each inbound DevTools message in three variants
//! (`CefDevToolsProtocol::Message` raw JSON, `MethodResult` pre-parsed
//! response, `Event` pre-parsed event). We listen on the raw `Message`
//! variant only — it carries the full envelope including `sessionId`,
//! which the other two strip.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{broadcast, oneshot};

use tauri::{AppHandle, Manager, Webview};

/// Timeout for a single request/response round-trip. Long enough for cold
/// attach on slow machines but short enough to fail fast on a stuck channel.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(35);

/// Capacity of the per-transport event broadcast channel. CDP can produce
/// bursts (e.g. on `Page.enable` the initial frame-history dump). 256 keeps
/// memory bounded while absorbing typical burst sizes.
const EVENT_CHANNEL_CAP: usize = 256;

type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

/// One CDP frame delivered as an event (no `id` in the envelope).
#[derive(Clone, Debug)]
pub struct EventFrame {
    pub method: String,
    pub params: Value,
    pub session_id: String,
}

/// Per-webview CDP transport. Holds the pending-id table and event
/// broadcaster. Constructed by [`install_for_webview`] and (typically)
/// inserted into the per-app [`CdpRegistry`] for later lookup.
pub struct WebviewCdpTransport {
    label: String,
    webview: Webview<tauri::Cef>,
    next_id: Mutex<i64>,
    pending: PendingMap,
    events_tx: broadcast::Sender<EventFrame>,
}

impl WebviewCdpTransport {
    /// Submit a CDP request and await its response.
    ///
    /// The request `id` is auto-assigned. `session_id`, when supplied, is
    /// inlined into the envelope so the call routes to a previously-attached
    /// child target.
    pub async fn call(
        self: &Arc<Self>,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        self.call_with_timeout(method, params, session_id, CALL_TIMEOUT)
            .await
    }

    /// Same as [`call`](Self::call) but with a caller-supplied response
    /// timeout. Slack's IDB serialise batches can legitimately exceed the
    /// default 35s budget; the canonical 60s timeout in that module flows
    /// through here without forcing every other caller to opt into a
    /// longer wait.
    pub async fn call_with_timeout(
        self: &Arc<Self>,
        method: &str,
        params: Value,
        session_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = {
            let mut n = self.next_id.lock().expect("next_id mutex poisoned");
            let id = *n;
            *n += 1;
            id
        };
        let (tx, rx) = oneshot::channel();
        {
            let mut p = self.pending.lock().expect("pending mutex poisoned");
            p.insert(id, tx);
        }

        let mut req = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session_id {
            req["sessionId"] = json!(s);
        }
        let body = serde_json::to_vec(&req).map_err(|e| format!("encode: {e}"))?;

        // `send_dev_tools_message` blocks on a `std::sync::mpsc::Receiver`
        // while the message is dispatched onto the CEF main thread. Off-load
        // to a blocking pool so we don't park a tokio worker thread.
        log::trace!(
            "[cdp][{}] >> id={} method={} session_id={:?}",
            self.label,
            id,
            method,
            session_id
        );
        let webview = self.webview.clone();
        let send_res = tauri::async_runtime::spawn_blocking(move || {
            webview
                .send_dev_tools_message(&body)
                .map_err(|e| format!("send_dev_tools_message: {e}"))
        })
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?;
        if let Err(e) = send_res {
            // Clean up the pending entry; otherwise it leaks until next call.
            let mut p = self.pending.lock().expect("pending mutex poisoned");
            p.remove(&id);
            return Err(e);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => {
                // oneshot dropped — transport torn down between dispatch and
                // receive. Surface as a transport error rather than panic.
                Err(format!("cdp response channel dropped (method={method})"))
            }
            Err(_) => {
                // Timeout. Make a best-effort to evict the pending entry so
                // a delayed response doesn't pollute the next caller.
                let mut p = self.pending.lock().expect("pending mutex poisoned");
                p.remove(&id);
                Err(format!("cdp call timeout (method={method})"))
            }
        }
    }

    /// Subscribe to the event stream for this transport. Each subscriber
    /// gets its own queue; lagged subscribers see `RecvError::Lagged` and
    /// must re-sync.
    pub fn subscribe_events(self: &Arc<Self>) -> broadcast::Receiver<EventFrame> {
        self.events_tx.subscribe()
    }

    /// Webview label associated with this transport — for diagnostics only.
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// Process-global registry of installed CDP transports keyed by webview
/// label. Stored via `app.manage(CdpRegistry::default())` at boot so
/// scanners and the per-account session opener can look up the correct
/// transport without holding a typed `Webview<Cef>` themselves.
#[derive(Default)]
pub struct CdpRegistry {
    transports: Mutex<HashMap<String, Arc<WebviewCdpTransport>>>,
}

impl CdpRegistry {
    /// Look up an installed transport by webview label. Returns `None`
    /// when the webview has not been created yet (cold boot before the
    /// account is opened, or after it was forgotten).
    pub fn by_label(&self, label: &str) -> Option<Arc<WebviewCdpTransport>> {
        self.transports
            .lock()
            .expect("CdpRegistry mutex poisoned")
            .get(label)
            .cloned()
    }

    /// Look up the transport for an `acct_*`-labelled webview. The
    /// account id is the unsuffixed value passed to
    /// [`webview_accounts::label_for`] (typically already sanitized).
    pub fn by_account(&self, account_id: &str) -> Option<Arc<WebviewCdpTransport>> {
        self.by_label(&format!("acct_{account_id}"))
    }

    /// Atomic "get or create" for a transport keyed by webview label.
    ///
    /// Holds the registry mutex across both the existence check AND the
    /// caller-supplied creator, so two concurrent
    /// [`install_for_webview`] callers can never both register a
    /// `on_dev_tools_protocol` observer on the same webview. The CEF
    /// runtime offers no un-register hook, so a double-registration is
    /// permanent — every inbound frame would fan out to both observer
    /// closures, splitting `id`-keyed responses between two
    /// disconnected `next_id` counters and breaking response routing.
    ///
    /// The creator closure is responsible for constructing the transport
    /// (which includes registering the CEF observer). If the closure
    /// returns `Err`, the registry is unchanged.
    fn get_or_create<F>(&self, label: &str, create: F) -> Result<Arc<WebviewCdpTransport>, String>
    where
        F: FnOnce() -> Result<Arc<WebviewCdpTransport>, String>,
    {
        let mut t = self.transports.lock().expect("CdpRegistry mutex poisoned");
        if let Some(existing) = t.get(label) {
            return Ok(Arc::clone(existing));
        }
        let transport = create()?;
        t.insert(label.to_string(), Arc::clone(&transport));
        Ok(transport)
    }

    /// Remove a transport from the registry by label. Called when a
    /// webview is closed / forgotten so subsequent
    /// [`Self::by_label`] / [`Self::by_account`] lookups return `None`
    /// instead of a stale entry.
    pub fn forget_label(&self, label: &str) {
        self.transports
            .lock()
            .expect("CdpRegistry mutex poisoned")
            .remove(label);
    }

    /// Convenience wrapper around [`Self::forget_label`] for the
    /// account-suffixed label scheme.
    pub fn forget_account(&self, account_id: &str) {
        self.forget_label(&format!("acct_{account_id}"));
    }
}

/// Process-global typed CEF [`AppHandle`]. Populated exactly once from
/// the `Builder::<tauri::Cef>::setup` callback in `lib.rs`. Used by
/// [`install_for_account`] to look up a webview by label with the
/// concrete `Webview<Cef>` typed handle that
/// [`Webview::send_dev_tools_message`] requires — avoids a
/// runtime-generic transmute at the install site.
///
/// `OnceLock` is appropriate because the cell is written exactly once,
/// during `Builder::setup`, before any webview is created.
static CEF_APP_HANDLE: OnceLock<AppHandle<tauri::Cef>> = OnceLock::new();

/// Record the typed CEF [`AppHandle`] in the process-global cell. Called
/// once from the `Builder::setup` callback in `lib.rs`. Subsequent calls
/// are silently ignored (`OnceLock::set` returns `Err` on the second
/// write) so re-entry during hot-reload doesn't panic.
pub fn set_cef_app_handle(app: AppHandle<tauri::Cef>) {
    let _ = CEF_APP_HANDLE.set(app);
}

/// Install (or look up) the in-process CDP transport for an
/// account-keyed webview. The account id is the same value the
/// `webview_accounts::label_for` helper composes a label from
/// (`acct_{id}`). Idempotent — repeated calls for the same account
/// return the cached transport.
///
/// Returns `Err` when the webview hasn't been created yet (the typical
/// cold-boot race against a scanner that started before
/// `webview_accounts::open` finished). Callers back off and retry.
pub fn install_for_account(account_id: &str) -> Result<Arc<WebviewCdpTransport>, String> {
    install_for_label(&format!("acct_{account_id}"))
}

/// Install (or look up) the in-process CDP transport for any
/// CEF-backed webview, keyed by its concrete label. Generic counterpart
/// of [`install_for_account`] for webviews that aren't account scanners
/// — e.g. Meet call windows labelled `meet-call-{request_id}`.
///
/// Returns `Err` when the webview hasn't been created yet (cold-boot
/// race). Callers back off and retry.
pub fn install_for_label(label: &str) -> Result<Arc<WebviewCdpTransport>, String> {
    let app = CEF_APP_HANDLE
        .get()
        .ok_or_else(|| "cdp::set_cef_app_handle has not been called yet".to_string())?;
    let registry_state = app
        .try_state::<CdpRegistry>()
        .ok_or_else(|| "CdpRegistry not managed by app".to_string())?;
    let registry = registry_state.inner();
    if let Some(existing) = registry.by_label(label) {
        return Ok(existing);
    }
    let webview = app
        .get_webview(label)
        .ok_or_else(|| format!("no webview for label={label}"))?;
    install_for_webview(registry, webview)
}

/// Install a CDP transport on `webview`, register the observer, and
/// insert the resulting [`WebviewCdpTransport`] into `registry`.
///
/// Idempotent and concurrency-safe: the existence check, observer
/// registration, and registry insert all happen while the registry
/// mutex is held, so two concurrent callers for the same webview
/// label can never both attach an observer (CEF gives no un-register
/// hook — a double-registration would permanently split responses
/// between two `next_id` counters).
pub fn install_for_webview(
    registry: &CdpRegistry,
    webview: Webview<tauri::Cef>,
) -> Result<Arc<WebviewCdpTransport>, String> {
    let label = webview.label().to_string();
    registry.get_or_create(&label, || {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (events_tx, _rx0) = broadcast::channel::<EventFrame>(EVENT_CHANNEL_CAP);

        let transport = Arc::new(WebviewCdpTransport {
            label: label.clone(),
            webview: webview.clone(),
            next_id: Mutex::new(1),
            pending: Arc::clone(&pending),
            events_tx: events_tx.clone(),
        });

        let pending_for_observer = Arc::clone(&pending);
        let events_for_observer = events_tx.clone();
        let label_for_observer = label.clone();
        webview
            .on_dev_tools_protocol(move |protocol| {
                use tauri::CefDevToolsProtocol as P;
                // Listen only on the raw `Message` variant — it carries
                // the full envelope (including `sessionId`) which
                // `MethodResult` and `Event` strip. Tauri-CEF still
                // fires all three for the same wire message, so
                // consuming both would double-dispatch.
                if let P::Message(bytes) = protocol {
                    let v: Value = match serde_json::from_slice(&bytes) {
                        Ok(v) => v,
                        Err(e) => {
                            log::warn!(
                                "[cdp][{}] inbound: parse error: {} (bytes_len={})",
                                label_for_observer,
                                e,
                                bytes.len()
                            );
                            return;
                        }
                    };
                    handle_inbound(
                        &label_for_observer,
                        &v,
                        &pending_for_observer,
                        &events_for_observer,
                    );
                }
            })
            .map_err(|e| format!("on_dev_tools_protocol: {e}"))?;

        log::info!(
            "[cdp][{}] in-process transport installed (observer registered)",
            label
        );

        Ok(transport)
    })
}

fn handle_inbound(
    label: &str,
    v: &Value,
    pending: &PendingMap,
    events_tx: &broadcast::Sender<EventFrame>,
) {
    if let Some(id) = v.get("id").and_then(|x| x.as_i64()) {
        let waiter = {
            let mut p = pending.lock().expect("pending mutex poisoned");
            p.remove(&id)
        };
        match waiter {
            Some(tx) => {
                let res = if let Some(err) = v.get("error") {
                    Err(format!("cdp error: {err}"))
                } else {
                    Ok(v.get("result").cloned().unwrap_or(Value::Null))
                };
                let _ = tx.send(res);
            }
            None => {
                log::trace!(
                    "[cdp][{}] inbound: orphan response id={} (caller already gone)",
                    label,
                    id
                );
            }
        }
        return;
    }

    let Some(method) = v.get("method").and_then(|x| x.as_str()) else {
        return;
    };
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    let session_id = v
        .get("sessionId")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let frame = EventFrame {
        method: method.to_string(),
        params,
        session_id,
    };
    // `send` only errors when there are zero subscribers — fine to drop
    // events when nobody is listening.
    let _ = events_tx.send(frame);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Inbound response carrying a known `id` resolves the matching pending
    /// oneshot with the `result` payload.
    #[tokio::test]
    async fn inbound_response_resolves_pending() {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (events_tx, _) = broadcast::channel::<EventFrame>(8);

        let (tx, rx) = oneshot::channel();
        pending.lock().unwrap().insert(42, tx);

        handle_inbound(
            "test",
            &json!({ "id": 42, "result": { "ok": true } }),
            &pending,
            &events_tx,
        );

        let got = rx.await.expect("oneshot resolved").expect("ok response");
        assert_eq!(got, json!({ "ok": true }));
        assert!(
            pending.lock().unwrap().is_empty(),
            "pending entry must be evicted on resolve"
        );
    }

    /// Inbound error frame routes to the pending sender as `Err(_)`.
    #[tokio::test]
    async fn inbound_error_resolves_pending_as_err() {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (events_tx, _) = broadcast::channel::<EventFrame>(8);

        let (tx, rx) = oneshot::channel();
        pending.lock().unwrap().insert(7, tx);

        handle_inbound(
            "test",
            &json!({ "id": 7, "error": { "code": -32000, "message": "boom" } }),
            &pending,
            &events_tx,
        );

        let got = rx.await.expect("oneshot resolved");
        assert!(got.is_err(), "error frame must surface as Err");
    }

    /// Inbound event frame with no `id` fans out to subscribers.
    #[tokio::test]
    async fn inbound_event_broadcasts_to_subscribers() {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (events_tx, mut events_rx) = broadcast::channel::<EventFrame>(8);

        handle_inbound(
            "test",
            &json!({
                "method": "Page.loadEventFired",
                "sessionId": "abc",
                "params": { "timestamp": 1.23 },
            }),
            &pending,
            &events_tx,
        );

        let frame = events_rx.recv().await.expect("event received");
        assert_eq!(frame.method, "Page.loadEventFired");
        assert_eq!(frame.session_id, "abc");
        assert_eq!(frame.params, json!({ "timestamp": 1.23 }));
    }

    /// Orphan responses (no matching pending entry) drop silently.
    #[tokio::test]
    async fn inbound_orphan_response_drops_silently() {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (events_tx, _) = broadcast::channel::<EventFrame>(8);
        handle_inbound(
            "test",
            &json!({ "id": 999, "result": {} }),
            &pending,
            &events_tx,
        );
        assert!(pending.lock().unwrap().is_empty());
    }

    // `registry.forget_account` / `by_account` are exercised in the
    // integration test `cdp_in_process_e2e.rs` against a real Tauri-CEF
    // runtime — unit tests cannot synthesize a `Webview<Cef>` without a
    // running CEF instance, and the registry map itself is a trivial
    // `HashMap` so the wiring tests above already cover all branches.
}
