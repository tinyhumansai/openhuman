//! Host tools that invoke the TinyFlows Copilot implementation through its bus contract.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tinyflows_bus::copilot::{ToolCatalogRequest, ToolDescriptor};
use tinytools::{PermissionLevel, Tool, ToolResult};

use crate::config::Config;

pub(crate) async fn call<R, A>(config: &Config, member: &'static str, args: A) -> Result<R, String>
where
    R: DeserializeOwned,
    A: Serialize,
{
    let proxy = crate::modules::tinyflows::copilot_proxy(config).await?;
    proxy
        .call(member, args)
        .await
        .map_err(|error| error.wire_message())
}

/// The module's tool metadata plus the host-only node-kind overlay for calls.
pub(crate) struct CopilotToolCatalog {
    config: Arc<Config>,
    node_kinds: Vec<Value>,
    descriptors: HashMap<String, ToolDescriptor>,
}

impl CopilotToolCatalog {
    /// Load the tools the compiled module serves for this host configuration.
    pub(crate) fn load(config: Arc<Config>) -> Result<Self, String> {
        // Tool registration is synchronous, so fetch module-owned metadata
        // through the storage blocking bridge. This call carries the supplied
        // config and resolves no tenant data; tool calls take their authority
        // from ToolRunContext.
        let call_config = Arc::clone(&config);
        let node_kinds = crate::storage::block_on_anyhow(async move {
            crate::flows::all_node_kind_contracts(&call_config)
                .await
                .map_err(anyhow::Error::msg)
        })
        .map_err(|error| format!("could not load workflow node contracts: {error}"))
        .and_then(|contracts| serde_json::to_value(contracts).map_err(|error| error.to_string()))?
        .as_array()
        .cloned()
        .ok_or_else(|| "workflow node contracts were not an array".to_string())?;
        let request = ToolCatalogRequest {
            node_kinds: node_kinds.clone(),
        };
        let call_config = Arc::clone(&config);
        let descriptors: Vec<ToolDescriptor> = crate::storage::block_on_anyhow(async move {
            let proxy = crate::modules::tinyflows::copilot_proxy(&call_config)
                .await
                .map_err(anyhow::Error::msg)?;
            proxy
                .call("ToolCatalog", (request,))
                .await
                .map_err(|error| anyhow::anyhow!(error.wire_message()))
        })
        .map_err(|error| format!("could not load TinyFlows Copilot tools: {error}"))?;

        let mut by_name = HashMap::with_capacity(descriptors.len());
        for descriptor in descriptors {
            permission_level(&descriptor.permission).ok_or_else(|| {
                format!(
                    "TinyFlows Copilot tool `{}` has unsupported permission `{}`",
                    descriptor.name, descriptor.permission
                )
            })?;
            if by_name
                .insert(descriptor.name.clone(), descriptor)
                .is_some()
            {
                return Err("TinyFlows Copilot returned duplicate tool names".into());
            }
        }
        Ok(Self {
            config,
            node_kinds,
            descriptors: by_name,
        })
    }

    pub(crate) fn tool(&self, name: &str) -> Result<BusCopilotTool, String> {
        let descriptor = self
            .descriptors
            .get(name)
            .cloned()
            .ok_or_else(|| format!("TinyFlows Copilot does not provide `{name}`"))?;
        Ok(BusCopilotTool {
            config: Arc::clone(&self.config),
            descriptor,
            node_kinds: self.node_kinds.clone(),
        })
    }
}

/// A `tinytools::Tool` facade over one tool served by TinyFlows Copilot.
pub(crate) struct BusCopilotTool {
    config: Arc<Config>,
    descriptor: ToolDescriptor,
    node_kinds: Vec<Value>,
}

impl BusCopilotTool {
    #[cfg(test)]
    pub(crate) fn from_name(config: Arc<Config>, name: &str) -> Self {
        CopilotToolCatalog::load(config)
            .and_then(|catalog| catalog.tool(name))
            .unwrap_or_else(|error| panic!("could not prepare Copilot tool `{name}`: {error}"))
    }

    async fn invoke(
        &self,
        args: Value,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let host = super::builder_host::register(
            Arc::clone(&self.config),
            crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context),
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let proxy = crate::modules::tinyflows::copilot_invoke_proxy(&self.config)
            .await
            .map_err(anyhow::Error::msg)?;
        let request = tinyflows_bus::copilot::InvokeToolRequest {
            name: self.descriptor.name.clone(),
            arguments: args,
            host: host.address(),
            node_kinds: self.node_kinds.clone(),
        };
        let result: Value = proxy
            .call("InvokeTool", (request,))
            .await
            .map_err(|error| anyhow::anyhow!(error.wire_message()))?;
        serde_json::from_value(result)
            .map_err(|error| anyhow::anyhow!("invalid TinyFlows Copilot tool result: {error}"))
    }
}

#[async_trait::async_trait]
impl Tool for BusCopilotTool {
    fn name(&self) -> &str {
        &self.descriptor.name
    }

    fn description(&self) -> &str {
        &self.descriptor.description
    }

    fn parameters_schema(&self) -> Value {
        self.descriptor.parameters_schema.clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.invoke(args, None).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.invoke(args, context).await
    }

    fn permission_level(&self) -> PermissionLevel {
        permission_level(&self.descriptor.permission).unwrap_or(PermissionLevel::Dangerous)
    }

    fn external_effect(&self) -> bool {
        self.descriptor.external_effect
    }
}

fn permission_level(permission: &str) -> Option<PermissionLevel> {
    match permission {
        "None" => Some(PermissionLevel::None),
        "ReadOnly" => Some(PermissionLevel::ReadOnly),
        "Write" => Some(PermissionLevel::Write),
        "Execute" => Some(PermissionLevel::Execute),
        "Dangerous" => Some(PermissionLevel::Dangerous),
        _ => None,
    }
}

#[cfg(test)]
#[path = "copilot_bus_tests.rs"]
mod tests;
