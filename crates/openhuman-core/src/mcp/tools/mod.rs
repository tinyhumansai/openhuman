//! The generic configured-server bridge as agent tools: `mcp_list_servers`,
//! `mcp_list_tools` and `mcp_call_tool`, one file each.
//!
//! TinyMCP owns the bridge declarations, transport calls, rendering, secret
//! scrubbing, and call classification. These adapters keep OpenHuman's act
//! gate and prompt-injection screen and adapt the returned bus vocabulary to
//! `tinytools`. They register only when the static `[[mcp_client.servers]]`
//! registry is non-empty (`tools/ops.rs`).

use serde_json::Value;

mod call;
mod list_servers;
mod list_tools;

pub(in crate::mcp) use call::classified_result;
pub use call::mcp_call_tool;
pub use list_servers::McpListServersTool;
pub use list_tools::McpListToolsTool;

fn bridge_tool_spec(name: &str) -> tinymcp_bus::agent_tools::AgentToolSpec {
    tinymcp_bus::agent_tools::bridge_tool_specs()
        .into_iter()
        .find(|spec| spec.name == name)
        .expect("TinyMCP bridge contract includes the requested tool")
}

pub(in crate::mcp) fn input_module_error_code(
    error: tinybus::module::ModuleCallError,
) -> &'static str {
    use tinybus::module::ModuleCallError;
    match error {
        ModuleCallError::Unavailable | ModuleCallError::IncompatibleContract => {
            tinymcp_bus::errors::BUS
        }
        ModuleCallError::TransportFailed => tinymcp_bus::errors::TRANSPORT,
        ModuleCallError::ModuleFault => tinymcp_bus::errors::BUS,
    }
}

pub(in crate::mcp) fn input_module_error_message(
    error: tinybus::module::ModuleCallError,
) -> &'static str {
    use tinybus::module::ModuleCallError;
    match error {
        ModuleCallError::Unavailable => "TinyMCP module is unavailable",
        ModuleCallError::IncompatibleContract => "TinyMCP contract is incompatible",
        ModuleCallError::TransportFailed => "TinyMCP transport failed",
        ModuleCallError::ModuleFault => "TinyMCP execution failed",
    }
}

/// Reads a required identifier without applying MCP-owned normalization.
fn required_string_arg(args: &Value, key: &str) -> anyhow::Result<String> {
    let missing = || anyhow::anyhow!("missing required `{key}`");
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(missing)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
