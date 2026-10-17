//! Serialized access to one TinyMCP object owned by a workspace.
//!
//! `OpenConfigured` carries credentials, so the root lifecycle calls always
//! use TinyBus confidential delivery. The returned child object path remains
//! private to this adapter. Its state machine retains open and close receipts
//! before polling, which lets a later call resume cleanup after its caller was
//! cancelled or lost a reply.

use std::time::Duration;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};
use tinymcp_bus::{
    agent_tools::{McpToolPresentation, PrepareAgentToolsRequest},
    client_lifecycle::{CleanupAction, ConfiguredLifecycle, Fault, Poll, Retire, Step},
    config::{McpClientConfig, OpenConfiguredRequest, OpenConfiguredResponse, TransitionStatus},
    names, McpRemoteTool, ScrubStaticToolsRequest, ServerArgumentsValidation, SupervisorBatch,
    ValidateServerArgumentsRequest,
};

use crate::config::Config;
use tinybus::module::{ModuleCallError, ModuleClient};

const MODULE: &str = "tinymcp";
const TRANSITION_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSITION_POLL_INTERVAL: Duration = Duration::from_millis(20);
static SHUTDOWN_REGISTERED: AtomicBool = AtomicBool::new(false);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// TinyMCP admits one directory lifecycle transition at a time across every
/// workspace it serves, and answers a second concurrent open or close with
/// `InvalidArgument: a directory lifecycle transition is pending`. The module
/// does not queue them, so the host serializes them here: the gate is held from
/// the first lifecycle call until the transition has been polled to a terminal
/// status (the module releases its own gate only then).
static LIFECYCLE_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

    async fn prepare_agent_tools(
        &self,
        request: PrepareAgentToolsRequest,
    ) -> Result<Vec<McpToolPresentation>, ModuleCallError> {
        self.root
            .call(MODULE, names::methods::PREPARE_AGENT_TOOLS, (request,))
            .await
    }

    async fn validate_server_arguments(
        &self,
        request: ValidateServerArgumentsRequest,
    ) -> Result<ServerArgumentsValidation, ModuleCallError> {
        self.root
            .call_confidential(
                MODULE,
                names::methods::VALIDATE_SERVER_ARGUMENTS,
                (request,),
            )
            .await
    }

    async fn normalize_tool_arguments(
        &self,
        arguments: Option<Value>,
    ) -> Result<Map<String, Value>, ModuleCallError> {
        self.root
            .call_confidential(
                MODULE,
                names::methods::NORMALIZE_TOOL_ARGUMENTS,
                (arguments,),
            )
            .await
    }

    async fn transform_text(
        &self,
        request: tinymcp_bus::TransformTextRequest,
    ) -> Result<String, ModuleCallError> {
        self.root
            .call_confidential(MODULE, names::methods::TRANSFORM_TEXT, (request,))
            .await
    }
}

/// One workspace's serialized TinyMCP lifecycle and private object proxy.
pub(crate) struct McpConfiguredClient {
    data_dir: String,
    port: Box<dyn McpModulePort>,
    state: tokio::sync::Mutex<ConfiguredLifecycle>,
    retiring: AtomicBool,
}

mod lifecycle;

