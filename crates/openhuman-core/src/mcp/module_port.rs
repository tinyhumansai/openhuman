//! Bus adapter pinned to the module instance that starts its lifecycle.
use super::*;
#[cfg(feature = "modules")]
use tinybus::module::ModuleObjectClient;

pub(super) struct TinyMcpPort {
    root: ModuleClient,
    /// The host config the stdio sandbox's toolchain grants resolve against.
    host_config: Option<crate::config::Config>,
    #[cfg(feature = "modules")]
    pinned: tokio::sync::OnceCell<ModuleObjectClient>,
}

impl TinyMcpPort {
    pub(super) fn new(root: ModuleClient) -> Self {
        Self {
            root,
            host_config: None,
            #[cfg(feature = "modules")]
            pinned: tokio::sync::OnceCell::new(),
        }
    }

    /// Resolves the stdio sandbox's toolchain grants against `config` when the
    /// configured client is opened.
    pub(super) fn with_host_config(mut self, config: crate::config::Config) -> Self {
        self.host_config = Some(config);
        self
    }

    #[cfg(feature = "modules")]
    async fn pinned(&self) -> Result<&ModuleObjectClient, ModuleCallError> {
        self.pinned
            .get_or_try_init(|| async {
                self.root
                    .object_client(MODULE, names::OBJECT_PATH)
                    .await?
                    .pin_owner()
                    .await
            })
            .await
    }

    async fn root_call<R: DeserializeOwned>(
        &self,
        member: &str,
        args: impl Serialize,
        confidential: bool,
    ) -> Result<R, ModuleCallError> {
        #[cfg(feature = "modules")]
        {
            let root = self.pinned().await?;
            if confidential {
                root.call_confidential(member, args).await
            } else {
                root.call(member, args).await
            }
        }
        #[cfg(not(feature = "modules"))]
        {
            if confidential {
                self.root.call_confidential(MODULE, member, args).await
            } else {
                self.root.call(MODULE, member, args).await
            }
        }
    }
}

#[async_trait]
impl McpModulePort for TinyMcpPort {
    async fn open_configured(
        &self,
        data_dir: &str,
        config: &McpClientConfig,
    ) -> Result<OpenConfiguredResponse, ModuleCallError> {
        let mut client = config.clone();
        if let (Some(host), Some(sandbox)) = (&self.host_config, client.stdio_sandbox.as_mut()) {
            crate::mcp::host_sandbox::with_toolchain_grants(host, sandbox).await;
        }
        self.root_call(
            names::methods::OPEN_CONFIGURED,
            (OpenConfiguredRequest {
                data_dir: data_dir.to_owned(),
                client,
            },),
            true,
        )
        .await
    }

    async fn close(&self, data_dir: &str) -> Result<Option<String>, ModuleCallError> {
        self.root_call(names::methods::CLOSE, (data_dir.to_owned(),), false)
            .await
    }

    async fn transition_status(
        &self,
        transition_id: &str,
    ) -> Result<TransitionStatus, ModuleCallError> {
        self.root_call(
            names::methods::TRANSITION_STATUS,
            (transition_id.to_owned(),),
            false,
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
            let object = self.pinned().await?.object_at(object_path)?;
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

    async fn call_object_if_ready(
        &self,
        object_path: &str,
        member: &str,
        args: Value,
        confidential: bool,
    ) -> Result<Value, ModuleCallError> {
        #[cfg(feature = "modules")]
        {
            let owner = self.pinned.get().ok_or(ModuleCallError::Unavailable)?;
            let object = owner.object_at(object_path)?;
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
        self.root_call(names::methods::PREPARE_AGENT_TOOLS, (request,), false)
            .await
    }

    async fn validate_server_arguments(
        &self,
        request: ValidateServerArgumentsRequest,
    ) -> Result<ServerArgumentsValidation, ModuleCallError> {
        self.root_call(names::methods::VALIDATE_SERVER_ARGUMENTS, (request,), true)
            .await
    }

    async fn normalize_tool_arguments(
        &self,
        arguments: Option<Value>,
    ) -> Result<Map<String, Value>, ModuleCallError> {
        self.root_call(names::methods::NORMALIZE_TOOL_ARGUMENTS, (arguments,), true)
            .await
    }

    async fn transform_text(
        &self,
        request: tinymcp_bus::TransformTextRequest,
    ) -> Result<String, ModuleCallError> {
        self.root_call(names::methods::TRANSFORM_TEXT, (request,), true)
            .await
    }
}

#[cfg(test)]
#[path = "module_port_tests.rs"]
mod tests;
