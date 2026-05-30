//! Yuanbao WebSocket connection manager.
//!
//! Owns one WebSocket to the gateway and runs:
//!   1. token sign-fetch (via [`SignManager`]) → `auth-bind` handshake
//!   2. periodic `ping` heartbeats
//!   3. inbound frame fan-out (decoded `ConnFrame` → mpsc)
//!   4. outbound request/response correlation via per-`msg_id` oneshot
//!   5. exponential-backoff reconnect with a no-retry close-code allowlist
//!
//! All public APIs are `&self` so the connection can be wrapped in
//! `Arc<…>` and shared between the listen loop and outbound senders.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex as ParkingMutex;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::time;
use tokio_tungstenite::{
    connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream,
};
use tracing::{error, info, warn};
use uuid::Uuid;

use super::config::YuanbaoConfig;
use super::errors::{YuanbaoError, NO_RECONNECT_CLOSE_CODES};
use super::proto::{
    decode_auth_bind_rsp, decode_conn_msg, encode_auth_bind, encode_ping, encode_push_ack,
};
use super::proto_constants::*;
use super::sign::SignManager;
use super::types::{Account, ConnFrame, ConnectionState};

type WsSender =
    futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

/// One inbound event delivered to the listen loop.
pub enum InboundEvent {
    /// A regular biz push.
    Push(ConnFrame),
    /// Server told us we were kicked off.
    Kickout(String),
}

/// In-flight outbound request awaiting a matching `Response` frame.
type PendingMap = HashMap<String, oneshot::Sender<ConnFrame>>;

/// Long-lived connection manager.
pub struct YuanbaoConnection {
    config: YuanbaoConfig,
    state: ParkingMutex<ConnectionState>,
    is_connected: AtomicBool,
    msg_id_seq: AtomicU64,
    sender: Mutex<Option<WsSender>>,
    inbound_tx: mpsc::UnboundedSender<InboundEvent>,
    account: ParkingMutex<Account>,
    sign_manager: Option<Arc<SignManager>>,
    pending: ParkingMutex<PendingMap>,
}

