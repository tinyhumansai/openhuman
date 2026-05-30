use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use crate::openhuman::wallet::{self, PrepareTransferParams};
use async_trait::async_trait;
use serde_json::json;

pub struct WalletPrepareTransferTool;

impl WalletPrepareTransferTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for WalletPrepareTransferTool {
    fn name(&self) -> &str {
        "wallet_prepare_transfer"
    }

    fn description(&self) -> &str {
        "Prepare a cryptocurrency transfer. Returns a quote that must be confirmed before execution."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "chain": {
                    "type": "string",
                    "enum": ["evm", "btc", "solana", "tron"],
                    "description": "Blockchain network to use"
                },
                "toAddress": {
                    "type": "string",
                    "description": "Destination wallet address"
                },
                "amountRaw": {
                    "type": "string",
                    "description": "Transfer amount in the chain's smallest unit (e.g. wei for EVM)"
                },
                "assetSymbol": {
                    "type": "string",
                    "description": "Asset symbol (e.g. ETH, USDC). Defaults to the native asset."
                },
                "evmNetwork": {
                    "type": "string",
                    "enum": ["ethereum_mainnet", "base_mainnet", "arbitrum_one", "optimism_mainnet", "polygon_mainnet", "bsc_mainnet"],
                    "description": "Optional EVM network when chain='evm'. Defaults to ethereum_mainnet."
                }
            },
            "required": ["chain", "toAddress", "amountRaw"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        let params: PrepareTransferParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => {
                log::debug!("[wallet_prepare_transfer] invalid arguments: {e}");
                return Ok(ToolResult::error(format!("invalid arguments: {e}")));
            }
        };

        log::debug!(
            "[wallet_prepare_transfer] chain={:?} to={}…{} amount_len={}",
            params.chain,
            &params.to_address[..params.to_address.len().min(6)],
            &params.to_address[params.to_address.len().saturating_sub(4)..],
            params.amount_raw.len()
        );

        match wallet::prepare_transfer(params).await {
            Ok(outcome) => {
                let json_str = serde_json::to_string_pretty(&outcome.value)?;
                log::debug!("[wallet_prepare_transfer] success");
                Ok(ToolResult::success(json_str))
            }
            Err(e) => {
                log::warn!("[wallet_prepare_transfer] failed: {e}");
                Ok(ToolResult::error(e))
            }
        }
    }
}
