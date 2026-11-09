//! Tool-discovery and execution handlers: `list_tools`, `execute`.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::config::Config;
use crate::core::all::{ControllerFuture, ControllerTask};
use crate::integrations::composio::ops;

use super::util::{read_optional, read_required_non_empty, to_json};

pub(super) fn handle_list_tools(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let toolkits = read_optional::<Vec<String>>(&params, "toolkits")?;
        let tags = read_optional::<Vec<String>>(&params, "tags")?;
        to_json(ops::composio_list_tools(&ctx, &config, toolkits, tags).await?)
    })
}

pub(super) fn handle_execute(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let parsed = parse_execute(params)?;
        run_execute(ctx, config, parsed).await
    })
}

#[derive(Debug, PartialEq)]
pub(super) struct ExecuteParams {
    tool: String,
    arguments: Option<Value>,
    connection_id: Option<String>,
}

/// Preserve the RPC readers' validation and error precedence for execute.
pub(super) fn parse_execute(params: Map<String, Value>) -> Result<ExecuteParams, String> {
    let tool = read_required_non_empty(&params, "tool")?;
    let arguments = read_optional::<Value>(&params, "arguments")?;
    let connection_id = read_optional::<String>(&params, "connection_id")?;
    Ok(ExecuteParams {
        tool,
        arguments,
        connection_id,
    })
}

pub(super) fn load_execute_config(ctx: DispatchCtx) -> ControllerTask<Config> {
    Box::pin(async move { ctx.load_config().await })
}

pub(super) fn run_execute(
    _ctx: DispatchCtx,
    config: Config,
    params: ExecuteParams,
) -> ControllerFuture {
    Box::pin(async move {
        to_json(
            ops::composio_execute(
                &config,
                &params.tool,
                params.arguments,
                params.connection_id.as_deref(),
            )
            .await?,
        )
    })
}

#[cfg(test)]
#[path = "handlers_tools_tests.rs"]
mod tests;
