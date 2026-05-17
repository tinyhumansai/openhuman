use serde::Deserialize;
use serde_json::{json, Value};

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::javascript;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ListToolsParams {}

#[derive(Debug, Deserialize)]
pub(crate) struct ExecuteToolParams {
    pub(crate) tool_name: String,
    #[serde(default)]
    pub(crate) args: Value,
    #[serde(default)]
    pub(crate) prefer_markdown: bool,
}

pub(crate) async fn list_tools_handler(
    _params: ListToolsParams,
) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let tools = javascript::list_tools(&config)?;
    let payload = json!({ "tools": tools });
    let log = vec![format!(
        "javascript.list_tools: count={}",
        payload["tools"]
            .as_array()
            .map(|tools| tools.len())
            .unwrap_or(0)
    )];
    RpcOutcome::new(payload, log).into_cli_compatible_json()
}

pub(crate) async fn execute_tool_handler(
    params: ExecuteToolParams,
) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let args = if params.args.is_null() {
        json!({})
    } else {
        params.args
    };
    let outcome =
        javascript::execute_tool(&config, &params.tool_name, args, params.prefer_markdown).await?;
    let payload = json!({
        "tool_name": outcome.tool_name,
        "elapsed_ms": outcome.elapsed_ms,
        "result": outcome.result,
    });
    let log = vec![format!(
        "javascript.execute_tool: tool_name={} elapsed_ms={}",
        payload["tool_name"].as_str().unwrap_or("<unknown>"),
        payload["elapsed_ms"].as_u64().unwrap_or(0)
    )];
    RpcOutcome::new(payload, log).into_cli_compatible_json()
}
