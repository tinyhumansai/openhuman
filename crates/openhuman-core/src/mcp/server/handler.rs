//! OpenHuman's [`McpServerHandler`]: the host policy behind the MCP surface.
//!
//! `tinymcp::server` owns the protocol and both transports. This is what it
//! asks OpenHuman: who the server is, which tools exist (`tools::specs`, gated
//! by the loaded config), what calling one does (`tools::dispatch`, under
//! `SecurityPolicy` and the write-audit pipeline), and which prompt resources
//! are served (`resources`).
//!
//! It also applies the one piece of transport state OpenHuman cares about:
//! the subagent delegation depth that rides the loopback HTTP hop as
//! [`subagent_depth::HEADER_SUBAGENT_DEPTH`]. Every tool call runs inside
//! [`subagent_depth::scope`] at the depth its request carried (0 when absent,
//! as on stdio), so `agent.run_subagent` bounds recursion per chain.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::{Map, Value};
use tinymcp::{
    McpServerHandler, RequestContext, ResourceSpec, ServerInfo, ServerToolSpec, ToolCallError,
};

use super::{resources, subagent_depth, tools};

/// The base of every MCP session's `source_type`: `mcp`, or
/// `mcp:<client-name>` once `initialize` names the client. Audit rows and
/// memory writes are attributed with it.
const SOURCE_TYPE_PREFIX: &str = "mcp";

/// The `instructions` OpenHuman's `initialize` result carries.
const INSTRUCTIONS: &str = "OpenHuman MCP exposes first-level core integration: inspect the live tool catalog with core.list_tools or core.tool_instructions, inspect subagents with agent.list_subagents, run a standalone subagent with agent.run_subagent, use web_search or web_answer for live web lookups (and searxng_search when self-hosted search is enabled), and use memory.recall (answer with citations), memory.fetch or memory.list for local memory reads, and memory.learn or memory.forget to change it.";

/// The handler every OpenHuman MCP transport serves.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OpenHumanMcpHandler;

/// The handler, shareable across a transport's connections.
pub(crate) fn handler() -> Arc<dyn McpServerHandler> {
    Arc::new(OpenHumanMcpHandler)
}

impl McpServerHandler for OpenHumanMcpHandler {
    fn server_info(&self) -> ServerInfo {
        ServerInfo::new("openhuman-core", env!("CARGO_PKG_VERSION")).with_instructions(INSTRUCTIONS)
    }

    fn source_type_prefix(&self) -> &str {
        SOURCE_TYPE_PREFIX
    }

    fn list_tools<'a>(&'a self, _ctx: &'a RequestContext) -> BoxFuture<'a, Vec<ServerToolSpec>> {
        Box::pin(async {
            tools::list_tool_specs()
                .await
                .iter()
                .map(tools::server_tool_spec)
                .collect()
        })
    }

    fn call_tool<'a>(
        &'a self,
        ctx: &'a RequestContext,
        name: &'a str,
        arguments: Map<String, Value>,
    ) -> BoxFuture<'a, Result<Value, ToolCallError>> {
        let depth = subagent_depth::parse_header(ctx.header(subagent_depth::HEADER_SUBAGENT_DEPTH));
        log::trace!(
            "[mcp_server] tools/call tool={name} client_source_type={} chain_depth={depth}",
            ctx.source_type()
        );
        // Boxed by the trait: a subagent run is a whole agent turn, and its
        // future must not sit on the transport's stack.
        Box::pin(subagent_depth::scope(
            depth,
            tools::call_tool(name, Value::Object(arguments), ctx.source_type()),
        ))
    }

    fn list_resources(&self) -> Vec<ResourceSpec> {
        resources::resource_specs()
    }

    fn read_resource<'a>(&'a self, uri: &'a str) -> BoxFuture<'a, Result<Value, ToolCallError>> {
        Box::pin(async move { resources::read_resource(uri) })
    }
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;
