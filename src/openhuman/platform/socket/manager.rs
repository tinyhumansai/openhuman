//! SocketManager — persistent Rust-native Socket.IO connection via WebSocket.
//!
//! Implements Engine.IO v4 and Socket.IO v4 protocols directly over WebSocket
//! using `tokio-tungstenite` with `rustls` TLS.
//!
//! Responsibilities:
//! - MCP `listTools` / `toolCall` handled directly via the WorkflowRegistry
//! - Non-MCP server events forwarded to running skills and to the frontend
//! - Connection state logging for observability
//! - Automatic reconnection with exponential backoff

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, OnceLock,
};

use parking_lot::{Mutex, RwLock};
use serde_json::json;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Duration;

use crate::api::models::socket::{ConnectionStatus, SocketState};
use crate::openhuman::skills::webhooks::WebhookRouter;

use super::token_provider::{static_token_provider, TokenProvider};
use super::ws_loop::ws_loop;

// ---------------------------------------------------------------------------
// Global accessor
// ---------------------------------------------------------------------------

static GLOBAL_SOCKET_MANAGER: OnceLock<Arc<SocketManager>> = OnceLock::new();

/// Register the global `SocketManager` instance (called once during bootstrap).
pub fn set_global_socket_manager(mgr: Arc<SocketManager>) {
    if GLOBAL_SOCKET_MANAGER.set(mgr).is_err() {
        log::warn!("[socket] global SocketManager already set — ignoring duplicate");
    }
}

/// Retrieve the global `SocketManager`, if initialized.
pub fn global_socket_manager() -> Option<&'static Arc<SocketManager>> {
    GLOBAL_SOCKET_MANAGER.get()
}

// ---------------------------------------------------------------------------
// Shared state (visible to sibling modules)
// ---------------------------------------------------------------------------

/// State shared between the `SocketManager` handle and the background loop.
pub(super) struct SharedState {
    /// Router for delivering incoming webhooks to skills.
    pub(super) webhook_router: RwLock<Option<Arc<WebhookRouter>>>,
    /// Pending Socket.IO ACK callbacks keyed by outbound ack id.
    pub(super) ack_registry: AckRegistry,
    /// Current connection status.
    pub(super) status: RwLock<ConnectionStatus>,
    /// Socket ID assigned by the server.
    pub(super) socket_id: RwLock<Option<String>>,
    /// Last user-visible connection warning surfaced through `SocketState.error`
    /// (e.g. "backend redirected ws→wss; update BACKEND_URL"). Cleared on every
    /// successful handshake and on disconnect.
    pub(super) error: RwLock<Option<String>>,
    /// `(url, token)` the background loop is currently authenticating with, and
    /// the reason it lives here rather than on the handle: `ws_loop` re-reads the
    /// token provider before **every** attempt, so a refresh mid-session would
    /// leave a manager-side copy naming a credential this socket no longer uses.
    /// Seeded by `spawn_loop` and rewritten by the loop on each attempt; cleared
    /// on disconnect. Read by [`SocketManager::is_live_for`].
    pub(super) connection_identity: RwLock<Option<(String, String)>>,
}

/// The connection's readiness flag, guarded by a lock so a reader can hold the
/// flag `true` across the send it authorises.
///
/// A bare `AtomicBool` cannot express that: `emit` would load `true`, and the
/// background loop's teardown could then clear the flag *and* drain the emit
/// queue in the gap before `emit`'s `tx.send` runs, so the message lands in the
/// just-drained channel and rides the *next* reconnect's socket (a fresh sid
/// whose roster the backend has already cleared). Both critical sections —
/// `emit`'s "is-ready? then send" and teardown's "clear then drain" — take this
/// lock, so they are mutually exclusive and that interleaving cannot happen. The
/// guard is a leaf: it is only ever held for a synchronous flag read/write plus
/// a non-blocking channel `send`/`try_recv`, never across an `.await` and never
/// while another socket lock is held, so it introduces no lock-ordering hazard
/// with `emit_tx` or the `status` `RwLock`.
pub(super) type EmitReady = Arc<Mutex<bool>>;

