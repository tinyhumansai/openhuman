//! TinyMCP server-protocol adapter over the published bus contract.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use serde_json::{Map, Value};
use tinymcp_bus::{
    names, ServerHostCall, ServerHostReply, ServerHttpResponseMetadata, ServerHttpSessionConfig,
    ServerHttpSessionRequest, ServerInput, ServerOperationRef, ServerOperationSnapshot,
    ServerOperationState, ServerSessionConfig,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::Config;
use tinybus::module::{ModuleCallError, ModuleClient};

use super::handler;

/// The host callback for a session: dispatches under the process default
/// context (the MCP server is a single-tenant host), held weakly so a
/// long-lived session never pins it.
fn host_dispatch() -> impl Fn(ServerHostCall) -> HostCallbackFuture + Send + Sync + 'static {
    let weak = crate::core::runtime::DispatchCtx::process_default().downgrade();
    move |call| {
        let ctx = crate::core::runtime::DispatchCtx::from_weak(weak.as_ref());
        Box::pin(handler::dispatch(ctx, call))
    }
}

const MODULE: &str = "tinymcp";
const POLL_INTERVAL: Duration = Duration::from_millis(5);
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);
static SHUTDOWN_REGISTERED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, thiserror::Error)]
pub(super) enum ServerError {
    #[error("MCP module {0}")]
    Module(ModuleCallError),
    #[error("MCP server operation was cancelled")]
    Cancelled,
    #[error("MCP server module returned an unknown operation state")]
    UnknownState,
    #[error("MCP server module operation failed: {0}")]
    Failed(String),
    #[error("MCP server module returned no response metadata")]
    MissingHttpMetadata,
    #[error("a previous MCP request is still pending for this session")]
    PendingRequest,
}

#[async_trait]
pub(super) trait ServerPort: Send + Sync {
    async fn open(&self, config: ServerSessionConfig) -> Result<String, ModuleCallError>;
    async fn open_http(&self, config: ServerHttpSessionConfig) -> Result<String, ModuleCallError>;
    async fn submit(
        &self,
        session: &str,
        headers: Map<String, Value>,
        input: ServerInput,
    ) -> Result<(), ModuleCallError>;
    async fn poll(&self, session: &str) -> Result<ServerOperationSnapshot, ModuleCallError>;
    async fn complete(
        &self,
        session: &str,
        callback: &str,
        reply: ServerHostReply,
    ) -> Result<(), ModuleCallError>;
    async fn cancel(&self, operation: ServerOperationRef) -> Result<(), ModuleCallError>;
    async fn close(&self, session: &str) -> Result<(), ModuleCallError>;
    async fn http_session(
        &self,
        request: ServerHttpSessionRequest,
    ) -> Result<ServerHttpResponseMetadata, ModuleCallError>;
}

type HostCallbackFuture = Pin<Box<dyn Future<Output = ServerHostReply> + Send + 'static>>;
type HostCallbackDispatcher = Arc<dyn Fn(ServerHostCall) -> HostCallbackFuture + Send + Sync>;

struct TinyMcpPort {
    client: ModuleClient,
}

#[async_trait]
impl ServerPort for TinyMcpPort {
    async fn open(&self, config: ServerSessionConfig) -> Result<String, ModuleCallError> {
        self.client
            .call(MODULE, names::methods::SERVER_OPEN, (config,))
            .await
    }

    async fn open_http(&self, config: ServerHttpSessionConfig) -> Result<String, ModuleCallError> {
        self.client
            .call(MODULE, names::methods::SERVER_OPEN_HTTP, (config,))
            .await
    }

    async fn submit(
        &self,
        session: &str,
        headers: Map<String, Value>,
        input: ServerInput,
    ) -> Result<(), ModuleCallError> {
        self.client
            .call(
                MODULE,
                names::methods::SERVER_SUBMIT,
                (session.to_owned(), headers, input),
            )
            .await
    }

    async fn poll(&self, session: &str) -> Result<ServerOperationSnapshot, ModuleCallError> {
        self.client
            .call(MODULE, names::methods::SERVER_POLL, (session.to_owned(),))
            .await
    }

