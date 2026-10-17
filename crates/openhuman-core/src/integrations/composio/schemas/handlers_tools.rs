//! Tool-discovery and execution handlers: `list_tools`, `execute`.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
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
        let tool = read_required_non_empty(&params, "tool")?;
        let arguments = read_optional::<Value>(&params, "arguments")?;
        let connection_id = read_optional::<String>(&params, "connection_id")?;
        to_json(ops::composio_execute(&config, &tool, arguments, connection_id.as_deref()).await?)
    })
}