impl YuanbaoConnection {
    pub fn new(
        config: YuanbaoConfig,
        inbound_tx: mpsc::UnboundedSender<InboundEvent>,
        sign_manager: Option<Arc<SignManager>>,
    ) -> Arc<Self> {
        let initial_account = Account {
            uid: config.bot_id.clone(),
            ..Default::default()
        };
        Arc::new(Self {
            config,
            state: ParkingMutex::new(ConnectionState::Disconnected),
            is_connected: AtomicBool::new(false),
            msg_id_seq: AtomicU64::new(1),
            sender: Mutex::new(None),
            inbound_tx,
            account: ParkingMutex::new(initial_account),
            sign_manager,
            pending: ParkingMutex::new(HashMap::new()),
        })
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::Relaxed)
    }

    pub fn state(&self) -> ConnectionState {
        *self.state.lock()
    }

    fn set_state(&self, new: ConnectionState) {
        *self.state.lock() = new;
        self.is_connected
            .store(matches!(new, ConnectionState::Connected), Ordering::Relaxed);
    }

    /// Current account info (best-effort — empty fields until auth-bind succeeds).
    pub fn account(&self) -> Account {
        self.account.lock().clone()
    }

    fn update_account(&self, f: impl FnOnce(&mut Account)) {
        let mut g = self.account.lock();
        f(&mut g);
    }

    /// Per-process monotonic application msg_id.
    pub fn next_msg_id(&self, prefix: &str) -> String {
        let n = self.msg_id_seq.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}_{n}")
    }

    /// Send a raw binary frame. Returns `NotConnected` if the connection
    /// isn't currently up.
    pub async fn send_frame(&self, data: Vec<u8>) -> Result<(), YuanbaoError> {
        let mut guard = self.sender.lock().await;
        match guard.as_mut() {
            Some(s) => s
                .send(Message::Binary(data))
                .await
                .map_err(|e| YuanbaoError::WebSocket(e.to_string())),
            None => Err(YuanbaoError::NotConnected),
        }
    }

    /// Send an already-encoded `ConnMsg` (alias of `send_frame`).
    pub async fn send_conn_msg(&self, frame_bytes: Vec<u8>) -> Result<(), YuanbaoError> {
        self.send_frame(frame_bytes).await
    }

    /// Send a request and wait for the matching `Response` (correlated by
    /// `msg_id`). Times out after `timeout` and removes the pending entry.
    pub async fn send_and_wait(
        &self,
        msg_id: &str,
        frame_bytes: Vec<u8>,
        timeout: Duration,
    ) -> Result<ConnFrame, YuanbaoError> {
        let (tx, rx) = oneshot::channel();
        {
            let mut p = self.pending.lock();
            p.insert(msg_id.to_string(), tx);
        }
        if let Err(e) = self.send_frame(frame_bytes).await {
            self.pending.lock().remove(msg_id);
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(frame)) => Ok(frame),
            Ok(Err(_)) => {
                self.pending.lock().remove(msg_id);
                Err(YuanbaoError::SendFailed(format!(
                    "correlator channel closed for msg_id={msg_id}"
                )))
            }
            Err(_) => {
                self.pending.lock().remove(msg_id);
                Err(YuanbaoError::Timeout(format!("msg_id={msg_id}")))
            }
        }
    }

    /// Trigger a graceful shutdown.
    pub async fn shutdown(&self) {
        let mut guard = self.sender.lock().await;
        if let Some(mut s) = guard.take() {
            let _ = s.send(Message::Close(None)).await;
            let _ = s.close().await;
        }
        // Drop all pending waiters so callers stop hanging.
        let mut pending = self.pending.lock();
        pending.clear();
        self.set_state(ConnectionState::Disconnected);
    }

    /// Main reconnection loop. Returns when `shutdown` flips to `true`.
    pub async fn run(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        let max_attempts = if self.config.max_reconnect_attempts > 0 {
            self.config.max_reconnect_attempts
        } else {
            MAX_RECONNECT_ATTEMPTS
        };
        let mut attempt: u32 = 0;

        loop {
            if *shutdown.borrow() {
                info!("[yuanbao] shutdown signaled, stopping connection loop");
                self.shutdown().await;
                return;
            }
            if attempt >= max_attempts {
                error!("[yuanbao] giving up after {} reconnect attempts", attempt);
                return;
            }

            self.set_state(if attempt == 0 {
                ConnectionState::Connecting
            } else {
                ConnectionState::Reconnecting
            });

            let outcome = self.connect_once(&mut shutdown).await;
            match outcome {
                Ok(Some(code)) if NO_RECONNECT_CLOSE_CODES.contains(&code) => {
                    error!("[yuanbao] no-reconnect close code {} — stopping", code);
                    return;
                }
                Ok(close_code) => {
                    // Successful connection: reset the attempt counter so
                    // intermittent disconnects don't permanently exhaust the
                    // reconnect budget.
                    attempt = 0;
                    info!("[yuanbao] connection closed (code={:?})", close_code);
                }
                Err(e) => warn!("[yuanbao] connection error: {}", e),
            }

            self.set_state(ConnectionState::Disconnected);
            *self.sender.lock().await = None;
            self.pending.lock().clear();

            // `connect_once` may have returned because shutdown fired inside
            // its read loop. In that case we must not sleep through the
            // reconnect backoff — exit immediately so stop is responsive.
            if *shutdown.borrow() {
                info!("[yuanbao] shutdown signaled, stopping connection loop");
                self.shutdown().await;
                return;
            }

            attempt += 1;
            let delay = backoff_seconds(attempt);
            info!(
                "[yuanbao] reconnecting in {}s (attempt {}/{})",
                delay, attempt, max_attempts
            );
            tokio::select! {
                _ = time::sleep(Duration::from_secs(delay)) => {}
                _ = shutdown.changed() => {
                    info!("[yuanbao] shutdown received during backoff");
                    self.shutdown().await;
                    return;
                }
            }
        }
    }

    async fn connect_once(
        &self,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<Option<u16>, YuanbaoError> {
        // Resolve token (may hit the sign endpoint).
        let (token, bot_id, source) = self.resolve_token().await?;
        if !bot_id.is_empty() {
            self.update_account(|a| {
                if a.uid.is_empty() {
                    a.uid = bot_id.clone();
                }
            });
        }

        let url = &self.config.ws_domain;
        info!("[yuanbao] connecting to {}", url);
        let (ws_stream, _resp) = connect_async(url)
            .await
            .map_err(|e| YuanbaoError::WebSocket(e.to_string()))?;

        let (sender, mut receiver) = ws_stream.split();
        *self.sender.lock().await = Some(sender);
        info!("[yuanbao] WebSocket connected — sending auth-bind");

        self.set_state(ConnectionState::Authenticating);
        self.send_auth_bind(&token, &bot_id, &source).await?;

        // Wait for auth-bind response.
        let auth_timeout = Duration::from_secs(AUTH_TIMEOUT_SECS);
        let auth_msg = tokio::time::timeout(auth_timeout, receiver.next())
            .await
            .map_err(|_| YuanbaoError::AuthTimeout)?
            .ok_or_else(|| YuanbaoError::WebSocket("closed during auth-bind".into()))?
            .map_err(|e| YuanbaoError::WebSocket(e.to_string()))?;

        self.handle_auth_response(&auth_msg)?;
        self.set_state(ConnectionState::Connected);
        info!("[yuanbao] auth-bind successful, entering read loop");

        let ping_secs = if self.config.heartbeat_interval_secs > 0 {
            self.config.heartbeat_interval_secs
        } else {
            PING_INTERVAL_SECS
        };
        let mut ping_interval = time::interval(Duration::from_secs(ping_secs));
        ping_interval.tick().await; // skip first tick

        let mut close_code: Option<u16> = None;
        let mut consecutive_ping_failures: u32 = 0;

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    info!("[yuanbao] shutdown received in read loop");
                    return Ok(None);
                }
                _ = ping_interval.tick() => {
                    let msg_id = self.next_msg_id("ping");
                    let frame = encode_ping(&msg_id);
                    if let Err(e) = self.send_frame(frame).await {
                        warn!("[yuanbao] ping send failed: {}", e);
                        consecutive_ping_failures += 1;
                        if consecutive_ping_failures >= HEARTBEAT_TIMEOUT_THRESHOLD {
                            warn!(
                                "[yuanbao] {} consecutive ping failures — dropping",
                                consecutive_ping_failures
                            );
                            break;
                        }
                    } else {
                        consecutive_ping_failures = 0;
                    }
                }
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(Message::Binary(data))) => self.handle_binary(data).await,
                        Some(Ok(Message::Close(frame))) => {
                            close_code = frame.map(|f| u16::from(f.code));
                            info!("[yuanbao] received close frame: {:?}", close_code);
                            break;
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            let mut guard = self.sender.lock().await;
                            if let Some(s) = guard.as_mut() {
                                let _ = s.send(Message::Pong(payload)).await;
                            }
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            warn!("[yuanbao] websocket read error: {}", e);
                            break;
                        }
                        None => {
                            info!("[yuanbao] websocket stream ended");
                            break;
                        }
                    }
                }
            }
        }

        Ok(close_code)
    }

    async fn resolve_token(&self) -> Result<(String, String, String), YuanbaoError> {
        let cfg = &self.config;
        if !cfg.token.is_empty() {
            // Pre-signed token: no source returned by the sign endpoint.
            // Mirrors yuanbao-openclaw-plugin's static-token branch, which
            // returns source="bot".
            return Ok((cfg.token.clone(), cfg.bot_id.clone(), String::new()));
        }
        let mgr = self
            .sign_manager
            .as_ref()
            .ok_or_else(|| YuanbaoError::AuthFailed("no token and no SignManager".into()))?;
        if cfg.app_secret.is_empty() {
            return Err(YuanbaoError::AuthFailed(
                "app_secret required to sign".into(),
            ));
        }
        let entry = mgr
            .get_token(
                &cfg.app_key,
                &cfg.app_secret,
                &cfg.api_domain,
                &cfg.route_env,
            )
            .await?;
        Ok((entry.token, entry.bot_id, entry.source))
    }

    async fn send_auth_bind(
        &self,
        token: &str,
        bot_id: &str,
        source: &str,
    ) -> Result<(), YuanbaoError> {
        let cfg = &self.config;
        let uid = if bot_id.is_empty() {
            self.account.lock().uid.clone()
        } else {
            bot_id.to_string()
        };
        let msg_id = format!("auth_{}", Uuid::new_v4());
        // Auth-bind payload aligned with yuanbao-openclaw-plugin:
        //   biz_id = "ybBot" (server rejects raw app_key with 40011).
        //   source comes from the sign endpoint response; fall back to
        //   "bot" when missing (matches the plugin's static-token branch
        //   and `data.source || "bot"` resolution).
        let resolved_source = if source.is_empty() { "bot" } else { source };
        // Align with yuanbao-openclaw-plugin: app_version → plugin_version,
        // DeviceInfo field 24 → bot_version (OpenHuman framework / CARGO_PKG_VERSION).
        let plugin_version = super::config::strip_version_prefix(&cfg.bot_version);
        let framework_version = env!("CARGO_PKG_VERSION");
        let frame = encode_auth_bind(
            "ybBot",
            &uid,
            resolved_source,
            token,
            &msg_id,
            plugin_version,
            std::env::consts::OS,
            framework_version,
            &cfg.route_env,
        );
        self.send_frame(frame).await
    }

    fn handle_auth_response(&self, msg: &Message) -> Result<(), YuanbaoError> {
        let data = match msg {
            Message::Binary(b) => b,
            _ => {
                return Err(YuanbaoError::AuthFailed(
                    "expected binary auth-bind response".into(),
                ))
            }
        };
        let frame = decode_conn_msg(data)?;
        if frame.cmd != cmd::AUTH_BIND {
            return Err(YuanbaoError::AuthFailed(format!(
                "unexpected cmd in auth response: {:?}",
                frame.cmd
            )));
        }
        if frame.status != 0 {
            return Err(YuanbaoError::AuthFailed(format!(
                "auth rejected: status={}",
                frame.status
            )));
        }
        // Body carries code/message/connect_id — back-fill the account.
        if !frame.data.is_empty() {
            let rsp = decode_auth_bind_rsp(&frame.data)?;
            if rsp.code != 0 {
                return Err(YuanbaoError::AuthFailed(format!(
                    "auth-bind code={} message={}",
                    rsp.code, rsp.message
                )));
            }
            if !rsp.connect_id.is_empty() {
                self.update_account(|a| a.connect_id = rsp.connect_id.clone());
                info!("[yuanbao] auth-bind connect_id={}", rsp.connect_id);
            }
        }
        Ok(())
    }

    async fn handle_binary(&self, data: Vec<u8>) {
        let frame = match decode_conn_msg(&data) {
            Ok(f) => f,
            Err(e) => {
                warn!("[yuanbao] failed to decode binary frame: {}", e);
                return;
            }
        };

        info!(
            "[yuanbao] rx cmd={} module={} cmd_type={} seq={} msg_id={} data_len={}",
            frame.cmd,
            frame.module,
            frame.cmd_type,
            frame.seq_no,
            frame.msg_id,
            frame.data.len()
        );

        // Responses → match against pending requests via msg_id.
        if frame.cmd_type == cmd_type::RESPONSE {
            if !frame.msg_id.is_empty() {
                if let Some(tx) = self.pending.lock().remove(&frame.msg_id) {
                    let _ = tx.send(frame);
                    return;
                }
            }
            info!(
                "[yuanbao] response with no waiter cmd={} msg_id={}",
                frame.cmd, frame.msg_id
            );
            return;
        }

        // For server-driven pushes, ACK first when the head asks for it.
        if frame.cmd_type == cmd_type::PUSH && frame.need_ack {
            let ack = encode_push_ack(&frame);
            if let Err(e) = self.send_frame(ack).await {
                warn!("[yuanbao] failed to send PushAck: {}", e);
            }
        }

        // Handle conn-level builtin pushes inline.
        if frame.cmd == cmd::KICKOUT {
            let reason = String::from_utf8_lossy(&frame.data).into_owned();
            warn!("[yuanbao] kickout received: {}", reason);
            let _ = self.inbound_tx.send(InboundEvent::Kickout(reason));
            return;
        }
        if frame.cmd == cmd::UPDATE_META {
            return;
        }

        if frame.cmd_type != cmd_type::PUSH {
            info!(
                "[yuanbao] dropping non-push frame cmd_type={} cmd={}",
                frame.cmd_type, frame.cmd
            );
            return;
        }

        info!(
            "[yuanbao] push forwarded to listener cmd={} module={} seq={}",
            frame.cmd, frame.module, frame.seq_no
        );
        if self.inbound_tx.send(InboundEvent::Push(frame)).is_err() {
            error!("[yuanbao] inbound channel closed — listener gone");
        }
    }
}

