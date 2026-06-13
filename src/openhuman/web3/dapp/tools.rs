//! `web3_dapp` agent tools: prepare a generic EVM contract call and execute it.

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use crate::openhuman::web3::store::execute_quote;
use crate::openhuman::web3::types::{DappCallParams, ExecuteQuoteParams};
use crate::openhuman::web3::{execute_tool_schema, ops, to_tool_result};

pub struct Web3DappCallTool;
pub struct Web3DappExecuteTool;

impl Web3DappCallTool {
    pub fn new() -> Self {
        Self
    }
}
impl Web3DappExecuteTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for Web3DappCallTool {
    fn name(&self) -> &str {
        "web3_dapp_call"
    }
    fn description(&self) -> &str {
        "Prepare a generic EVM dapp contract call from pre-encoded calldata. Returns a quoteId to confirm with web3_dapp_execute."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "contractAddress": {"type": "string", "description": "Target contract address."},
                "calldata": {"type": "string", "description": "0x-prefixed hex calldata."},
                "valueRaw": {"type": "string", "description": "Optional native value (smallest unit). Defaults to '0'."},
                "evmNetwork": {
                    "type": "string",
                    "enum": ["ethereum_mainnet", "base_mainnet", "arbitrum_one", "optimism_mainnet", "polygon_mainnet", "bsc_mainnet"],
                    "description": "Optional EVM network. Defaults to ethereum_mainnet."
                }
            },
            "required": ["contractAddress", "calldata"],
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
        let params: DappCallParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("invalid arguments: {e}"))),
        };
        Ok(to_tool_result(ops::prepare_dapp_call(params).await))
    }
}

#[async_trait]
impl Tool for Web3DappExecuteTool {
    fn name(&self) -> &str {
        "web3_dapp_execute"
    }
    fn description(&self) -> &str {
        "Confirm and execute a prepared web3_dapp call (signs + broadcasts)."
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
