//! Serialized access to one TinyMCP object owned by a workspace.
//!
//! `OpenConfigured` carries credentials, so the root lifecycle calls always
//! use TinyBus confidential delivery. The returned child object path remains
//! private to this adapter. Its state machine retains open and close receipts
//! before polling, which lets a later call resume cleanup after its caller was
//! cancelled or lost a reply.

use std::time::Duration;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tinymcp_bus::{
    config::{McpClientConfig, OpenConfiguredRequest, OpenConfiguredResponse, TransitionStatus},
    names,
};

use crate::{
    config::Config,
    modules::{client::ModuleCallError, client::ModuleClient},
};

const MODULE: &str = "tinymcp";
const TRANSITION_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSITION_POLL_INTERVAL: Duration = Duration::from_millis(20);
static CLIENTS: OnceLock<Mutex<HashMap<PathBuf, Arc<McpConfiguredClient>>>> = OnceLock::new();
static SHUTDOWN_REGISTERED: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct OpenObject {
    config: McpClientConfig,
    object_path: String,
}

#[derive(Clone)]
enum State {
    Closed,
    StartingOpen {
        config: McpClientConfig,
        previous: Option<OpenObject>,
    },
    Opening {
        config: McpClientConfig,
        previous: Option<OpenObject>,
        receipt: OpenConfiguredResponse,
    },
    Open(OpenObject),
    StartingClose(OpenObject),
    Closing {
        object: OpenObject,
        transition_id: String,
    },
}

#[async_trait]
pub(crate) trait McpModulePort: Send + Sync {
    async fn open_configured(
        &self,
        data_dir: &str,
        config: &McpClientConfig,
    ) -> Result<OpenConfiguredResponse, ModuleCallError>;

    async fn close(&self, data_dir: &str) -> Result<Option<String>, ModuleCallError>;

    async fn transition_status(
        &self,
        transition_id: &str,
    ) -> Result<TransitionStatus, ModuleCallError>;

    async fn call_object(
        &self,
        object_path: &str,
        member: &str,
        args: Value,
        confidential: bool,
    ) -> Result<Value, ModuleCallError>;
}

struct TinyMcpPort {
    root: ModuleClient,
}

#[async_trait]
impl McpModulePort for TinyMcpPort {
    async fn open_configured(
        &self,
        data_dir: &str,
        config: &McpClientConfig,
    ) -> Result<OpenConfiguredResponse, ModuleCallError> {
        self.root
            .call_confidential(
                MODULE,
                names::methods::OPEN_CONFIGURED,
                (OpenConfiguredRequest {
                    data_dir: data_dir.to_owned(),
                    client: config.clone(),
                },),
            )
            .await
    }

    async fn close(&self, data_dir: &str) -> Result<Option<String>, ModuleCallError> {
        self.root
            .call(MODULE, names::methods::CLOSE, (data_dir.to_owned(),))
            .await
    }

    async fn transition_status(
        &self,
        transition_id: &str,
    ) -> Result<TransitionStatus, ModuleCallError> {
        self.root
            .call(
                MODULE,
                names::methods::TRANSITION_STATUS,
                (transition_id.to_owned(),),
            )
            .await
    }

    async fn call_object(
        &self,
        object_path: &str,
        member: &str,
        args: Value,
        confidential: bool,
    ) -> Result<Value, ModuleCallError> {
        #[cfg(feature = "modules")]
        {
            let object = self.root.object_client(MODULE, object_path).await?;
            if confidential {
                object.call_confidential(member, args).await
            } else {
                object.call(member, args).await
            }
        }
        #[cfg(not(feature = "modules"))]
        {
            let _ = (object_path, member, args, confidential);
            Err(ModuleCallError::Unavailable)
        }
    }
}

/// One workspace's serialized TinyMCP lifecycle and private object proxy.
pub(crate) struct McpConfiguredClient {
    data_dir: String,
    port: Box<dyn McpModulePort>,
    state: tokio::sync::Mutex<State>,
}

impl McpConfiguredClient {
    /// The process-owned configured object for this workspace. Entries remain
    /// reachable until `close` confirms cleanup; workspace/config switching is
    /// serialized by the client itself.
    pub(crate) fn for_config(config: &Config) -> Result<Arc<Self>, ModuleCallError> {
        if !config.modules.enabled {
            ModuleClient::report_unavailable(MODULE);
            return Err(ModuleCallError::Unavailable);
        }

        let data_dir = crate::core::runtime::agent_scope_dir(config);
        let clients = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut clients = clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(client) = clients.get(&data_dir) {
            return Ok(Arc::clone(client));
        }
        let client = Arc::new(Self::new(&data_dir, config)?);
        clients.insert(data_dir, Arc::clone(&client));
        Ok(client)
    }

