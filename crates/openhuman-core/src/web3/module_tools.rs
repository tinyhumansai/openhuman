//! Agent tool adapters that route Web3 operations through the TinyWallet bus.

use crate::core::runtime::DispatchCtx;
use async_trait::async_trait;
use serde_json::Value;
use tinytools::{Tool, ToolCallOptions, ToolExposure, ToolResult};
use tinywallet_bus::tools::ToolDeclaration;

#[derive(Debug)]
pub(super) struct Web3ModuleTool {
    declaration: &'static ToolDeclaration,
}

impl Web3ModuleTool {
    pub(super) const fn new(declaration: &'static ToolDeclaration) -> Self {
        Self { declaration }
    }
}

#[async_trait]
impl Tool for Web3ModuleTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        self.declaration.name
    }

    fn description(&self) -> &'static str {
        self.declaration.description
    }

    fn parameters_schema(&self) -> Value {
        self.declaration.parameters.clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, tinytools::ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let approval = crate::agent::tinyagents::host::run_context::approval_for_tool(context);
        let ctx = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context);
        self.run(&ctx, args, approval.chat.as_ref()).await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        _options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        self.execute(args).await
    }
}

impl Web3ModuleTool {
    /// Runs the tool for a turn whose approval context is `chat`.
    pub(crate) async fn run(
        &self,
        ctx: &DispatchCtx,
        args: Value,
        chat: Option<&crate::security::approval::ApprovalChatContext>,
    ) -> anyhow::Result<ToolResult> {
        let owner = crate::modules::web3_service::owner_of(chat);
        let config = match ctx.load_config().await {
            Ok(config) => config,
            Err(error) => return Ok(ToolResult::error(error)),
        };
        let result: anyhow::Result<Value> = match self.name() {
            "web3_swap_quote" => crate::modules::web3_service::quote(
                ctx,
                &config,
                owner.clone(),
                tinywallet_bus::names::methods::WEB3_SWAP_QUOTE,
                args,
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .and_then(|value| serde_json::to_value(value).map_err(Into::into)),
            "web3_bridge_quote" => crate::modules::web3_service::quote(
                ctx,
                &config,
                owner.clone(),
                tinywallet_bus::names::methods::WEB3_BRIDGE_QUOTE,
                args,
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .and_then(|value| serde_json::to_value(value).map_err(Into::into)),
            "web3_dapp_call" => crate::modules::web3_service::quote(
                ctx,
                &config,
                owner.clone(),
                tinywallet_bus::names::methods::WEB3_DAPP_QUOTE,
                args,
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .and_then(|value| serde_json::to_value(value).map_err(Into::into)),
            "web3_swap_routes" => crate::modules::web3_service::routes(ctx, &config)
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string())),
            "web3_swap_execute" | "web3_bridge_execute" | "web3_dapp_execute" => {
                match serde_json::from_value::<crate::modules::web3_service::ExecuteParams>(args) {
                    Ok(params) => crate::modules::web3_service::execute(
                        ctx,
                        &config,
                        owner,
                        params.quote_id,
                        params.confirmed,
                    )
                    .await
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
                    .and_then(|value| serde_json::to_value(value).map_err(Into::into)),
                    Err(error) => {
                        return Ok(ToolResult::error(format!("invalid arguments: {error}")));
                    }
                }
            }
            _ => return Ok(ToolResult::error("unknown Web3 tool")),
        };
        match result {
            Ok(value) => Ok(ToolResult::success(serde_json::to_string_pretty(&value)?)),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}
