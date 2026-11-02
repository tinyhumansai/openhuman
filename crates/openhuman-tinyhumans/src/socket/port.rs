use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::RwLock;
use serde_json::Value;
use tinyhumans_sdk::{
    socket::{SocketConnection, SocketEvent},
    TinyHumansClient,
};
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::JoinSet;

use openhuman_embed::__host::platform::socket::{
    ConnectionStatus, SocketPort, SocketState, TokenProvider,
};

use super::sdk_token_provider_with_observer;

enum SocketCommand {
    Emit {
        event: String,
        data: Value,
        result: oneshot::Sender<Result<(), String>>,
    },
    EmitWithAck {
        event: String,
        data: Value,
        timeout: Duration,
        result: oneshot::Sender<Result<Value, String>>,
    },
}

enum SocketWaiter {
    Emit(oneshot::Sender<Result<(), String>>),
    EmitWithAck(oneshot::Sender<Result<Value, String>>),
}

enum SocketResult {
    Emit(Result<(), String>),
    EmitWithAck(Result<Value, String>),
}

struct SocketActorHandle {
    commands: mpsc::Sender<SocketCommand>,
    shutdown: watch::Sender<bool>,
    done: oneshot::Receiver<()>,
}

/// TinyHumans' SDK-backed implementation of the core socket port.
pub(crate) struct SdkSocketPort {
    pub(super) state: Arc<RwLock<SocketState>>,
    pub(super) identity: Arc<RwLock<Option<(String, String)>>>,
    transport_connected: Arc<AtomicBool>,
    loop_active: Arc<AtomicBool>,
    stopped_on_failure: Arc<AtomicBool>,
    actor: Mutex<Option<SocketActorHandle>>,
    webhook_router: RwLock<Option<Arc<openhuman_embed::__host::skills::webhooks::WebhookRouter>>>,
}

impl Default for SdkSocketPort {
    fn default() -> Self {
        Self {
            state: Arc::new(RwLock::new(SocketState::default())),
            identity: Arc::new(RwLock::new(None)),
            transport_connected: Arc::new(AtomicBool::new(false)),
            loop_active: Arc::new(AtomicBool::new(false)),
            stopped_on_failure: Arc::new(AtomicBool::new(false)),
            actor: Mutex::new(None),
            webhook_router: RwLock::new(None),
        }
    }
}

#[async_trait]
impl SocketPort for SdkSocketPort {
    async fn connect(&self, url: &str, token: &str) -> Result<(), String> {
        let token = token.to_owned();
        self.connect_with_provider(
            url,
            Arc::new(move || {
                let token = token.clone();
                Box::pin(async move {
                    if token.trim().is_empty() {
                        Err("empty session token — authenticate first".to_owned())
                    } else {
                        Ok(token)
                    }
                })
            }),
        )
        .await
    }

    async fn connect_with_provider(
        &self,
        url: &str,
        provider: TokenProvider,
    ) -> Result<(), String> {
        let identity_token = provider().await?;
        if identity_token.trim().is_empty() {
            return Err("empty session token — authenticate first".to_owned());
        }
        let handshake_url = crate::backend::url::websocket_url(url);
        if !crate::backend::url::is_safe_socket_endpoint(&handshake_url) {
            return Err("backend socket requires TLS or a loopback endpoint".to_owned());
        }
        self.disconnect().await?;

        *self.identity.write() = Some((url.to_owned(), identity_token));
        let identity = Arc::clone(&self.identity);
        let identity_url = url.to_owned();

        let connection = TinyHumansClient::new(url)
            .connect_socket_with_token_provider(sdk_token_provider_with_observer(
                provider,
                move |token| *identity.write() = Some((identity_url.clone(), token)),
            ))
            .await
            .map_err(|error| {
                *self.identity.write() = None;
                error.to_string()
            })?;
        let (tx, rx) = mpsc::channel(128);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let (done, done_rx) = oneshot::channel();
        *self.actor.lock().await = Some(SocketActorHandle {
            commands: tx,
            shutdown,
            done: done_rx,
        });
        *self.state.write() = SocketState {
            status: ConnectionStatus::Connecting,
            socket_id: None,
            error: None,
        };
        self.loop_active.store(true, Ordering::Release);
        self.transport_connected.store(false, Ordering::Release);
        self.stopped_on_failure.store(false, Ordering::Release);
        tokio::spawn(run_socket_actor(
            connection,
            rx,
            shutdown_rx,
            done,
            Arc::clone(&self.state),
            Arc::clone(&self.transport_connected),
            Arc::clone(&self.loop_active),
            Arc::clone(&self.stopped_on_failure),
        ));
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), String> {
        if let Some(actor) = self.actor.lock().await.take() {
            actor.shutdown.send_replace(true);
            let _ = actor.done.await;
        }
        self.loop_active.store(false, Ordering::Release);
        self.transport_connected.store(false, Ordering::Release);
        self.stopped_on_failure.store(false, Ordering::Release);
        *self.identity.write() = None;
        *self.state.write() = SocketState::default();
        Ok(())
    }

