//! What a server declares for UI, logged when it connects.
//!
//! One line per tool that sends `_meta` (its keys, plus any widget template
//! URI) and one per resource. Only names, keys and `ui://` URIs are logged,
//! never values that could carry data.

use std::sync::Arc;

use serde_json::Value;

use crate::mcp::config_servers::McpServerRegistry;
use crate::mcp::host::{self, McpHost};

use super::resolve::is_ui_uri;

fn template_uri(meta: &Value) -> Option<String> {
    tinymcp::ui::template_from_meta(meta).map(|(uri, _)| uri)
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

fn log_resource(server: &str, uri: &str, mime: Option<&str>) {
    let shown = if is_ui_uri(uri) {
        uri
    } else {
        "<non-ui resource>"
    };
    tracing::info!(
        server = %server,
        uri = %shown,
        mime = ?mime,
        "[mcp-ui] resource"
    );
}

/// Logs an installed server's declared UI.
pub async fn log_installed_server(host: &McpHost, server_id: &str) {
    let connections = host.dynamic().connections();
    let tools = connections.tools_for(server_id).await.unwrap_or_default();
    let metas = connections.tool_metas(server_id).await.unwrap_or_default();
    tracing::info!(
        server = %server_id,
        tools = tools.len(),
        with_meta = metas.len(),
        "[mcp-ui] server connected"
    );
    let mut names: Vec<&String> = metas.keys().collect();
    names.sort();
    for name in names {
        log_tool(server_id, name, &metas[name]);
    }
    match connections.list_resources(server_id).await {
        Ok(resources) => {
            for resource in resources {
                log_resource(server_id, &resource.uri, resource.mime_type.as_deref());
            }
        }
        Err(error) => {
            tracing::info!(server = %server_id, %error, "[mcp-ui] resources/list unavailable")
        }
    }
}

/// [`log_installed_server`] in the background, against the process host.
pub fn spawn_log_installed(server_id: String) {
    let Some(service) = host::try_service() else {
        return;
    };
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        log_installed_server(&service, &server_id).await;
    });
}

/// Logs every connected installed server on `host`.
pub async fn log_all_installed(host: Arc<McpHost>) {
    for server in host.dynamic().connected_overview().await {
        log_installed_server(&host, &server.server_id).await;
    }
}

/// Logs a configured server's declared UI, from the registry's last listing.
pub async fn log_configured_server(
    registry: &McpServerRegistry,
    store: &tinymcp::registry::Store,
    server: &str,
) {
    let tools = registry.cached_tools(server, store).unwrap_or_default();
    tracing::info!(server = %server, tools = tools.len(), "[mcp-ui] configured server listed");
    for tool in &tools {
        if let Some(meta) = registry.tool_meta(server, &tool.name) {
            log_tool(server, &tool.name, &meta);
        }
    }
    match registry.list_resources(server).await {
        Ok(resources) => {
            for resource in resources {
                log_resource(server, &resource.uri, resource.mime_type.as_deref());
            }
        }
        Err(error) => {
            tracing::info!(server = %server, %error, "[mcp-ui] resources/list unavailable")
        }
    }
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