    async fn complete(
        &self,
        session: &str,
        callback: &str,
        reply: ServerHostReply,
    ) -> Result<(), ModuleCallError> {
        self.client
            .call(
                MODULE,
                names::methods::SERVER_COMPLETE,
                (session.to_owned(), callback.to_owned(), reply),
            )
            .await
    }

    async fn cancel(&self, operation: ServerOperationRef) -> Result<(), ModuleCallError> {
        self.client
            .call(MODULE, names::methods::SERVER_CANCEL, (operation,))
            .await
    }

    async fn close(&self, session: &str) -> Result<(), ModuleCallError> {
        self.client
            .call(MODULE, names::methods::SERVER_CLOSE, (session.to_owned(),))
            .await
    }

    async fn http_session(
        &self,
        request: ServerHttpSessionRequest,
    ) -> Result<ServerHttpResponseMetadata, ModuleCallError> {
        self.client
            .call(MODULE, names::methods::SERVER_HTTP_SESSION, (request,))
            .await
    }
}

pub(super) struct ServerSession {
    port: Arc<dyn ServerPort>,
    callback_dispatch: HostCallbackDispatcher,
    id: String,
    next_operation: AtomicU64,
    operation: tokio::sync::Mutex<()>,
    terminal: CancellationToken,
    pending_input: tokio::sync::Mutex<Option<PendingInput>>,
    pending_callback: tokio::sync::Mutex<Option<PendingCallback>>,
}

#[derive(Clone)]
struct PendingInput {
    headers: Map<String, Value>,
    input: ServerInput,
}

enum PendingCallback {
    Running {
        id: String,
        task: tokio::task::JoinHandle<ServerHostReply>,
    },
    Complete {
        id: String,
        reply: ServerHostReply,
    },
}

impl ServerSession {
    pub(super) fn port_for_config(config: &Config) -> Arc<dyn ServerPort> {
        Self::register_shutdown_hook(config.clone());
        Arc::new(TinyMcpPort {
            client: crate::modules::client(config.clone()),
        })
    }

    fn register_shutdown_hook(config: Config) {
        if SHUTDOWN_REGISTERED.swap(true, Ordering::AcqRel) {
            return;
        }
        crate::core::shutdown::register(move || {
            let config = config.clone();
            async move {
                let client = crate::modules::client(config);
                if let Err(error) = client
                    .call::<usize>(MODULE, names::methods::SERVER_SHUTDOWN, ())
                    .await
                {
                    tracing::warn!(?error, "[mcp_server] module shutdown failed");
                }
                SHUTDOWN_REGISTERED.store(false, Ordering::Release);
            }
        });
    }

    pub(super) async fn open(config: &Config) -> Result<Self, ServerError> {
        Self::open_with_port(Self::port_for_config(config)).await
    }

    async fn open_with_port(port: Arc<dyn ServerPort>) -> Result<Self, ServerError> {
        Self::open_with_port_and_dispatch(port, host_dispatch()).await
    }

