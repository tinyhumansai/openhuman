//! `web3_bridge` agent tools: quote a cross-chain bridge and execute it.

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use crate::openhuman::web3::store::execute_quote;
use crate::openhuman::web3::types::{BridgeQuoteParams, ExecuteQuoteParams};
use crate::openhuman::web3::{execute_tool_schema, ops, to_tool_result};

pub struct Web3BridgeQuoteTool;
pub struct Web3BridgeExecuteTool;

impl Web3BridgeQuoteTool {
    pub fn new() -> Self {
        Self
    }
}
impl Web3BridgeExecuteTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for Web3BridgeQuoteTool {
    fn name(&self) -> &str {
        "web3_bridge_quote"
    }
    fn description(&self) -> &str {
        "Prepare a cross-chain bridge via deBridge DLN. Returns a quote + quoteId to confirm with web3_bridge_execute. Source and destination chains must differ."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "srcChainId": {"type": "integer", "description": "Source deBridge chain id."},
                "srcChainTokenIn": {"type": "string", "description": "Source token address."},
                "srcChainTokenInAmount": {"type": "string", "description": "Source amount in the token's smallest unit."},
                "dstChainId": {"type": "integer", "description": "Destination deBridge chain id (must differ from source)."},
                "dstChainTokenOut": {"type": "string", "description": "Destination token address."},
                "dstChainTokenOutAmount": {"type": "string", "description": "Optional. 'auto' for market rate (default)."},
                "dstChainTokenOutRecipient": {"type": "string", "description": "Optional. Defaults to the wallet's own destination address."},
                "srcChainOrderAuthorityAddress": {"type": "string", "description": "Optional. Defaults to the wallet's source address."},
                "dstChainOrderAuthorityAddress": {"type": "string", "description": "Optional. Defaults to the wallet's destination address."}
            },
            "required": ["srcChainId", "srcChainTokenIn", "srcChainTokenInAmount", "dstChainId", "dstChainTokenOut"],
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
        let params: BridgeQuoteParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("invalid arguments: {e}"))),
        };
        Ok(to_tool_result(ops::quote_bridge(params).await))
    }
}

#[async_trait]
impl Tool for Web3BridgeExecuteTool {
    fn name(&self) -> &str {
        "web3_bridge_execute"
    }
    fn description(&self) -> &str {
        "Confirm and execute a prepared web3_bridge quote (signs + broadcasts the source-chain tx)."
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
