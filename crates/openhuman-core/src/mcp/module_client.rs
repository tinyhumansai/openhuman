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
use serde_json::{Map, Value};
use tinymcp_bus::{
    agent_tools::{McpToolPresentation, PrepareAgentToolsRequest},
    config::{McpClientConfig, OpenConfiguredRequest, OpenConfiguredResponse, TransitionStatus},
    names, McpRemoteTool, ScrubStaticToolsRequest, ServerArgumentsValidation, SupervisorBatch,
    ValidateServerArgumentsRequest,
};

use super::host_cleanup::{CleanupAction, CleanupReservation, CLEANUP_RESERVATIONS};

use crate::{
    config::Config,
    modules::{client::ModuleCallError, client::ModuleClient},
};

const MODULE: &str = "tinymcp";
const TRANSITION_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSITION_POLL_INTERVAL: Duration = Duration::from_millis(20);
static CLIENTS: OnceLock<Mutex<HashMap<PathBuf, Arc<McpConfiguredClient>>>> = OnceLock::new();
static SHUTDOWN_REGISTERED: AtomicBool = AtomicBool::new(false);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
static AFTER_CONTEXT_CHECK: OnceLock<Mutex<Option<(PathBuf, tokio::sync::oneshot::Sender<()>)>>> =
    OnceLock::new();

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

    async fn prepare_agent_tools(
        &self,
        request: PrepareAgentToolsRequest,
    ) -> Result<Vec<McpToolPresentation>, ModuleCallError>;

    async fn validate_server_arguments(
        &self,
        request: ValidateServerArgumentsRequest,
    ) -> Result<ServerArgumentsValidation, ModuleCallError>;

    /// Normalizes model-supplied arguments through TinyMCP's root operation.
    async fn normalize_tool_arguments(
        &self,
        _arguments: Option<Value>,
    ) -> Result<Map<String, Value>, ModuleCallError> {
        Err(ModuleCallError::IncompatibleContract)
    }

    /// Applies a TinyMCP-owned text transformation through the root operation.
    async fn transform_text(
        &self,
        _request: tinymcp_bus::TransformTextRequest,
    ) -> Result<String, ModuleCallError> {
        Err(ModuleCallError::IncompatibleContract)
    }
}

#[path = "module_port.rs"]
mod module_port;
use module_port::TinyMcpPort;

#[path = "cached_cleanup.rs"]
mod cached_cleanup;

#[path = "agent_cleanup.rs"]
pub(crate) mod agent_cleanup;

/// One workspace's serialized TinyMCP lifecycle and private object proxy.
pub(crate) struct McpConfiguredClient {
    data_dir: String,
    port: Box<dyn McpModulePort>,
    state: tokio::sync::Mutex<State>,
    retiring: AtomicBool,
}