/// The outbound emit channel bundled with the readiness flag of the **same**
/// connection that owns it.
///
/// Readiness travels with the channel so `emit` can decide "is this message
/// deliverable?" atomically with picking the channel it would send on: both are
/// read under the single `emit_tx` lock. `spawn_loop` installs a fresh
/// `EmitChannel` (fresh sender + fresh `ready = false` flag) for every
/// connection, and the background loop flips *this connection's* `ready` to
/// `true` only after the Socket.IO CONNECT ACK and back to `false` on teardown.
/// A reconnect therefore swaps the sender and its flag together — an `emit`
/// holding the lock can never pair a live-looking status with a stale
/// pre-handshake channel (the reverse of the TOCTOU the status-only gate had).
///
/// `ready` is additionally the serialization point between `emit` and teardown:
/// see [`EmitReady`] for why the flag is a `Mutex<bool>` rather than an atomic.
pub(super) struct EmitChannel {
    /// Sender into the background loop's outbound queue.
    pub(super) tx: mpsc::UnboundedSender<String>,
    /// `true` only between this connection's CONNECT ACK and its teardown, and
    /// the lock that makes `emit`'s check+send exclusive with teardown's
    /// clear+drain (see [`EmitReady`]).
    pub(super) ready: EmitReady,
}

pub(super) struct AckRegistry {
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>,
}

impl Default for AckRegistry {
    fn default() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
        }
    }
}

impl AckRegistry {
    pub(super) fn register(&self) -> (u64, oneshot::Receiver<serde_json::Value>) {
        let ack_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(ack_id, tx);
        (ack_id, rx)
    }

    pub(super) fn resolve(&self, ack_id: u64, data: serde_json::Value) -> bool {
        if let Some(tx) = self.pending.lock().remove(&ack_id) {
            let _ = tx.send(data);
            true
        } else {
            false
        }
    }

    pub(super) fn remove(&self, ack_id: u64) {
        self.pending.lock().remove(&ack_id);
    }

    pub(super) fn cancel_all(&self) {
        self.pending.lock().clear();
    }
}

// ---------------------------------------------------------------------------
// SocketManager
// ---------------------------------------------------------------------------

/// Manages a persistent Socket.IO connection to the backend.
///
/// Handles protocol-level handshakes (Engine.IO / Socket.IO), heartbeats, and
/// automatic reconnection while providing a high-level API for emitting events
/// and syncing tool state.
pub struct SocketManager {
    /// Shared state accessible from both the manager and the background loop.
    pub(super) shared: Arc<SharedState>,
    /// Channel for sending outgoing messages to the background loop, bundled
    /// with the readiness flag of the connection that owns it. `emit` selects the
    /// channel under this lock, then gates its check+send on the channel's own
    /// `ready` lock, so a reconnect can never swap the channel out mid-emit nor
    /// let teardown drain between the readiness check and the send
    /// (see [`EmitChannel`] and [`EmitReady`]).
    emit_tx: tokio::sync::Mutex<Option<EmitChannel>>,
    /// Channel for signaling the background loop to shut down.
    shutdown_tx: tokio::sync::Mutex<Option<watch::Sender<bool>>>,
    /// Join handle for the background connection loop.
    loop_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Serializes identity-sensitive disconnect → bridge bind → connect
    /// transactions while still allowing ordinary emits and state reads.
    identity_rebind: tokio::sync::Mutex<()>,
}

impl SocketManager {
    /// Create a new, disconnected SocketManager.
    pub fn new() -> Self {
        log::debug!("[socket] SocketManager created (disconnected)");
        Self {
            shared: Arc::new(SharedState {
                webhook_router: RwLock::new(None),
                ack_registry: AckRegistry::default(),
                status: RwLock::new(ConnectionStatus::Disconnected),
                socket_id: RwLock::new(None),
                error: RwLock::new(None),
                connection_identity: RwLock::new(None),
            }),
            emit_tx: tokio::sync::Mutex::new(None),
            shutdown_tx: tokio::sync::Mutex::new(None),
            loop_handle: tokio::sync::Mutex::new(None),
            identity_rebind: tokio::sync::Mutex::new(()),
        }
    }

    /// Lock an identity-sensitive socket rebind for its complete transaction.
    pub(crate) async fn lock_identity_rebind(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.identity_rebind.lock().await
    }

    /// Set the webhook router for skill-targeted webhook delivery.
    pub fn set_webhook_router(&self, router: Arc<WebhookRouter>) {
        log::debug!("[socket] WebhookRouter attached");
        *self.shared.webhook_router.write() = Some(router);
    }

    /// Get the webhook router, if one has been set.
    pub fn webhook_router(&self) -> Option<Arc<WebhookRouter>> {
        self.shared.webhook_router.read().clone()
    }