    /// Register this runtime's cleanup. The flag is reset by the drained hook,
    /// so a later embedded-runtime start can register a fresh one.
    pub(crate) fn register_shutdown_hook() {
        if SHUTDOWN_REGISTERED.swap(true, Ordering::AcqRel) {
            return;
        }
        crate::core::shutdown::register(|| async {
            let clients = CLIENTS
                .get()
                .map(|clients| {
                    clients
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .values()
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for client in clients {
                if let Err(error) = client.close().await {
                    tracing::warn!(?error, "[mcp] workspace cleanup is still pending");
                }
            }
            SHUTDOWN_REGISTERED.store(false, Ordering::Release);
        });
    }

    pub(crate) fn new(
        data_dir: &std::path::Path,
        config: &Config,
    ) -> Result<Self, ModuleCallError> {
        let data_dir = data_dir
            .to_str()
            .ok_or(ModuleCallError::IncompatibleContract)?
            .to_owned();
        Ok(Self::with_port(
            data_dir,
            Box::new(TinyMcpPort {
                root: ModuleClient::new(config.clone()),
            }),
        ))
    }

    #[cfg(test)]
    pub(crate) fn install_test_port(
        config: &Config,
        port: Box<dyn McpModulePort>,
    ) -> Result<Arc<Self>, ModuleCallError> {
        let data_dir = crate::core::runtime::agent_scope_dir(config);
        let client = Arc::new(Self::with_port(
            data_dir
                .to_str()
                .ok_or(ModuleCallError::IncompatibleContract)?
                .to_owned(),
            port,
        ));
        let clients = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
        clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(data_dir, Arc::clone(&client));
        Ok(client)
    }

    fn with_port(data_dir: String, port: Box<dyn McpModulePort>) -> Self {
        Self {
            data_dir,
            port,
            state: tokio::sync::Mutex::new(State::Closed),
        }
    }

    /// Calls one member on this workspace's private configured object.
    pub(crate) async fn call<R: DeserializeOwned>(
        &self,
        config: &McpClientConfig,
        member: &str,
        args: impl Serialize,
    ) -> Result<R, ModuleCallError> {
        self.call_inner(config, member, args, false).await
    }

    /// Calls a member that accepts credentials or other sensitive values.
    pub(crate) async fn call_confidential<R: DeserializeOwned>(
        &self,
        config: &McpClientConfig,
        member: &str,
        args: impl Serialize,
    ) -> Result<R, ModuleCallError> {
        self.call_inner(config, member, args, true).await
    }

    async fn call_inner<R: DeserializeOwned>(
        &self,
        config: &McpClientConfig,
        member: &str,
        args: impl Serialize,
        confidential: bool,
    ) -> Result<R, ModuleCallError> {
        let args = serde_json::to_value(args).map_err(|_| ModuleCallError::ModuleFault)?;
        let mut state = self.state.lock().await;
        let object = self.ensure_open(&mut state, config).await?;
        let reply = self
            .port
            .call_object(&object.object_path, member, args, confidential)
            .await?;
        serde_json::from_value(reply).map_err(|_| {
            ModuleClient::report_malformed_reply(MODULE);
            ModuleCallError::ModuleFault
        })
    }

    /// Closes this workspace and returns only after TinyMCP confirms that its
    /// transports and child object are gone. A cancelled/lost response is
    /// retried from the retained transition state on the next call.
    pub(crate) async fn close(&self) -> Result<(), ModuleCallError> {
        let mut state = self.state.lock().await;
        match state.clone() {
            State::Closed => return Ok(()),
            State::Open(object) => *state = State::StartingClose(object),
            State::StartingOpen { .. } | State::Opening { .. } => {
                self.finish_open(&mut state).await?;
                if let State::Open(object) = state.clone() {
                    *state = State::StartingClose(object);
                }
            }
            State::StartingClose(_) | State::Closing { .. } => {}
        }
        self.finish_close(&mut state).await
    }

    async fn ensure_open(
        &self,
        state: &mut State,
        requested: &McpClientConfig,
    ) -> Result<OpenObject, ModuleCallError> {
        loop {
            match state.clone() {
                State::Open(object) if object.config == *requested => return Ok(object),
                State::Open(object) => {
                    *state = State::StartingOpen {
                        config: requested.clone(),
                        previous: Some(object),
                    };
                }
                State::Closed => {
                    *state = State::StartingOpen {
                        config: requested.clone(),
                        previous: None,
                    };
                }
                State::StartingOpen { .. } | State::Opening { .. } => {
                    self.finish_open(state).await?;
                }
                State::StartingClose(_) | State::Closing { .. } => {
                    self.finish_close(state).await?;
                }
            }
        }
    }

    async fn finish_open(&self, state: &mut State) -> Result<(), ModuleCallError> {
        let mut retried_terminal_transition = false;
        loop {
            match state.clone() {
                State::StartingOpen { config, previous } => {
                    // Record intent before awaiting the confidential call. If
                    // the caller is cancelled after TinyMCP accepts it, retrying
                    // this same configuration recovers the idempotent receipt.
                    match self.port.open_configured(&self.data_dir, &config).await {
                        Ok(receipt) => {
                            *state = State::Opening {
                                config,
                                previous,
                                receipt,
                            };
                        }
                        Err(error) => {
                            *state = previous.map_or(State::Closed, State::Open);
                            return Err(error);
                        }
                    }
                }
                State::Opening {
                    config,
                    previous,
                    receipt,
                } => {
                    let status = self.poll_transition(&receipt.transition_id).await?;
                    match status {
                        TransitionStatus::Pending => continue,
                        TransitionStatus::Complete => {
                            *state = State::Open(OpenObject {
                                config,
                                object_path: receipt.object_path,
                            });
                            return Ok(());
                        }
                        TransitionStatus::Failed | TransitionStatus::Unknown => {
                            if retried_terminal_transition {
                                return Err(ModuleCallError::TransportFailed);
                            }
                            retried_terminal_transition = true;
                            *state = State::StartingOpen { config, previous };
                        }
                        _ => return Err(ModuleCallError::ModuleFault),
                    }
                }
                State::Open(_) | State::Closed => return Ok(()),
                State::StartingClose(_) | State::Closing { .. } => {
                    return Err(ModuleCallError::TransportFailed);
                }
            }
        }
    }

    async fn finish_close(&self, state: &mut State) -> Result<(), ModuleCallError> {
        let mut retried_terminal_transition = false;
        loop {
            match state.clone() {
                State::Closed => return Ok(()),
                State::StartingClose(object) => {
                    // The module returns the same pending receipt if a previous
                    // Close reply was lost. `None` means cleanup already ended.
                    match self.port.close(&self.data_dir).await {
                        Ok(Some(transition_id)) => {
                            *state = State::Closing {
                                object,
                                transition_id,
                            };
                        }
                        Ok(None) => {
                            *state = State::Closed;
                            return Ok(());
                        }
                        Err(error) => return Err(error),
                    }
                }
                State::Closing {
                    object,
                    transition_id,
                } => match self.poll_transition(&transition_id).await? {
                    TransitionStatus::Pending => continue,
                    TransitionStatus::Complete => {
                        *state = State::Closed;
                        return Ok(());
                    }
                    TransitionStatus::Failed | TransitionStatus::Unknown => {
                        if retried_terminal_transition {
                            return Err(ModuleCallError::TransportFailed);
                        }
                        retried_terminal_transition = true;
                        *state = State::StartingClose(object);
                    }
                    _ => return Err(ModuleCallError::ModuleFault),
                },
                State::Open(object) => *state = State::StartingClose(object),
                State::StartingOpen { .. } | State::Opening { .. } => {
                    return Err(ModuleCallError::TransportFailed);
                }
            }
        }
    }

    async fn poll_transition(
        &self,
        transition_id: &str,
    ) -> Result<TransitionStatus, ModuleCallError> {
        tokio::time::timeout(TRANSITION_TIMEOUT, async {
            loop {
                match self.port.transition_status(transition_id).await? {
                    TransitionStatus::Pending => tokio::time::sleep(TRANSITION_POLL_INTERVAL).await,
                    status => return Ok(status),
                }
            }
        })
        .await
        .map_err(|_| ModuleCallError::TransportFailed)?
    }
}

#[cfg(test)]
#[path = "module_client_tests.rs"]
mod tests;
