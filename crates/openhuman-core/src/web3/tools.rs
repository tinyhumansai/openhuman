//! Agent tool adapters that route wallet and Web3 operations through the
//! TinyWallet bus.
//!
//! Both families wrap a static [`ToolDeclaration`] from `tinywallet-bus`; the
//! declaration supplies the name, description, and parameter schema, and the
//! host dispatches by name to the matching bus operation.

use crate::core::runtime::DispatchCtx;
use async_trait::async_trait;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use tinytools::{PermissionLevel, Tool, ToolExposure, ToolResult};
use tinywallet_bus::tools::ToolDeclaration;
use tinywallet_bus::wire::{EvmNetwork, WalletChain};

/// Which shipped declaration list a tool came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    /// `wallet_*` tools from `wallet_tool_declarations`.
    Wallet,
    /// `web3_*` tools from `web3_tool_declarations`.
    Web3,
}

#[derive(Debug)]
pub(super) struct ModuleTool {
    declaration: &'static ToolDeclaration,
    family: Family,
}

impl ModuleTool {
    const fn wallet(declaration: &'static ToolDeclaration) -> Self {
        Self {
            declaration,
            family: Family::Wallet,
        }
    }

    pub(super) const fn web3(declaration: &'static ToolDeclaration) -> Self {
        Self {
            declaration,
            family: Family::Web3,
        }
    }

    /// Whether this tool signs and broadcasts (moves funds) rather than reads
    /// or quotes.
    fn signs_and_broadcasts(&self) -> bool {
        self.family == Family::Web3
            && matches!(
                self.declaration.name,
                "web3_swap_execute" | "web3_bridge_execute" | "web3_dapp_execute"
            )
    }
}

#[async_trait]
impl Tool for ModuleTool {
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

    /// The Web3 execute tools sign and broadcast a transaction; without this
    /// they inherited the trait default (`ReadOnly`, no external effect) and
    /// the approval gate treated a swap or bridge like a read.
    fn permission_level(&self) -> PermissionLevel {
        if self.signs_and_broadcasts() {
            PermissionLevel::Write
        } else {
            PermissionLevel::ReadOnly
        }
    }

    fn external_effect(&self) -> bool {
        self.signs_and_broadcasts()
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
}

impl ModuleTool {
    /// Runs the tool for a turn whose approval context is `chat`.
    pub(crate) async fn run(
        &self,
        ctx: &DispatchCtx,
        args: Value,
        chat: Option<&crate::security::approval::ApprovalChatContext>,
    ) -> anyhow::Result<ToolResult> {
        match self.family {
            Family::Wallet => self.run_wallet(ctx, args, chat).await,
            Family::Web3 => self.run_web3(ctx, args, chat).await,
        }
    }
}

// ---------------------------------------------------------------------------
// Wallet family
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TxQuery {
    chain: WalletChain,
    #[serde(default)]
    evm_network: Option<EvmNetwork>,
    hash: String,
}

fn outcome_value<T: Serialize>(
    outcome: Result<crate::core::Outcome<T>, String>,
) -> Result<Value, String> {
    outcome.and_then(|outcome| {
        serde_json::to_value(outcome.value)
            .map_err(|error| format!("failed to serialize wallet result: {error}"))
    })
}

impl ModuleTool {
    async fn run_wallet(
        &self,
        ctx: &DispatchCtx,
        args: Value,
        chat: Option<&crate::security::approval::ApprovalChatContext>,
    ) -> anyhow::Result<ToolResult> {
        let result = match self.name() {
            "wallet_status" => outcome_value(crate::web3::wallet::status(ctx).await),
            "wallet_chain_status" => outcome_value(crate::web3::wallet::chain_status(ctx).await),
            "wallet_prepare_transfer" => {
                let params = match serde_json::from_value(args) {
                    Ok(params) => params,
                    Err(error) => {
                        return Ok(ToolResult::error(format!("invalid arguments: {error}")))
                    }
                };
                outcome_value(
                    crate::web3::wallet::prepare_transfer(
                        ctx,
                        params,
                        crate::modules::web3_service::owner_of(chat),
                    )
                    .await,
                )
            }
            "wallet_tx_status" | "wallet_tx_receipt" | "wallet_lookup_tx" => {
                let query: TxQuery = match serde_json::from_value(args) {
                    Ok(query) => query,
                    Err(error) => {
                        return Ok(ToolResult::error(format!("invalid arguments: {error}")))
                    }
                };
                let result = match self.name() {
                    "wallet_tx_status" => outcome_value(
                        crate::web3::wallet::tx_status(
                            ctx,
                            query.chain,
                            query.evm_network,
                            &query.hash,
                        )
                        .await,
                    ),
                    "wallet_tx_receipt" => outcome_value(
                        crate::web3::wallet::tx_receipt(
                            ctx,
                            query.chain,
                            query.evm_network,
                            &query.hash,
                        )
                        .await,
                    ),
                    _ => outcome_value(
                        crate::web3::wallet::lookup_tx(
                            ctx,
                            query.chain,
                            query.evm_network,
                            &query.hash,
                        )
                        .await,
                    ),
                };
                result
            }
            _ => return Ok(ToolResult::error("unknown wallet tool")),
        };

        match result {
            Ok(value) => Ok(ToolResult::success(serde_json::to_string_pretty(&value)?)),
            Err(error) => Ok(ToolResult::error(error)),
        }
    }
}

// ---------------------------------------------------------------------------
// Web3 family (swaps / bridges / dapp calls)
// ---------------------------------------------------------------------------

impl ModuleTool {
    async fn run_web3(
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

// ---------------------------------------------------------------------------
// Registration entry points
// ---------------------------------------------------------------------------

/// The six established wallet tools, backed by OpenHuman's module adapters,
/// followed by the two policy-bound tools (`wallet_balances`,
/// `wallet_execute_prepared`) from [`super::wallet::automation_tools`].
pub fn wallet_agent_tools() -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = tinywallet_bus::tools::wallet_tool_declarations()
        .iter()
        .map(|declaration| {
            Box::new(ModuleTool::wallet(
                tinywallet_bus::tools::find_wallet_tool_declaration(declaration.name)
                    .expect("the declaration came from the shipped Wallet list"),
            )) as Box<dyn Tool>
        })
        .collect();
    tools.extend(super::wallet::automation_tools::automation_tools());
    tools
}

/// The swap / bridge / dapp tools from the shipped Web3 declaration list.
pub(super) fn web3_agent_tools() -> Vec<Box<dyn Tool>> {
    tinywallet_bus::tools::web3_tool_declarations()
        .iter()
        .map(|declaration| {
            Box::new(ModuleTool::web3(
                tinywallet_bus::tools::find_web3_tool_declaration(declaration.name)
                    .expect("the declaration came from the shipped Web3 list"),
            )) as Box<dyn Tool>
        })
        .collect()
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
