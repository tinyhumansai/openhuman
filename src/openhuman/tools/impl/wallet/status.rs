use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use crate::openhuman::wallet;
use async_trait::async_trait;
use serde_json::json;

pub struct WalletStatusTool;

impl WalletStatusTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for WalletStatusTool {
    fn name(&self) -> &str {
        "wallet_status"
    }

    fn description(&self) -> &str {
        "Check wallet configuration status — whether the wallet is set up, which chains are configured, and available accounts."
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
        match wallet::status().await {
            Ok(outcome) => {
                let json_str = serde_json::to_string_pretty(&outcome.value)?;
                Ok(ToolResult::success(json_str))
            }
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}
