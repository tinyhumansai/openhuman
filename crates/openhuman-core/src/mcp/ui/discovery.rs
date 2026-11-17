//! What a configured server declares for UI, logged when its tools are listed.
//!
//! One line per tool that sends `_meta` (its keys, plus any widget template
//! URI). Only names, keys and `ui://` URIs are logged,
//! never values that could carry data.

use serde_json::Value;

fn template_uri(meta: &Value) -> Option<String> {
    tinymcp_bus::ui::template_from_meta(meta).map(|(uri, _)| uri)
}

fn meta_keys(meta: &Value) -> Vec<String> {
    let Some(map) = meta.as_object() else {
        return Vec::new();
    };
    let mut keys: Vec<String> = map.keys().cloned().collect();
    if let Some(ui) = map.get("ui").and_then(Value::as_object) {
        keys.extend(ui.keys().map(|key| format!("ui.{key}")));
    }
    keys
}

fn log_tool(server: &str, tool: &str, meta: &Value) {
    tracing::info!(
        server = %server,
        tool = %tool,
        meta_keys = ?meta_keys(meta),
        template = ?template_uri(meta),
        "[mcp-ui] tool _meta"
    );
}

/// Logs a configured server's declared UI, from the tools it just listed.
pub fn log_configured_server(server: &str, tools: &[tinymcp_bus::McpRemoteTool]) {
    tracing::info!(server = %server, tools = tools.len(), "[mcp-ui] configured server listed");
    for tool in tools {
        if let Some(meta) = tool.meta.as_ref() {
            log_tool(server, &tool.name, meta);
        }
    }
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