    pub(super) async fn open_with_port_and_dispatch<F, Fut>(
        port: Arc<dyn ServerPort>,
        dispatch: F,
    ) -> Result<Self, ServerError>
    where
        F: Fn(ServerHostCall) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ServerHostReply> + Send + 'static,
    {
        let callback_dispatch: HostCallbackDispatcher =
            Arc::new(move |call| Box::pin(dispatch(call)));
        let (info, source_type_prefix, resources) = handler::declarations();
        let sequence = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed).max(1);
        let id = port
            .open(ServerSessionConfig {
                session_id: Uuid::from_u128(u128::from(sequence)).to_string(),
                info,
                source_type_prefix,
                resources,
            })
            .await
            .map_err(ServerError::Module)?;
        Ok(Self {
            port,
            callback_dispatch,
            id,
            next_operation: AtomicU64::new(1),
            operation: tokio::sync::Mutex::new(()),
            terminal: CancellationToken::new(),
            pending_input: tokio::sync::Mutex::new(None),
            pending_callback: tokio::sync::Mutex::new(None),
        })
    }

    pub(super) async fn open_http(
        port: Arc<dyn ServerPort>,
        requested_session_id: Option<String>,
        known_session: bool,
        expected_protocol_version: Option<String>,
    ) -> Result<Self, ServerError> {
        let (info, source_type_prefix, resources) = handler::declarations();
        let sequence = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed).max(1);
        let id = port
            .open_http(ServerHttpSessionConfig {
                server: ServerSessionConfig {
                    session_id: Uuid::from_u128(u128::from(sequence)).to_string(),
                    info,
                    source_type_prefix,
                    resources,
                },
                requested_session_id,
                known_session,
                expected_protocol_version,
            })
            .await
            .map_err(ServerError::Module)?;
        Ok(Self {
            port,
            callback_dispatch: Arc::new(host_dispatch()),
            id,
            next_operation: AtomicU64::new(1),
            operation: tokio::sync::Mutex::new(()),
            terminal: CancellationToken::new(),
            pending_input: tokio::sync::Mutex::new(None),
            pending_callback: tokio::sync::Mutex::new(None),
        })
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) async fn submit(
        &self,
        headers: Map<String, Value>,
        line: String,
        cancellation: &CancellationToken,
    ) -> Result<ServerOperationSnapshot, ServerError> {
        let _operation = self.operation.lock().await;
        if self.terminal.is_cancelled() {
            return Err(ServerError::Cancelled);
        }
        let pending = {
            let mut current = self.pending_input.lock().await;
            match current.as_ref() {
                Some(pending) if pending.headers == headers && pending.input.line == line => {
                    pending.clone()
                }
                Some(_) => return Err(ServerError::PendingRequest),
                None => {
                    let input = ServerInput {
                        operation_id: Uuid::from_u128(u128::from(
                            self.next_operation.fetch_add(1, Ordering::Relaxed).max(1),
                        ))
                        .to_string(),
                        line,
                    };
                    let pending = PendingInput { headers, input };
                    *current = Some(pending.clone());
                    pending
                }
            }
        };
        tokio::select! {
            _ = cancellation.cancelled() => {
                self.cancel_input(&pending).await?;
                self.stop_callback().await?;
                return Err(ServerError::Cancelled);
            }
            _ = self.terminal.cancelled() => {
                self.cancel_input(&pending).await?;
                self.stop_callback().await?;
                return Err(ServerError::Cancelled);
            }
            result = self.port.submit(&self.id, pending.headers.clone(), pending.input.clone()) => {
                result.map_err(ServerError::Module)?;
            }
        }

        loop {
            let snapshot = tokio::select! {
                _ = cancellation.cancelled() => {
                    self.cancel_input(&pending).await?;
                    self.stop_callback().await?;
                    return Err(ServerError::Cancelled);
                }
                _ = self.terminal.cancelled() => {
                    self.cancel_input(&pending).await?;
                    self.stop_callback().await?;
                    return Err(ServerError::Cancelled);
                }
                result = self.port.poll(&self.id) => result.map_err(ServerError::Module)?,
            };
            match snapshot.state.clone() {
                ServerOperationState::Pending => tokio::time::sleep(POLL_INTERVAL).await,
                ServerOperationState::Callback { callback } => {
                    let reply = match self
                        .host_reply(callback.id.clone(), callback.call, cancellation)
                        .await
                    {
                        Ok(reply) => reply,
                        Err(ServerError::Cancelled) => {
                            self.cancel_input(&pending).await?;
                            self.stop_callback().await?;
                            return Err(ServerError::Cancelled);
                        }
                        Err(error) => return Err(error),
                    };
                    self.port
                        .complete(&self.id, &callback.id, reply)
                        .await
                        .map_err(ServerError::Module)?;
                }
                ServerOperationState::Complete { .. } => {
                    *self.pending_input.lock().await = None;
                    *self.pending_callback.lock().await = None;
                    return Ok(snapshot);
                }
                ServerOperationState::Cancelled => {
                    *self.pending_input.lock().await = None;
                    return Err(ServerError::Cancelled);
                }
                ServerOperationState::Failed { message } => {
                    *self.pending_input.lock().await = None;
                    return Err(ServerError::Failed(message));
                }
                ServerOperationState::Unknown => return Err(ServerError::UnknownState),
                _ => return Err(ServerError::UnknownState),
            }
        }
    }

    async fn host_reply(
        &self,
        callback_id: String,
        call: tinymcp_bus::ServerHostCall,
        cancellation: &CancellationToken,
    ) -> Result<ServerHostReply, ServerError> {
        let mut pending = self.pending_callback.lock().await;
        if let Some(PendingCallback::Complete { id, reply }) = pending.as_ref() {
            if id == &callback_id {
                return Ok(reply.clone());
            }
        }
        if !matches!(pending.as_ref(), Some(PendingCallback::Running { id, .. }) if id == &callback_id)
        {
            *pending = Some(PendingCallback::Running {
                id: callback_id.clone(),
                // The host callback runs under the process default, like
                // `host_dispatch`: the MCP server is a single-tenant host.
                task: crate::core::runtime::DispatchCtx::process_default()
                    .spawn((self.callback_dispatch)(call)),
            });
        }
        let result = match pending.as_mut() {
            Some(PendingCallback::Running { task, .. }) => {
                tokio::select! {
                    _ = cancellation.cancelled() => return Err(ServerError::Cancelled),
                    _ = self.terminal.cancelled() => return Err(ServerError::Cancelled),
                    result = task => result,
                }
            }
            Some(PendingCallback::Complete { reply, .. }) => return Ok(reply.clone()),
            None => return Err(ServerError::UnknownState),
        };
        let reply = result.unwrap_or_else(|_| ServerHostReply::Internal {
            message: "host callback stopped".to_owned(),
        });
        *pending = Some(PendingCallback::Complete {
            id: callback_id,
            reply: reply.clone(),
        });
        Ok(reply)
    }

    pub(super) async fn close(&self) -> Result<(), ServerError> {
        let local_cleanup = self.terminate_callbacks().await;
        let module_cleanup = self.port.close(&self.id).await.map_err(ServerError::Module);
        local_cleanup.and(module_cleanup)
    }

    /// Stop host callbacks and pending operations without closing the module's
    /// HTTP session, which the module already closed through DELETE.
    pub(super) async fn terminate_callbacks(&self) -> Result<(), ServerError> {
        self.terminal.cancel();
        let _operation = self.operation.lock().await;
        let callback_cleanup = self.stop_callback().await;
        *self.pending_input.lock().await = None;
        callback_cleanup
    }

    async fn cancel_input(&self, pending: &PendingInput) -> Result<(), ServerError> {
        self.port
            .cancel(ServerOperationRef {
                session_id: self.id.clone(),
                operation_id: pending.input.operation_id.clone(),
            })
            .await
            .map_err(ServerError::Module)?;
        *self.pending_input.lock().await = None;
        Ok(())
    }

    async fn stop_callback(&self) -> Result<(), ServerError> {
        let task = {
            let mut pending = self.pending_callback.lock().await;
            match pending.take() {
                Some(PendingCallback::Running { task, .. }) => Some(task),
                Some(PendingCallback::Complete { .. }) | None => None,
            }
        };
        if let Some(task) = task {
            task.abort();
            if task.await.is_err_and(|error| !error.is_cancelled()) {
                return Err(ServerError::Failed("host callback stopped".to_owned()));
            }
        }
        Ok(())
    }

    pub(super) async fn http_session(
        &self,
        request: ServerHttpSessionRequest,
    ) -> Result<ServerHttpResponseMetadata, ServerError> {
        self.port
            .http_session(request)
            .await
            .map_err(ServerError::Module)
    }
}

pub(super) fn headers_object(
    headers: impl Iterator<Item = (String, String)>,
) -> Map<String, Value> {
    headers
        .map(|(name, value)| (name, Value::String(value)))
        .collect()
}

#[cfg(test)]
#[path = "module_tests.rs"]
mod tests;