    /// Get the current socket state (status, ID, error).
    pub fn get_state(&self) -> SocketState {
        SocketState {
            status: *self.shared.status.read(),
            socket_id: self.shared.socket_id.read().clone(),
            error: self.shared.error.read().clone(),
        }
    }

    /// Check if the socket is currently connected.
    pub fn is_connected(&self) -> bool {
        *self.shared.status.read() == ConnectionStatus::Connected
    }

    /// True when a **live** connection is already serving exactly this `url`
    /// under exactly this session token.
    ///
    /// Startup has two independent connect paths — the core's bootstrap
    /// auto-connect and the renderer's `socket_connect_with_session` RPC — and
    /// each unconditionally tore the other's socket down and redid the whole
    /// Engine.IO/Socket.IO handshake, so a cold start opened two EIO sessions a
    /// couple of seconds apart (#6181). Callers consult this before starting an
    /// identity rebind so the second path becomes a no-op.
    ///
    /// Deliberately conservative: a different URL, a different token, or any
    /// status other than `Connected` all report `false`, so an account switch
    /// (same URL, new token) still forces a real reconnect and an unhealthy
    /// socket is still replaced. The token compared against is the one
    /// `ws_loop` used for its most recent attempt, not the one this manager was
    /// handed at spawn — a provider that refreshes the session mid-loop keeps
    /// matching instead of forcing a pointless reconnect.
    pub fn is_live_for(&self, url: &str, token: &str) -> bool {
        self.is_connected()
            && self
                .shared
                .connection_identity
                .read()
                .as_ref()
                .is_some_and(|(u, t)| u == url && t == token)
    }

    // -----------------------------------------------------------------------
    // Connection lifecycle
    // -----------------------------------------------------------------------

    /// Connect to the specified URL using the provided authentication token.
    ///
    /// Spawns a background `ws_loop` that manages the connection with automatic
    /// reconnection and exponential backoff.
    ///
    /// Returns `Err` immediately if `token` is empty — every reconnect attempt
    /// would either 401 at the SIO CONNECT step or fail upstream at the gateway,
    /// producing exactly the kind of retry-storm noise this module is designed to
    /// suppress. Callers receive an actionable error and the RPC response reflects
    /// the actual outcome rather than optimistically reporting `{"status":"Connecting"}`.
    pub async fn connect(&self, url: &str, token: &str) -> Result<(), String> {
        if token.trim().is_empty() {
            log::error!("[socket] connect: refusing to start — empty session token");
            return Err("empty session token — authenticate first".to_string());
        }
        // Wrap the static token in a provider closure. Existing callers that
        // pass a concrete token value continue to work unchanged; the provider
        // returns that same token on every call (static semantics). For
        // live-session refresh, callers should use `connect_with_session` which
        // builds a provider via `token_provider_from_config`.
        let provider = static_token_provider(token.to_string());
        self.spawn_loop(url, provider, token.to_string()).await
    }

    /// Connect using a **live-refresh token provider**.
    ///
    /// Unlike [`connect`] which wraps a single static token, this method
    /// accepts a [`TokenProvider`] closure that is called before every
    /// reconnect attempt. Use this when the token may change between retries
    /// (e.g. after a session refresh or re-login) so the loop always sends the
    /// freshest available credential.
    ///
    /// The provider is called immediately to validate that a token is available
    /// before the background task is spawned — callers receive an actionable
    /// `Err` if no token is stored rather than spawning a doomed retry loop.
    pub async fn connect_with_provider(
        &self,
        url: &str,
        token_provider: TokenProvider,
    ) -> Result<(), String> {
        // Validate that a token is available right now before spawning. This
        // mirrors the empty-token guard in `connect()` and ensures callers
        // see an immediate error if the session store is empty.
        let token = match token_provider() {
            Ok(t) if !t.trim().is_empty() => t,
            Ok(_) => {
                log::error!(
                    "[socket] connect_with_provider: refusing to start — provider returned empty token"
                );
                return Err("empty session token — authenticate first".to_string());
            }
            Err(e) => {
                log::error!(
                    "[socket] connect_with_provider: refusing to start — provider error: {e}"
                );
                return Err(e);
            }
        };
        self.spawn_loop(url, token_provider, token).await
    }