/// Backoff schedule used by `run()`. After the configured table is
/// exhausted we cap at the last entry forever (until the attempt budget
/// trips). Indexing is 1-based so attempt=1 → table[0].
fn backoff_seconds(attempt: u32) -> u64 {
    let idx = attempt.saturating_sub(1) as usize;
    if idx < RECONNECT_DELAYS.len() {
        RECONNECT_DELAYS[idx]
    } else {
        *RECONNECT_DELAYS.last().unwrap_or(&60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_follows_schedule() {
        assert_eq!(backoff_seconds(1), 1);
        assert_eq!(backoff_seconds(2), 2);
        assert_eq!(backoff_seconds(3), 5);
        assert_eq!(backoff_seconds(6), 60);
        assert_eq!(backoff_seconds(100), 60);
        assert_eq!(backoff_seconds(0), 1);
    }

    fn cfg() -> YuanbaoConfig {
        let mut c = YuanbaoConfig::default();
        c.app_key = "ak".into();
        c.ws_domain = "wss://example".into();
        c.token = "tok".into();
        c.bot_id = "bot1".into();
        c
    }

    #[tokio::test]
    async fn pending_correlator_times_out() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        let err = conn
            .send_and_wait("missing_id", vec![1, 2, 3], Duration::from_millis(20))
            .await
            .unwrap_err();
        // Without a connected socket, send_frame fails first → SendFailed/NotConnected.
        assert!(matches!(
            err,
            YuanbaoError::NotConnected | YuanbaoError::Timeout(_) | YuanbaoError::SendFailed(_)
        ));
    }

    #[tokio::test]
    async fn account_back_fill_picks_up_uid() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        assert_eq!(conn.account().uid, "bot1");
        conn.update_account(|a| a.connect_id = "cid_xyz".into());
        assert_eq!(conn.account().connect_id, "cid_xyz");
    }

    #[tokio::test]
    async fn next_msg_id_is_monotonic_and_prefixed() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        let a = conn.next_msg_id("pfx");
        let b = conn.next_msg_id("pfx");
        assert!(a.starts_with("pfx_"));
        assert!(b.starts_with("pfx_"));
        // Suffix is monotonically increasing.
        let na: u64 = a.strip_prefix("pfx_").unwrap().parse().unwrap();
        let nb: u64 = b.strip_prefix("pfx_").unwrap().parse().unwrap();
        assert!(nb > na);
    }

    #[tokio::test]
    async fn initial_state_is_disconnected() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        assert_eq!(conn.state(), ConnectionState::Disconnected);
        assert!(!conn.is_connected());
    }

    #[tokio::test]
    async fn set_state_connected_flips_is_connected_flag() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        conn.set_state(ConnectionState::Connected);
        assert_eq!(conn.state(), ConnectionState::Connected);
        assert!(conn.is_connected());
        conn.set_state(ConnectionState::Reconnecting);
        assert_eq!(conn.state(), ConnectionState::Reconnecting);
        assert!(!conn.is_connected());
    }

    #[tokio::test]
    async fn send_frame_without_socket_returns_not_connected() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        let err = conn.send_frame(vec![1, 2, 3]).await.unwrap_err();
        assert!(matches!(err, YuanbaoError::NotConnected));
        let err2 = conn.send_conn_msg(vec![4]).await.unwrap_err();
        assert!(matches!(err2, YuanbaoError::NotConnected));
    }

    #[tokio::test]
    async fn shutdown_clears_pending_and_sets_disconnected() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        conn.set_state(ConnectionState::Connected);
        // Drop a phantom pending entry then shutdown.
        let (phantom_tx, _phantom_rx) = oneshot::channel();
        conn.pending.lock().insert("ghost".into(), phantom_tx);
        conn.shutdown().await;
        assert_eq!(conn.state(), ConnectionState::Disconnected);
        assert!(!conn.is_connected());
        assert!(conn.pending.lock().is_empty());
    }

    #[test]
    fn backoff_caps_at_last_entry_for_huge_attempts() {
        let last = *RECONNECT_DELAYS.last().unwrap();
        assert_eq!(backoff_seconds(RECONNECT_DELAYS.len() as u32 + 5), last);
    }

    #[tokio::test]
    async fn resolve_token_uses_static_token_when_present() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        let (token, bot_id, source) = conn.resolve_token().await.unwrap();
        assert_eq!(token, "tok");
        assert_eq!(bot_id, "bot1");
        assert_eq!(source, "");
    }

    #[tokio::test]
    async fn resolve_token_without_token_and_without_sign_manager_errors() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut c = cfg();
        c.token = String::new();
        let conn = YuanbaoConnection::new(c, tx, None);
        match conn.resolve_token().await.unwrap_err() {
            YuanbaoError::AuthFailed(m) => assert!(m.contains("no token"), "got {m}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_token_with_sign_manager_but_no_app_secret_errors() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut c = cfg();
        c.token = String::new();
        c.app_secret = String::new();
        let mgr = SignManager::new(reqwest::Client::new());
        let conn = YuanbaoConnection::new(c, tx, Some(mgr));
        match conn.resolve_token().await.unwrap_err() {
            YuanbaoError::AuthFailed(m) => assert!(m.contains("app_secret"), "got {m}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_auth_bind_without_socket_returns_not_connected() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        let err = conn.send_auth_bind("tok", "bot1", "bot").await.unwrap_err();
        assert!(matches!(err, YuanbaoError::NotConnected));
    }

    #[tokio::test]
    async fn send_auth_bind_falls_back_to_account_uid_when_bot_id_empty() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        // bot_id="" → reads from account.uid (which was seeded from cfg.bot_id="bot1")
        let err = conn.send_auth_bind("tok", "", "").await.unwrap_err();
        assert!(matches!(err, YuanbaoError::NotConnected));
        // Account uid still in place.
        assert_eq!(conn.account().uid, "bot1");
    }

    #[test]
    fn handle_auth_response_rejects_non_binary_message() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        let msg = Message::Text("nope".into());
        match conn.handle_auth_response(&msg).unwrap_err() {
            YuanbaoError::AuthFailed(m) => {
                assert!(m.contains("binary"), "got {m}")
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn handle_auth_response_rejects_undecodable_binary() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        // Wholly invalid wire data — decode_conn_msg fails.
        let msg = Message::Binary(vec![0xFF, 0xFF, 0xFF, 0xFF]);
        let err = conn.handle_auth_response(&msg).unwrap_err();
        // Either Proto decode error or some other surface — must not be Ok.
        assert!(
            !matches!(err, YuanbaoError::AuthFailed(_) if format!("{err:?}").contains("binary"))
        );
    }

    /// Regression guard for the post-`connect_once` shutdown short-circuit:
    /// once shutdown is signaled, `run()` must not block on the reconnect
    /// backoff. We force connect_once to fail synchronously (invalid WS URL),
    /// then signal shutdown — total runtime must be well under the first
    /// backoff slot (`backoff_seconds(1) == 1s`).
    #[tokio::test]
    async fn run_exits_promptly_after_shutdown_signal() {
        use std::time::Instant;
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut c = cfg();
        // tokio-tungstenite rejects the URL synchronously — connect_once
        // returns Err in microseconds, putting `run()` on the post-connect
        // cleanup path that the fix targets.
        c.ws_domain = "not-a-valid-ws-url".to_string();
        c.max_reconnect_attempts = 100;
        let conn = YuanbaoConnection::new(c, tx, None);
        let (sd_tx, sd_rx) = watch::channel(false);

        let handle = tokio::spawn(conn.clone().run(sd_rx));
        // Let `run()` enter the loop and attempt connect_once at least once.
        time::sleep(Duration::from_millis(20)).await;

        let started = Instant::now();
        sd_tx.send(true).unwrap();

        // The first reconnect backoff slot is 1s. Without responsive
        // shutdown handling, run() would sleep through it before checking
        // the flag. 500ms gives us comfortable headroom while staying
        // far enough below the backoff to detect a regression.
        let res = time::timeout(Duration::from_millis(500), handle).await;
        res.expect("run() did not exit within 500ms of shutdown signal")
            .expect("run() task panicked");
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "run() took {:?} to exit after shutdown — backoff was not skipped",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn handle_binary_with_garbage_does_not_panic() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = YuanbaoConnection::new(cfg(), tx, None);
        // Should silently log + return — no panic.
        conn.handle_binary(vec![0xFF, 0xFF, 0xFF, 0xFF]).await;
    }
}