    async fn emit(&self, event: &str, data: Value) -> Result<(), String> {
        if !self.is_connected() {
            return Err("Not connected".to_owned());
        }
        let actor = self.actor.lock().await;
        let commands = actor
            .as_ref()
            .map(|actor| actor.commands.clone())
            .ok_or("Not connected")?;
        drop(actor);
        let (result, response) = oneshot::channel();
        commands
            .send(SocketCommand::Emit {
                event: event.to_owned(),
                data,
                result,
            })
            .await
            .map_err(|_| "Socket not connected".to_owned())?;
        response
            .await
            .map_err(|_| "Socket not connected".to_owned())?
    }

    async fn emit_with_ack(
        &self,
        event: &str,
        data: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let actor = self.actor.lock().await;
        let commands = actor
            .as_ref()
            .map(|actor| actor.commands.clone())
            .ok_or("Not connected")?;
        drop(actor);
        let (result, response) = oneshot::channel();
        commands
            .send(SocketCommand::EmitWithAck {
                event: event.to_owned(),
                data,
                timeout,
                result,
            })
            .await
            .map_err(|_| "Socket not connected".to_owned())?;
        response
            .await
            .map_err(|_| "Socket not connected".to_owned())?
    }

    fn get_state(&self) -> SocketState {
        self.state.read().clone()
    }

    fn is_connected(&self) -> bool {
        self.transport_connected.load(Ordering::Acquire)
    }

    fn is_live_for(&self, url: &str, token: &str) -> bool {
        let state = self.state.read();
        (self.transport_connected.load(Ordering::Acquire)
            || (state.status == ConnectionStatus::Connecting
                && self.loop_active.load(Ordering::Acquire)))
            && self
                .identity
                .read()
                .as_ref()
                .is_some_and(|(active_url, active_token)| {
                    active_url == url && active_token == token
                })
    }

    fn status_for(&self, url: &str, token: &str) -> Option<ConnectionStatus> {
        self.identity
            .read()
            .as_ref()
            .filter(|(active_url, active_token)| active_url == url && active_token == token)
            .map(|_| self.state.read().status)
    }

    fn is_loop_active(&self) -> bool {
        self.loop_active.load(Ordering::Acquire)
    }

    fn loop_stopped_on_failure(&self) -> bool {
        self.stopped_on_failure.load(Ordering::Acquire)
    }

    fn set_webhook_router(
        &self,
        router: Arc<openhuman_embed::__host::skills::webhooks::WebhookRouter>,
    ) {
        *self.webhook_router.write() = Some(router);
    }

    fn webhook_router(
        &self,
    ) -> Option<Arc<openhuman_embed::__host::skills::webhooks::WebhookRouter>> {
        self.webhook_router.read().clone()
    }
}