impl McpConfiguredClient {
    /// The process-owned configured object for this workspace. Entries remain
    /// reachable until `close` confirms cleanup; workspace/config switching is
    /// serialized by the client itself.
    pub(crate) fn for_config(config: &Config) -> Result<Arc<Self>, ModuleCallError> {
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            ModuleClient::report_unavailable(MODULE);
            return Err(ModuleCallError::Unavailable);
        }
        if !config.modules.enabled {
            ModuleClient::report_unavailable(MODULE);
            return Err(ModuleCallError::Unavailable);
        }
        if current_agent_context_is_removed() {
            ModuleClient::report_unavailable(MODULE);
            return Err(ModuleCallError::Unavailable);
        }
        let data_dir = crate::core::runtime::agent_scope_dir(config);
        if Self::cleanup_pending(&data_dir) {
            return Err(ModuleCallError::Unavailable);
        }
        #[cfg(test)]
        {
            let mut hook = AFTER_CONTEXT_CHECK
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if hook
                .as_ref()
                .is_some_and(|(expected, _)| expected == &data_dir)
            {
                if let Some((_, sender)) = hook.take() {
                    let _ = sender.send(());
                }
            }
        }
        let clients = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut clients = clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The cleanup reservation can be installed after the first check but
        // before this cache lock is acquired. Recheck before creating a client.
        if current_agent_context_is_removed() || Self::cleanup_pending(&data_dir) {
            ModuleClient::report_unavailable(MODULE);
            return Err(ModuleCallError::Unavailable);
        }
        if let Some(client) = clients.get(&data_dir) {
            if client.retiring.load(Ordering::Acquire) {
                return Err(ModuleCallError::Unavailable);
            }
            return Ok(Arc::clone(client));
        }
        let client = Arc::new(Self::new(&data_dir, config)?);
        clients.insert(data_dir, Arc::clone(&client));
        Ok(client)
    }

    /// A snapshot of configured workspaces currently retained by the host.
    /// The supervisor only drains open objects; it never creates a workspace.
    pub(crate) fn open_clients() -> Vec<(PathBuf, Arc<Self>)> {
        CLIENTS
            .get()
            .map(|clients| {
                clients
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .map(|(workspace, client)| (workspace.clone(), Arc::clone(client)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Drain observations from the module-owned supervisor for an open child.
    /// An object that is opening or closing is skipped and will be revisited
    /// on the next host poll.
    pub(crate) async fn drain_supervisor_events(
        &self,
        limit: usize,
    ) -> Result<Option<SupervisorBatch>, ModuleCallError> {
        let state = self.state.lock().await;
        if SHUTTING_DOWN.load(Ordering::Acquire) || self.retiring.load(Ordering::Acquire) {
            return Err(ModuleCallError::Unavailable);
        }
        let State::Open(object) = &*state else {
            return Ok(None);
        };
        let args = serde_json::to_value((limit,)).map_err(|_| ModuleCallError::ModuleFault)?;
        let reply = self
            .port
            .call_object(
                &object.object_path,
                names::methods::DRAIN_SUPERVISOR_EVENTS,
                args,
                false,
            )
            .await?;
        serde_json::from_value(reply)
            .map(Some)
            .map_err(|_| ModuleCallError::ModuleFault)
    }

    /// Register this runtime's cleanup. The flag is reset by the drained hook,
    /// so a later embedded-runtime start can register a fresh one.
    pub(crate) fn register_shutdown_hook() {
        if SHUTDOWN_REGISTERED.swap(true, Ordering::AcqRel) {
            return;
        }
        SHUTTING_DOWN.store(false, Ordering::Release);
        crate::core::shutdown::register(|| async {
            SHUTTING_DOWN.store(true, Ordering::Release);
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
                if client.retiring.load(Ordering::Acquire) {
                    let data_dir = PathBuf::from(&client.data_dir);
                    let reservation = Self::reservation_for(&data_dir);
                    Self::close_retired_client(data_dir, client, reservation).await;
                } else if let Err(error) = client.close().await {
                    tracing::warn!(?error, "[mcp] workspace cleanup is still pending");
                }
            }
            Self::retry_orphaned_cleanups();
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
            Box::new(TinyMcpPort::new(ModuleClient::new(config.clone()))),
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
            retiring: AtomicBool::new(false),
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

    /// Asks the module to prepare owner-derived names and rule facts for
    /// host-screened tools. This root operation needs no configured server
    /// object or credentials.
    pub(crate) async fn prepare_agent_tools(
        &self,
        request: PrepareAgentToolsRequest,
    ) -> Result<Vec<McpToolPresentation>, ModuleCallError> {
        let _accepted = self.state.lock().await;
        if SHUTTING_DOWN.load(Ordering::Acquire) || self.retiring.load(Ordering::Acquire) {
            return Err(ModuleCallError::Unavailable);
        }
        self.port.prepare_agent_tools(request).await
    }

    /// Scrubs configured credentials from static remote tool metadata inside
    /// TinyMCP before the host screens or presents that metadata.
    pub(crate) async fn scrub_static_tools(
        &self,
        config: &McpClientConfig,
        server: &str,
        tools: Vec<McpRemoteTool>,
    ) -> Result<Vec<McpRemoteTool>, ModuleCallError> {
        self.call_confidential(
            config,
            names::methods::STATIC_SCRUB_TOOLS,
            (ScrubStaticToolsRequest {
                server: server.to_owned(),
                tools,
            },),
        )
        .await
    }

    pub(crate) async fn validate_server_arguments(
        &self,
        request: ValidateServerArgumentsRequest,
    ) -> Result<ServerArgumentsValidation, ModuleCallError> {
        let _accepted = self.state.lock().await;
        if SHUTTING_DOWN.load(Ordering::Acquire) || self.retiring.load(Ordering::Acquire) {
            return Err(ModuleCallError::Unavailable);
        }
        self.port.validate_server_arguments(request).await
    }

    /// Normalizes host-supplied MCP arguments through TinyMCP's bounded
    /// implementation operation.
    pub(crate) async fn normalize_tool_arguments(
        &self,
        arguments: Option<Value>,
    ) -> Result<Map<String, Value>, ModuleCallError> {
        let _accepted = self.state.lock().await;
        if SHUTTING_DOWN.load(Ordering::Acquire) || self.retiring.load(Ordering::Acquire) {
            return Err(ModuleCallError::Unavailable);
        }
        self.port.normalize_tool_arguments(arguments).await
    }

    /// Normalizes a model-supplied MCP identifier in TinyMCP.
    pub(crate) async fn normalize_identifier(
        &self,
        value: &str,
    ) -> Result<String, ModuleCallError> {
        let _accepted = self.state.lock().await;
        if SHUTTING_DOWN.load(Ordering::Acquire) || self.retiring.load(Ordering::Acquire) {
            return Err(ModuleCallError::Unavailable);
        }
        self.port
            .transform_text(tinymcp_bus::TransformTextRequest {
                operation: tinymcp_bus::TextTransform::NormalizeIdentifier,
                text: value.to_owned(),
                max_bytes: tinymcp_bus::MAX_PROCESSING_BYTES,
            })
            .await
    }

    /// Loads and prepares configured-server tools from TinyMCP's persistent
    /// cache, listing a server only when no cached catalog exists. The host
    /// applies its prompt-injection filter before TinyMCP creates the stable
    /// model-facing declarations.
    pub(crate) async fn configured_tool_presentations(
        config: &Config,
    ) -> Result<Vec<McpToolPresentation>, ModuleCallError> {
        if !config.mcp_client.enabled {
            return Ok(Vec::new());
        }
        let client = Self::for_config(config)?;
        let client_config = crate::mcp::host::client_config(config);
        let mut sources = Vec::new();
        for declared in &config.mcp_client.servers {
            let server = &declared.server;
            if !server.enabled {
                continue;
            }
            let cached: Option<Vec<tinymcp_bus::McpTool>> = client
                .call(
                    &client_config,
                    names::methods::STATIC_CACHED_LIST_TOOLS,
                    (server.name.as_str(),),
                )
                .await?;
            let tools: Vec<McpRemoteTool> = match cached {
                Some(tools) => tools
                    .into_iter()
                    .map(|tool| McpRemoteTool {
                        name: tool.name,
                        title: None,
                        description: tool.description,
                        input_schema: tool.input_schema,
                        meta: None,
                    })
                    .collect(),
                None => {
                    client
                        .call(
                            &client_config,
                            names::methods::STATIC_LIST_TOOLS,
                            (server.name.as_str(),),
                        )
                        .await?
                }
            };
            let tools: Vec<McpRemoteTool> = client
                .scrub_static_tools(&client_config, &server.name, tools)
                .await?;
            let tools = crate::mcp::registry::tools_safe_for_agent(
                &server.name,
                tools
                    .into_iter()
                    .map(|tool| tinymcp_bus::McpTool {
                        name: tool.name,
                        description: tool.description.or(tool.title),
                        input_schema: tool.input_schema,
                    })
                    .collect(),
            );
            sources.push(tinymcp_bus::McpToolSourceSpec {
                server_id: server.name.clone(),
                label: server.name.clone(),
                family: server.name.clone(),
                display_name: server.name.clone(),
                tools,
                default_exposure: match declared.expose {
                    crate::config::McpToolExposure::Direct => tinymcp_bus::McpToolExposure::Direct,
                    crate::config::McpToolExposure::Deferred => {
                        tinymcp_bus::McpToolExposure::Deferred
                    }
                },
                direct_tools: declared.direct_tools.clone(),
            });
        }
        client
            .prepare_agent_tools(PrepareAgentToolsRequest { sources })
            .await
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
        if SHUTTING_DOWN.load(Ordering::Acquire) || self.retiring.load(Ordering::Acquire) {
            return Err(ModuleCallError::Unavailable);
        }
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

fn current_agent_context_is_removed() -> bool {
    let Some(context) = crate::core::runtime::CoreContext::current() else {
        return false;
    };
    context.session_agent().is_some()
        && context
            .host_overrides()
            .and_then(|overrides| overrides.approval_scope())
            .is_some_and(|scope| scope.is_closed())
}

#[cfg(test)]
#[path = "module_client_tests.rs"]
mod tests;

#[cfg(all(test, feature = "modules"))]
#[path = "module_client_owner_tests.rs"]
mod owner_tests;
