use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use crate::openhuman::wallet;
use async_trait::async_trait;
use serde_json::json;

pub struct WalletChainStatusTool;

impl WalletChainStatusTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for WalletChainStatusTool {
    fn name(&self) -> &str {
        "wallet_chain_status"
    }

    fn description(&self) -> &str {
        "List blockchain chain readiness — which chains have a configured account and RPC provider."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
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
        match wallet::chain_status().await {
            Ok(outcome) => {
                let json_str = serde_json::to_string_pretty(&outcome.value)?;
                Ok(ToolResult::success(json_str))
            }
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}
