//! `web3_swap` agent tools: quote a single-chain swap, execute a prepared
//! quote, and list supported routes. All delegate to [`super::super::ops`] /
//! [`super::super::store`]; signing happens in the wallet.

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use crate::openhuman::web3::store::execute_quote;
use crate::openhuman::web3::types::{ExecuteQuoteParams, SwapQuoteParams};
use crate::openhuman::web3::{execute_tool_schema, ops, to_tool_result};

pub struct Web3SwapQuoteTool;
pub struct Web3SwapExecuteTool;
pub struct Web3SwapRoutesTool;

impl Web3SwapQuoteTool {
    pub fn new() -> Self {
        Self
    }
}
impl Web3SwapExecuteTool {
    pub fn new() -> Self {
        Self
    }
}
impl Web3SwapRoutesTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for Web3SwapQuoteTool {
    fn name(&self) -> &str {
        "web3_swap_quote"
    }
    fn description(&self) -> &str {
        "Prepare a single-chain crypto swap via deBridge. Returns a quote + quoteId to confirm with web3_swap_execute. For cross-chain swaps use web3_bridge_quote."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "chainId": {"type": "integer", "description": "deBridge chain id (1 ETH, 56 BNB, 137 Polygon, 8453 Base, 42161 Arbitrum, 10 Optimism, 7565164 Solana)."},
                "tokenIn": {"type": "string", "description": "Input token address (zero address for native)."},
                "tokenInAmount": {"type": "string", "description": "Input amount in the token's smallest unit."},
                "tokenOut": {"type": "string", "description": "Output token address."},
                "tokenOutRecipient": {"type": "string", "description": "Optional. Defaults to the wallet's own address."},
                "senderAddress": {"type": "string", "description": "Optional. Defaults to the wallet's own address."},
                "slippage": {"type": "string", "description": "Optional slippage percent or 'auto' (default 'auto')."}
            },
            "required": ["chainId", "tokenIn", "tokenInAmount", "tokenOut"],
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
        let params: SwapQuoteParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("invalid arguments: {e}"))),
        };
        Ok(to_tool_result(ops::quote_swap(params).await))
    }
}

#[async_trait]
impl Tool for Web3SwapExecuteTool {
    fn name(&self) -> &str {
        "web3_swap_execute"
    }
    fn description(&self) -> &str {
        "Confirm and execute a prepared web3_swap quote (signs + broadcasts)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        execute_tool_schema()
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
        let params: ExecuteQuoteParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("invalid arguments: {e}"))),
        };
        Ok(to_tool_result(execute_quote(params).await))
    }
}

#[async_trait]
impl Tool for Web3SwapRoutesTool {
    fn name(&self) -> &str {
        "web3_swap_routes"
    }
    fn description(&self) -> &str {
        "List the chains deBridge can swap/bridge between."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }
    async fn execute_with_options(
        &self,
        _args: serde_json::Value,
        _options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        Ok(to_tool_result(ops::routes().await))
    }
}
