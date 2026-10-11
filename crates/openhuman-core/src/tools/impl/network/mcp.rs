//! The generic MCP bridge tools (`mcp_list_servers`, `mcp_list_tools`,
//! `mcp_call_tool`).
//!
//! The tools, their schemas and the secret scrubber over their output live in
//! `tinymcp::tools`; what stays here is the one piece of host policy: the act
//! gate `mcp_call_tool` asks before it sends anything.

use std::sync::Arc;

use crate::mcp::config_servers::McpServerRegistry;
use crate::security::{SecurityPolicy, ToolOperation};

/// `mcp_call_tool` over `registry`, refusing any call `security` does not
/// allow as an acting operation.
pub fn mcp_call_tool(
    registry: Arc<McpServerRegistry>,
    security: Arc<SecurityPolicy>,
) -> tinymcp::tools::McpCallTool {
    tinymcp::tools::McpCallTool::new(
        registry,
        Arc::new(move |name| {
            security
                .enforce_tool_operation(ToolOperation::Act, name)
                .map_err(|err| anyhow::anyhow!(err))
        }),
    )
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