    /// Shared spawn path used by both [`connect`] and [`connect_with_provider`].
    ///
    /// Installs the rustls crypto provider, tears down any existing connection,
    /// records the connection's identity, constructs the channel pair, and
    /// spawns the background `ws_loop` task. Entry-point-specific validation
    /// (empty-token guard, provider pre-check) is done by the callers before this
    /// is called, and `token` is the value they validated — it is recorded, not
    /// sent, so `is_live_for` compares against the credential this connection was
    /// actually started with.
    async fn spawn_loop(
        &self,
        url: &str,
        provider: TokenProvider,
        token: String,
    ) -> Result<(), String> {
        // Ensure the rustls crypto provider is installed (needed for wss:// TLS).
        // This is a no-op if already installed.
        let _ = rustls::crypto::ring::default_provider().install_default();

        self.disconnect().await?;

        // Seed the identity this loop will serve so a redundant connect for the
        // same url+token can be skipped (see `is_live_for`). This is the token
        // the caller already validated, not a fresh provider call: the two must
        // agree, or the guard could match on a credential this connection never
        // used. `ws_loop` rewrites it before every attempt, so a token refreshed
        // mid-session replaces this seed rather than going stale behind it.
        *self.shared.connection_identity.write() = Some((url.to_string(), token));

        log::info!("[socket] Connecting to {}", url);

        *self.shared.status.write() = ConnectionStatus::Connecting;
        *self.shared.error.write() = None;
        emit_state_change(&self.shared);

        let (emit_tx, emit_rx) = mpsc::unbounded_channel::<String>();
        let internal_tx = emit_tx.clone();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Per-connection readiness flag: starts `false` (pre-handshake) and is
        // flipped by `ws_loop` at CONNECT ACK. Bundled with the sender so `emit`
        // reads the flag belonging to exactly this channel, not a flag a later
        // reconnect may have swapped underneath it. It is a `Mutex<bool>` rather
        // than an atomic so `emit`'s check+send and the loop's clear+drain can be
        // made mutually exclusive (see [`EmitReady`]).
        let emit_ready: EmitReady = Arc::new(Mutex::new(false));
        let loop_ready = Arc::clone(&emit_ready);

        *self.emit_tx.lock().await = Some(EmitChannel {
            tx: emit_tx,
            ready: emit_ready,
        });
        *self.shutdown_tx.lock().await = Some(shutdown_tx);

        let url = url.to_string();
        let shared = Arc::clone(&self.shared);

        let handle = tokio::spawn(async move {
            ws_loop(
                url,
                provider,
                shared,
                emit_rx,
                shutdown_rx,
                internal_tx,
                loop_ready,
            )
            .await;
        });

        *self.loop_handle.lock().await = Some(handle);
        Ok(())
    }

