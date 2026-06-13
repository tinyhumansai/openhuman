//! Read-only agent tools for inspecting on-chain transactions by hash:
//! `wallet_tx_status`, `wallet_tx_receipt`, `wallet_lookup_tx`. Each shares the
//! same `{chain, hash, evmNetwork?}` input and delegates to the matching
//! `wallet::*` dispatcher.

use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use crate::openhuman::wallet::{self, EvmNetwork, WalletChain};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TxQueryArgs {
    chain: WalletChain,
    #[serde(default)]
    evm_network: Option<EvmNetwork>,
    hash: String,
}

fn tx_query_schema(verb: &str) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "chain": {
                "type": "string",
                "enum": ["evm", "btc", "solana", "tron"],
                "description": format!("Blockchain network whose transaction to {verb}")
            },
            "hash": {
                "type": "string",
                "description": "Transaction hash / signature / txid to query"
            },
            "evmNetwork": {
                "type": "string",
                "enum": ["ethereum_mainnet", "base_mainnet", "arbitrum_one", "optimism_mainnet", "polygon_mainnet", "bsc_mainnet"],
                "description": "Optional EVM network when chain='evm'. Defaults to ethereum_mainnet."
            }
        },
        "required": ["chain", "hash"],
        "additionalProperties": false
    })
}

fn parse_args(args: serde_json::Value) -> Result<TxQueryArgs, ToolResult> {
    serde_json::from_value(args).map_err(|e| ToolResult::error(format!("invalid arguments: {e}")))
}

pub struct WalletTxStatusTool;

impl WalletTxStatusTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for WalletTxStatusTool {
    fn name(&self) -> &str {
        "wallet_tx_status"
    }

    fn description(&self) -> &str {
        "Check the on-chain lifecycle state (pending / confirmed / failed / not_found) of a transaction by hash."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        tx_query_schema("check")
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
        let args = match parse_args(args) {
            Ok(a) => a,
            Err(err) => return Ok(err),
        };
        match wallet::tx_status(args.chain, args.evm_network, &args.hash).await {
            Ok(outcome) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &outcome.value,
            )?)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

pub struct WalletTxReceiptTool;

impl WalletTxReceiptTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for WalletTxReceiptTool {
    fn name(&self) -> &str {
        "wallet_tx_receipt"
    }

    fn description(&self) -> &str {
        "Fetch the receipt of a broadcast transaction (success flag, fee, block, gas used) by hash."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        tx_query_schema("fetch the receipt for")
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
        let args = match parse_args(args) {
            Ok(a) => a,
            Err(err) => return Ok(err),
        };
        match wallet::tx_receipt(args.chain, args.evm_network, &args.hash).await {
            Ok(outcome) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &outcome.value,
            )?)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

pub struct WalletLookupTxTool;

impl WalletLookupTxTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for WalletLookupTxTool {
    fn name(&self) -> &str {
        "wallet_lookup_tx"
    }

    fn description(&self) -> &str {
        "Look up the raw transaction payload by hash on the target chain."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        tx_query_schema("look up")
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
        let args = match parse_args(args) {
            Ok(a) => a,
            Err(err) => return Ok(err),
        };
        match wallet::lookup_tx(args.chain, args.evm_network, &args.hash).await {
            Ok(outcome) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &outcome.value,
            )?)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}