impl McpConfiguredClient {
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
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            return Err(ModuleCallError::Unavailable);
        }
        if self.retiring.load(Ordering::Acquire) {
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
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            return Err(ModuleCallError::Unavailable);
        }
        if self.retiring.load(Ordering::Acquire) {
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
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            return Err(ModuleCallError::Unavailable);
        }
        if self.retiring.load(Ordering::Acquire) {
            return Err(ModuleCallError::Unavailable);
        }
        self.port.normalize_tool_arguments(arguments).await
    }

    /// Normalizes a model-supplied MCP identifier in TinyMCP.
    pub(crate) async fn normalize_identifier(
        &self,
        value: &str,
    ) -> Result<String, ModuleCallError> {
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            return Err(ModuleCallError::Unavailable);
        }
        if self.retiring.load(Ordering::Acquire) {
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
                        meta: None,
                    })
                    .collect(),
            );
            sources.push(tinymcp_bus::McpToolSourceSpec {
                server_id: server.name.clone(),
                label: server.name.clone(),
                family: server.name.clone(),
                display_name: server.name.clone(),
                tools,
                default_exposure: declared.expose,
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
            crate::modules::report::malformed_reply(MODULE);
            ModuleCallError::ModuleFault
        })
    }

    /// Closes this workspace and returns only after TinyMCP confirms that its
    /// transports and child object are gone. A cancelled/lost response is
    /// retried from the retained transition state on the next call.
    pub(crate) async fn close(&self) -> Result<(), ModuleCallError> {
        let mut state = self.state.lock().await;
        self.drive(&mut state, None).await.map(|_| ())
    }

    async fn ensure_open(
        &self,
        state: &mut ConfiguredLifecycle,
        requested: &McpClientConfig,
    ) -> Result<tinymcp_bus::client_lifecycle::OpenObject, ModuleCallError> {
        self.drive(state, Some(requested))
            .await?
            .ok_or(ModuleCallError::ModuleFault)
    }

    /// Performs the bus call each `Step` of the module-owned lifecycle names
    /// and feeds the reply back, until the target is open (`Some`) or closed.
    async fn drive(
        &self,
        state: &mut ConfiguredLifecycle,
        requested: Option<&McpClientConfig>,
    ) -> Result<Option<tinymcp_bus::client_lifecycle::OpenObject>, ModuleCallError> {
        let mut gate = None;
        loop {
            let step = match requested {
                Some(config) => state.open_step(config),
                None => state.close_step(),
            };
            if gate.is_none() && !matches!(step, Step::Ready(_) | Step::Closed) {
                log::debug!(
                    "[mcp] waiting for the module lifecycle gate data_dir_len={}",
                    self.data_dir.len()
                );
                gate = Some(LIFECYCLE_GATE.lock().await);
            }
            match step {
                Step::Ready(object) => return Ok(Some(object)),
                Step::Closed => return Ok(None),
                Step::CallOpen(config) => {
                    // The lifecycle recorded intent before this confidential
                    // call; retrying the same configuration recovers the
                    // module's idempotent receipt after a cancelled caller.
                    // Servers and audit rows from before the storage contract
                    // move into the module's storage before it reads them.
                    // Best effort: a failure is retried at the next open.
                    crate::modules::import_legacy_mcp(
                        &crate::core::runtime::DispatchCtx::ambient(),
                        std::path::Path::new(&self.data_dir),
                    )
                    .await;
                    let reply = self.port.open_configured(&self.data_dir, &config).await;
                    state.open_replied(reply.clone().map_err(|_| ()));
                    reply?;
                }
                Step::PollOpen(id) => {
                    let status = self.poll_transition(&id).await?;
                    Self::settle(state.open_polled(status))?;
                }
                Step::CallClose => {
                    let reply = self.port.close(&self.data_dir).await;
                    state.close_replied(reply.clone().map_err(|_| ()));
                    reply?;
                }
                Step::PollClose(id) => {
                    let status = self.poll_transition(&id).await?;
                    Self::settle(state.close_polled(status))?;
                }
                _ => return Err(ModuleCallError::ModuleFault),
            }
        }
    }

    fn settle(poll: Poll) -> Result<(), ModuleCallError> {
        match poll {
            Poll::Fault(Fault::Transport) => Err(ModuleCallError::TransportFailed),
            Poll::Fault(_) => Err(ModuleCallError::ModuleFault),
            _ => Ok(()),
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

impl Retire for McpConfiguredClient {
    fn mark_retiring(&self) {
        self.retiring.store(true, Ordering::Release);
    }

    fn is_retiring(&self) -> bool {
        self.retiring.load(Ordering::Acquire)
    }
}
