//! The `mcp_ui` operations: serving a widget its document and running the
//! tool calls a widget asks for.
//!
//! A widget is untrusted page content. Everything it reaches goes through
//! here, under the same scan and policy the agent's own calls get, and a call
//! that can change something waits for the user unless the tool declares
//! itself read-only.

use std::time::Instant;

use serde_json::{json, Value};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::core::Outcome;
use crate::security::{SecurityPolicy, ToolOperation};

use super::cache;
use super::port::UiServerPort;
use super::resolve::{is_ui_uri, resource_from_contents};
use super::types::UiResource;

fn require(value: Option<String>, field: &str) -> Result<String, String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{field} must not be empty"))
}

fn encode(resource: &UiResource) -> Result<Value, String> {
    serde_json::to_value(resource).map_err(|error| format!("serialization error: {error}"))
}

/// The document a widget frame loads: an inline one by `inline_id`, or the
/// `ui://` resource `uri` on `server_id`.
///
/// # Errors
///
/// When the id is unknown or expired, the URI is not `ui://`, the server is
/// not available, or the document is not acceptable HTML.
pub async fn resource_read(
    port: &dyn UiServerPort,
    server_id: Option<String>,
    uri: Option<String>,
    inline_id: Option<String>,
) -> Result<Outcome<Value>, String> {
    if let Some(id) = inline_id.filter(|id| !id.trim().is_empty()) {
        let entry = cache::get_inline(id.trim())
            .ok_or_else(|| "this widget is no longer available".to_string())?;
        if let (Some(owner), Some(asked)) = (entry.server_id.as_deref(), server_id.as_deref()) {
            if owner != asked.trim() {
                return Err("this widget belongs to another server".to_string());
            }
        }
        tracing::debug!(inline_id = %id, "[mcp_ui] served an inline widget document");
        return Ok(Outcome::new(
            encode(&entry.resource)?,
            vec![format!("resource_read inline_id={id}")],
        ));
    }

    let server_id = require(server_id, "server_id")?;
    let uri = require(uri, "uri")?;
    if !is_ui_uri(&uri) {
        return Err("only ui:// resources can be shown".to_string());
    }
    if !port.is_available(&server_id).await {
        return Err(format!("MCP server `{server_id}` is not connected"));
    }
    if let Some(resource) = cache::get_read(&server_id, &uri) {
        tracing::debug!(server_id, uri, "[mcp_ui] widget document served from cache");
        return Ok(Outcome::new(
            encode(&resource)?,
            vec![format!("resource_read server_id={server_id} cached")],
        ));
    }
    let contents = port.read_resource(&server_id, &uri).await?;
    let resource = resource_from_contents(&contents, &uri)?;
    tracing::debug!(
        server_id,
        uri,
        bytes = resource.html.len(),
        connect = resource.csp.connect_domains.len(),
        "[mcp_ui] read a widget document"
    );
    cache::put_read(&server_id, &uri, resource.clone());
    Ok(Outcome::new(
        encode(&resource)?,
        vec![format!("resource_read server_id={server_id}")],
    ))
}

/// A tool call a widget asked for.
///
/// Answers `requires_confirmation: true` without calling when the tool is
/// not read-only and the user has not confirmed it.
///
/// # Errors
///
/// When the server is not available, the tool is not offered, hidden from
/// apps, or blocked by policy.
pub async fn tool_call(
    port: &dyn UiServerPort,
    security: &SecurityPolicy,
    server_id: Option<String>,
    tool_name: Option<String>,
    arguments: Option<Value>,
    confirmed: bool,
) -> Result<Outcome<Value>, String> {
    let server_id = require(server_id, "server_id")?;
    let tool_name = require(tool_name, "tool_name")?;
    let arguments = match arguments {
        None | Some(Value::Null) => json!({}),
        Some(value @ Value::Object(_)) => value,
        Some(_) => return Err("arguments must be an object".to_string()),
    };
    if !port.is_available(&server_id).await {
        return Err(format!("MCP server `{server_id}` is not connected"));
    }
    let descriptor = port
        .tool_descriptor(&server_id, &tool_name)
        .await
        .ok_or_else(|| format!("`{tool_name}` is not offered by this server"))?;
    if !tinymcp::ui::visible_to_app(descriptor.meta.as_ref()) {
        tracing::debug!(
            server_id,
            tool_name,
            "[mcp_ui] widget asked for a model-only tool"
        );
        return Err(format!("`{tool_name}` cannot be called from a widget"));
    }
    let read_only = tinymcp::ui::is_read_only(descriptor.annotations.as_ref());
    if !read_only && !confirmed {
        tracing::debug!(
            server_id,
            tool_name,
            "[mcp_ui] widget tool call needs confirmation"
        );
        return Ok(Outcome::new(
            json!({"requires_confirmation": true, "read_only": false}),
            vec![format!(
                "tool_call server_id={server_id} tool={tool_name} awaiting confirmation"
            )],
        ));
    }
    let operation = if read_only {
        ToolOperation::Read
    } else {
        ToolOperation::Act
    };
    security.enforce_tool_operation(operation, &format!("mcp_ui:{server_id}/{tool_name}"))?;

    let start = Instant::now();
    let result = port.call_tool(&server_id, &tool_name, arguments).await;
    let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    BUS.publish(DomainEvent::McpClientToolExecuted {
        server_id: server_id.clone(),
        tool_name: tool_name.clone(),
        success: result.is_ok(),
        elapsed_ms,
    });
    tracing::debug!(
        server_id,
        tool_name,
        elapsed_ms,
        ok = result.is_ok(),
        "[mcp_ui] widget tool call"
    );
    let raw = result?;
    let is_error = raw.get("isError").and_then(Value::as_bool).unwrap_or(false);
    Ok(Outcome::new(
        json!({"requires_confirmation": false, "read_only": read_only, "result": raw, "is_error": is_error}),
        vec![format!(
            "tool_call server_id={server_id} tool={tool_name} elapsed_ms={elapsed_ms}"
        )],
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