    /// Disconnect from the server and shut down the background loop.
    pub async fn disconnect(&self) -> Result<(), String> {
        super::medulla::workflows::end_connection_generation();
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(true);
        }
        self.shared.ack_registry.cancel_all();
        self.emit_tx.lock().await.take();
        if let Some(handle) = self.loop_handle.lock().await.take() {
            terminate_loop(handle, Duration::from_secs(5)).await;
        }
        *self.shared.status.write() = ConnectionStatus::Disconnected;
        *self.shared.socket_id.write() = None;
        *self.shared.error.write() = None;
        *self.shared.connection_identity.write() = None;
        emit_state_change(&self.shared);
        log::debug!("[socket] Disconnected");
        Ok(())
    }

    /// Emit a Socket.IO event to the server.
    ///
    /// Gated on the **owning connection's** readiness flag, not on the emit
    /// channel merely existing nor on the presentation-layer `status`. Two races
    /// motivate this:
    ///
    /// - `spawn_loop` installs `emit_tx` while the handshake is still in flight,
    ///   so a channel-only guard would report success for a message that
    ///   `ws_loop`'s `drain_pending_emits` silently discards if the handshake
    ///   then fails (#4355 / #6084 pre-handshake false success).
    /// - Reading `status` and then acquiring the `emit_tx` lock are two separate
    ///   steps; a reconnect in between could flip `status` and swap in a fresh
    ///   pre-handshake channel, so a `status`-only gate could enqueue onto the
    ///   *new* channel and return `Ok` for a message that channel then drops.
    ///
    /// The flag is created with, owned by, and flipped for a single connection,
    /// and it is read here under the same lock that hands us the channel — so
    /// status and channel can never be swapped mid-emit. Because readiness is
    /// cleared only on that connection's teardown (not on a presentation-layer
    /// `error` event that leaves the socket live), a still-connected socket
    /// keeps accepting emits. A pre-handshake or disconnected emit returns the
    /// same `"Not connected"` error as an emit before `connect` was ever called.
    ///
    /// The readiness check and the `tx.send` it authorises are performed under
    /// the `ready` lock (see [`EmitReady`]), so the background loop's teardown
    /// cannot clear the flag and drain the queue in the gap between them — which
    /// would otherwise let this method return `Ok(())` for a message that lands
    /// in the just-drained channel and rides the *next* reconnect's socket.
    pub async fn emit(&self, event: &str, data: serde_json::Value) -> Result<(), String> {
        // Encode outside the readiness lock: it is fallible and touches neither
        // the flag nor the channel, so there is no reason to widen the critical
        // section around it.
        let msg = encode_sio_event(event, data, None)?;
        let guard = self.emit_tx.lock().await;
        let Some(channel) = guard.as_ref() else {
            return Err("Not connected".to_string());
        };
        // Hold `ready` across the check *and* the send so teardown's clear+drain
        // (which takes the same lock) cannot interleave between them. `send` on an
        // unbounded channel does not block, so this leaf lock is never held over
        // an `.await`.
        let ready = channel.ready.lock();
        if !*ready {
            return Err("Not connected".to_string());
        }
        channel
            .tx
            .send(msg)
            .map_err(|_| "Socket not connected".to_string())
    }

    /// Emit a Socket.IO event and wait for the backend ACK callback.
    ///
    /// Unlike [`emit`](Self::emit), this deliberately does **not** gate on
    /// `Connected`: a message queued while `Connecting` is flushed once the
    /// handshake completes, and if the handshake fails instead the ack simply
    /// never arrives and this returns a timeout `Err`. Because delivery is
    /// confirmed by the ack (or its absence), a pre-handshake `emit_with_ack`
    /// cannot report a false success the way a bare `emit` could (#6084), so it
    /// keeps the queue-then-confirm behaviour rather than rejecting early.
    pub async fn emit_with_ack(
        &self,
        event: &str,
        data: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        let tx = self
            .emit_tx
            .lock()
            .await
            .as_ref()
            .map(|c| c.tx.clone())
            .ok_or_else(|| "Not connected".to_string())?;
        let (ack_id, ack_rx) = self.shared.ack_registry.register();
        let msg = encode_sio_event(event, data, Some(ack_id))?;
        if let Err(e) = tx.send(msg) {
            self.shared.ack_registry.remove(ack_id);
            return Err(format!("Socket not connected: {e}"));
        }

        log::debug!("[socket] emit_with_ack sent event={event} ack_id={ack_id}");
        match tokio::time::timeout(timeout, ack_rx).await {
            Ok(Ok(data)) => {
                log::debug!("[socket] emit_with_ack resolved event={event} ack_id={ack_id}");
                Ok(data)
            }
            Ok(Err(_)) => Err(format!(
                "Socket ack channel dropped for event {event} ack_id={ack_id}"
            )),
            Err(_) => {
                self.shared.ack_registry.remove(ack_id);
                Err(format!(
                    "Socket ack timeout for event {event} ack_id={ack_id}"
                ))
            }
        }
    }
}

/// Wait for the socket loop to observe shutdown, then abort and join it if a
/// transport operation outlives the grace period.
///
/// Dropping a timed-out `JoinHandle` detaches its task. During an account
/// switch that would let the old credential finish authenticating after the
/// new user's workflow bridge is installed, so timeout must mean termination,
/// not detachment.
async fn terminate_loop(mut handle: tokio::task::JoinHandle<()>, grace: Duration) {
    if tokio::time::timeout(grace, &mut handle).await.is_err() {
        log::warn!("[socket] connection loop did not stop within {grace:?} — aborting");
        handle.abort();
        let _ = handle.await;
    }
}

fn encode_sio_event(
    event: &str,
    data: serde_json::Value,
    ack_id: Option<u64>,
) -> Result<String, String> {
    let payload = serde_json::to_string(&json!([event, data])).map_err(|e| format!("{e}"))?;
    let ack = ack_id.map(|id| id.to_string()).unwrap_or_default();
    Ok(format!("42{ack}{payload}"))
}

impl Default for SocketManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// State-change helpers (used by sibling modules)
// ---------------------------------------------------------------------------

/// Log a state change for observability.
pub(super) fn emit_state_change(shared: &SharedState) {
    let status = *shared.status.read();
    let socket_id = shared.socket_id.read().clone();
    log::debug!("[socket] State changed: {:?}, sid={:?}", status, socket_id);
}

/// Log a server event for observability.
pub(super) fn emit_server_event(_shared: &SharedState, event_name: &str, _data: serde_json::Value) {
    log::debug!("[socket] Server event: {}", event_name);
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
