use super::{McpAuthChallenge, McpRemoteTool, McpSseEvent};
use anyhow::Context;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use std::collections::HashMap;

pub(super) fn parse_sse_message(body: &str) -> anyhow::Result<Value> {
    let events = parse_sse_events(body)?;
    let event = events
        .into_iter()
        .find_map(|event| event.data)
        .ok_or_else(|| anyhow::anyhow!("No SSE data frame found in MCP response: {body}"))?;
    Ok(event)
}

/// Return the first JSON `data:` frame from the **fully-terminated** prefix of a
/// partially-received SSE buffer, or `None` if no complete event carrying a data
/// frame has arrived yet.
///
/// MCP Streamable HTTP lets a server keep the POST response's SSE stream open
/// after it has already emitted the single JSON-RPC reply, so reading the whole
/// body to stream-close (`response.text().await`) stalls every tool call until
/// the server closes or the request times out — the dominant transport cause of
/// the multi-minute skill latency in #4195. Reading incrementally and stopping
/// at the first data frame lets the call return the instant the reply lands.
///
/// Only the portion up to the last blank-line event boundary is parsed, so a
/// half-received final `data:` line is never decoded as truncated JSON.
/// Keepalive comments and dataless events are skipped (returns `None`, keep
/// reading). An SSE event terminates on a blank line; `\r\n` is normalised to
/// `\n` first so CRLF streams split on the same boundary.
pub(super) fn first_complete_sse_data(buffer: &str) -> anyhow::Result<Option<Value>> {
    let normalized = buffer.replace("\r\n", "\n");
    // The terminator of the last complete event. Without one, nothing is
    // fully received yet.
    let Some(boundary) = normalized.rfind("\n\n") else {
        return Ok(None);
    };
    let complete = &normalized[..=boundary];
    let events = parse_sse_events(complete)?;
    Ok(events.into_iter().find_map(|event| event.data))
}

pub(super) fn parse_sse_events(body: &str) -> anyhow::Result<Vec<McpSseEvent>> {
    let mut events = Vec::new();
    let mut event_type: Option<String> = None;
    let mut event_id: Option<String> = None;
    let mut data_lines: Vec<String> = Vec::new();

    let flush = |events: &mut Vec<McpSseEvent>,
                 event_type: &mut Option<String>,
                 event_id: &mut Option<String>,
                 data_lines: &mut Vec<String>|
     -> anyhow::Result<()> {
        if event_type.is_none() && event_id.is_none() && data_lines.is_empty() {
            return Ok(());
        }
        let data = if data_lines.is_empty() {
            None
        } else {
            let joined = data_lines.join("\n");
            Some(
                serde_json::from_str(&joined)
                    .with_context(|| format!("Failed to parse SSE data frame JSON: {joined}"))?,
            )
        };
        events.push(McpSseEvent {
            event: event_type.take(),
            id: event_id.take(),
            data,
        });
        data_lines.clear();
        Ok(())
    };

    for raw_line in body.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            flush(&mut events, &mut event_type, &mut event_id, &mut data_lines)?;
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event_type = Some(value.trim_start().to_string());
        } else if let Some(value) = line.strip_prefix("id:") {
            event_id = Some(value.trim_start().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start().to_string());
        }
    }
    flush(&mut events, &mut event_type, &mut event_id, &mut data_lines)?;
    Ok(events)
}

pub(super) fn parse_www_authenticate_challenge(headers: &HeaderMap) -> Option<McpAuthChallenge> {
    let raw = headers.get("WWW-Authenticate")?.to_str().ok()?.trim();
    let mut parts = raw.splitn(2, ' ');
    let scheme = parts.next()?.trim().to_string();
    let params = parts.next().unwrap_or("").trim();
    let attrs = parse_auth_attribute_list(params);
    Some(McpAuthChallenge {
        scheme,
        realm: attrs.get("realm").cloned(),
        resource_metadata: attrs.get("resource_metadata").cloned(),
    })
}

pub(super) fn parse_auth_attribute_list(input: &str) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    for part in input.split(',') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        attrs.insert(key.trim().to_string(), value);
    }
    attrs
}

pub(super) fn header_to_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name)?.to_str().ok().map(|s| s.to_string())
}

pub(super) fn x_mcp_headers_from_schema(
    tool: &McpRemoteTool,
    arguments: &Value,
) -> anyhow::Result<Vec<(HeaderName, HeaderValue)>> {
    let mut headers = Vec::new();
    let Some(args) = arguments.as_object() else {
        return Ok(headers);
    };
    let properties = tool
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    for (param_name, schema) in properties {
        let Some(header_suffix) = schema.get("x-mcp-header").and_then(Value::as_str) else {
            continue;
        };
        let Some(value) = args.get(&param_name) else {
            continue;
        };
        let header_name =
            HeaderName::from_bytes(format!("Mcp-Param-{header_suffix}").as_bytes())
                .with_context(|| format!("invalid x-mcp-header name for `{param_name}`"))?;
        let header_value = match value {
            Value::String(s) => HeaderValue::from_str(s),
            other => HeaderValue::from_str(&other.to_string()),
        }
        .with_context(|| format!("invalid x-mcp-header value for `{param_name}`"))?;
        headers.push((header_name, header_value));
    }

    Ok(headers)
}
