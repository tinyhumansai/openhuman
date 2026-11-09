//! Shared test adapters for the memory tool's native facade.

use super::{is_write_action, CallFacts, MemoryTool};
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use serde_json::Value;
use std::sync::Arc;
use tinytools::ToolResult;

pub(super) async fn run_action(
    ctx: &DispatchCtx,
    config: &Config,
    args: &Value,
    facts: &CallFacts,
) -> ToolResult {
    let result = MemoryTool::new(Arc::new(config.clone()))
        .call_with_facts(args.clone(), ctx.clone(), facts.clone(), None)
        .await;
    if is_write_action(args) && !result.is_error {
        crate::memory::tool_writes::tests::drain_for(config, &facts.reach.at).await;
    }
    result
}
