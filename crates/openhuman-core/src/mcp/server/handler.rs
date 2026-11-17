//! Host callbacks for the module-owned MCP server protocol.
//!
//! TinyMCP validates MCP frames and emits typed callbacks. This module applies
//! OpenHuman policy and invokes the existing tool/resource implementations;
//! it does not parse protocol messages or own a transport.

use serde_json::{Map, Value};
use tinymcp_bus::{ServerHostCall, ServerHostReply, ToolCallError};

use crate::core::runtime::DispatchCtx;

use super::{resources, subagent_depth, tools};

const INSTRUCTIONS: &str = "OpenHuman MCP exposes first-level core integration: inspect the live tool catalog with core.list_tools or core.tool_instructions, inspect subagents with agent.list_subagents, run a standalone subagent with agent.run_subagent, use web_search or web_answer for live web lookups (and searxng_search when self-hosted search is enabled), and use memory.recall (answer with citations), memory.fetch or memory.list to read memory, and memory.learn or memory.forget to change it.";

/// Apply the host-owned part of a validated TinyMCP server callback.
pub(super) async fn dispatch(ctx: DispatchCtx, call: ServerHostCall) -> ServerHostReply {
    match call {
        ServerHostCall::ListTools { .. } => {
            let specs = tools::list_tool_specs(&ctx)
                .await
                .iter()
                .map(tools::server_tool_spec)
                .collect::<Vec<_>>();
            match serde_json::to_value(specs) {
                Ok(value) => ServerHostReply::Success { value },
                Err(_) => ServerHostReply::Internal {
                    message: "tool declarations could not be encoded".to_owned(),
                },
            }
        }
        ServerHostCall::CallTool {
            source_type,
            headers,
            name,
            arguments,
        } => {
            let depth = subagent_depth::parse_header(header_string(
                &headers,
                subagent_depth::HEADER_SUBAGENT_DEPTH,
            ));
            log::trace!(
                "[mcp_server] tools/call tool={name} client_source_type={source_type} chain_depth={depth:?}"
            );
            let result = subagent_depth::scope(
                depth,
                tools::call_tool(&ctx, &name, Value::Object(arguments), &source_type),
            )
            .await;
            result.map_or_else(reply_error, |value| ServerHostReply::Success { value })
        }
        ServerHostCall::ReadResource { uri, .. } => resources::read_resource(&ctx, &uri)
            .await
            .map_or_else(reply_error, |value| ServerHostReply::Success { value }),
        ServerHostCall::ListPrompts { .. } => ServerHostReply::Success {
            value: Value::Array(Vec::new()),
        },
        ServerHostCall::GetPrompt { name, .. } => ServerHostReply::InvalidParams {
            message: format!("unknown prompt `{name}`"),
        },
        ServerHostCall::Unsupported => ServerHostReply::Internal {
            message: "unsupported server callback".to_owned(),
        },
        _ => ServerHostReply::Internal {
            message: "unsupported server callback".to_owned(),
        },
    }
}

/// Declarations frozen into a module-owned server session.
pub(super) fn declarations() -> (Value, String, Vec<Value>) {
    let info = tinymcp_bus::ServerInfo::new("openhuman-core", env!("CARGO_PKG_VERSION"))
        .with_instructions(INSTRUCTIONS);
    let resources = resources::resource_specs()
        .into_iter()
        .filter_map(|resource| serde_json::to_value(resource).ok())
        .collect();
    (
        serde_json::to_value(info).expect("server info is serializable"),
        "mcp".to_owned(),
        resources,
    )
}

fn reply_error(error: ToolCallError) -> ServerHostReply {
    match error {
        ToolCallError::InvalidParams(message) => ServerHostReply::InvalidParams { message },
        ToolCallError::Internal(message) => ServerHostReply::Internal { message },
        ToolCallError::ResourceNotFound(message) => ServerHostReply::ResourceNotFound { message },
    }
}

fn header_string<'a>(headers: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_str())
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;