async fn run_socket_actor(
    mut connection: SocketConnection,
    mut commands: mpsc::Receiver<SocketCommand>,
    mut shutdown: watch::Receiver<bool>,
    done: oneshot::Sender<()>,
    state: Arc<RwLock<SocketState>>,
    transport_connected: Arc<AtomicBool>,
    loop_active: Arc<AtomicBool>,
    stopped_on_failure: Arc<AtomicBool>,
) {
    let emitter = connection.emitter();
    let mut tasks = JoinSet::<(u64, SocketResult)>::new();
    let mut waiters = std::collections::HashMap::<u64, SocketWaiter>::new();
    let mut next_task_id = 0u64;

    'actor: loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    let _ = connection.disconnect().await;
                    break 'actor;
                }
            }
            event = connection.next_event() => {
                let Some(event) = event else {
                    transport_connected.store(false, Ordering::Release);
                    loop_active.store(false, Ordering::Release);
                    stopped_on_failure.store(connection.stopped_on_failure(), Ordering::Release);
                    if connection.stopped_on_failure() {
                        state.write().status = ConnectionStatus::Error;
                    }
                    break 'actor;
                };
                if apply_supervisor_event(
                    &event,
                    &mut state.write(),
                    &transport_connected,
                    &loop_active,
                    &stopped_on_failure,
                    connection.stopped_on_failure(),
                ) {
                    continue;
                }
                if !should_dispatch_to_core(&event) {
                    log::debug!("[socket] unhandled SDK supervisor event: {}", event.name);
                    continue;
                }
                apply_backend_status_event(&event, &mut state.write());
                let data = event.payload.decode::<Value>(&event.name).unwrap_or(Value::Null);
                crate::devices::tunnel_client::dispatch_backend_event(&event.name, data.clone());
                openhuman_embed::__host::platform::socket::dispatch_socket_event(&event.name, data);
            }
            command = commands.recv() => {
                let Some(command) = command else { break; };
                match command {
                    SocketCommand::Emit { event, data, result } => {
                        next_task_id = next_task_id.wrapping_add(1);
                        let id = next_task_id;
                        waiters.insert(id, SocketWaiter::Emit(result));
                        let emitter = emitter.clone();
                        tasks.spawn(async move {
                            (id, SocketResult::Emit(emitter.emit(&event, &data).await.map_err(|error| error.to_string())))
                        });
                    }
                    SocketCommand::EmitWithAck { event, data, timeout, result } => {
                        next_task_id = next_task_id.wrapping_add(1);
                        let id = next_task_id;
                        waiters.insert(id, SocketWaiter::EmitWithAck(result));
                        let emitter = emitter.clone();
                        tasks.spawn(async move {
                            (id, SocketResult::EmitWithAck(emitter.emit_with_ack::<Value, Value>(&event, &data, timeout).await.map_err(|error| error.to_string())))
                        });
                    }
                }
            }
            completed = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Ok((id, result))) = completed {
                    if let Some(waiter) = waiters.remove(&id) {
                        match (waiter, result) {
                            (SocketWaiter::Emit(sender), SocketResult::Emit(result)) => { let _ = sender.send(result); }
                            (SocketWaiter::EmitWithAck(sender), SocketResult::EmitWithAck(result)) => { let _ = sender.send(result); }
                            (SocketWaiter::Emit(sender), SocketResult::EmitWithAck(_)) => { let _ = sender.send(Err("Socket operation result mismatch".to_owned())); }
                            (SocketWaiter::EmitWithAck(sender), SocketResult::Emit(_)) => { let _ = sender.send(Err("Socket operation result mismatch".to_owned())); }
                        }
                    }
                }
            }
        }
    }
    let _ = connection.disconnect().await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    for (_, waiter) in waiters {
        match waiter {
            SocketWaiter::Emit(sender) => {
                let _ = sender.send(Err("Socket disconnected".to_owned()));
            }
            SocketWaiter::EmitWithAck(sender) => {
                let _ = sender.send(Err("Socket disconnected".to_owned()));
            }
        }
    }
    while let Ok(command) = commands.try_recv() {
        match command {
            SocketCommand::Emit { result, .. } => {
                let _ = result.send(Err("Socket disconnected".to_owned()));
            }
            SocketCommand::EmitWithAck { result, .. } => {
                let _ = result.send(Err("Socket disconnected".to_owned()));
            }
        }
    }
    loop_active.store(false, Ordering::Release);
    transport_connected.store(false, Ordering::Release);
    let _ = done.send(());
}

fn should_dispatch_to_core(event: &SocketEvent) -> bool {
    event.source == tinyhumans_sdk::socket::SocketEventSource::Backend
}

fn apply_supervisor_event(
    event: &SocketEvent,
    state: &mut SocketState,
    transport_connected: &AtomicBool,
    loop_active: &AtomicBool,
    stopped_on_failure: &AtomicBool,
    stopped_on_failure_now: bool,
) -> bool {
    if event.source != tinyhumans_sdk::socket::SocketEventSource::Supervisor {
        return false;
    }
    match event.name.as_str() {
        "open" => {
            state.status = ConnectionStatus::Connecting;
            transport_connected.store(false, Ordering::Release);
        }
        "connect" => {
            apply_connect_event(state, event);
            transport_connected.store(true, Ordering::Release);
        }
        "close" => {
            state.status = ConnectionStatus::Reconnecting;
            state.socket_id = None;
            transport_connected.store(false, Ordering::Release);
            loop_active.store(true, Ordering::Release);
        }
        "error" => {
            state.status = ConnectionStatus::Reconnecting;
            state.error = decode_error_message(event);
            transport_connected.store(false, Ordering::Release);
            stopped_on_failure.store(stopped_on_failure_now, Ordering::Release);
        }
        _ => return false,
    }
    true
}

fn apply_backend_status_event(event: &SocketEvent, state: &mut SocketState) -> bool {
    if event.source != tinyhumans_sdk::socket::SocketEventSource::Backend {
        return false;
    }
    match event.name.as_str() {
        "ready" => {
            state.status = ConnectionStatus::Connected;
            state.error = None;
        }
        "error" => {
            state.status = ConnectionStatus::Error;
            state.error = decode_error_message(event);
        }
        _ => return false,
    }
    true
}

fn decode_error_message(event: &SocketEvent) -> Option<String> {
    event
        .payload
        .decode::<Value>(&event.name)
        .ok()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        })
}

fn apply_connect_event(state: &mut SocketState, event: &SocketEvent) {
    state.status = ConnectionStatus::Connected;
    state.socket_id = event
        .payload
        .decode::<Value>("connect")
        .ok()
        .and_then(|value| value.get("sid").and_then(Value::as_str).map(str::to_owned));
}

#[cfg(test)]
#[path = "port_tests.rs"]
mod tests;
