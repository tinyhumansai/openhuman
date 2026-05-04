//! `gitbooks` — answer questions about OpenHuman by talking to the
//! GitBook MCP server.
//!
//! GitBook hosts a stateless Streamable-HTTP MCP server that exposes
//! exactly two tools:
//!
//! - `searchDocumentation { query }` — returns excerpts + page links
//! - `getPage { url }` — returns the full markdown of a page
//!
//! We mirror them as `gitbooks_search` and `gitbooks_get_page`. The
//! server returns each JSON-RPC response in a single
//! `event: message\ndata: {…}` SSE frame, so we do a tiny inline
//! parse — no need to pull in a full SSE/MCP client crate yet.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Minimal MCP-over-HTTP client for stateless servers (no session id).
struct McpHttpClient {
    endpoint: String,
    timeout_secs: u64,
    next_id: AtomicI64,
}

impl McpHttpClient {
    fn new(endpoint: String, timeout_secs: u64) -> Self {
        Self {
            endpoint,
            timeout_secs,
            next_id: AtomicI64::new(1),
        }
    }

    async fn call_tool(&self, name: &str, arguments: Value) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        });

        let builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none());
        let builder =
            crate::openhuman::config::apply_runtime_proxy_to_builder(builder, "tool.gitbooks");
        let client = builder.build()?;

        // Log only the redacted host so query strings / path params /
        // any future bearer-token-bearing endpoints don't leak into
        // logs aggregated for triage.
        tracing::debug!(
            target: "[gitbooks]",
            endpoint = %redact_endpoint(&self.endpoint),
            tool = %name,
            "MCP tools/call"
        );

        let resp = client
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(serde_json::to_vec(&body)?)
            .send()
            .await?;

        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let text = resp.text().await?;

        if !status.is_success() {
            anyhow::bail!("MCP HTTP {} — {}", status.as_u16(), text);
        }

        let payload: Value = if content_type.starts_with("text/event-stream") {
            parse_sse_message(&text)?
        } else {
            serde_json::from_str(&text).map_err(|e| {
                anyhow::anyhow!("Failed to parse MCP JSON response: {e} — body: {text}")
            })?
        };

        if let Some(err) = payload.get("error") {
            anyhow::bail!("MCP error: {err}");
        }
        let result = payload
            .get("result")
            .ok_or_else(|| anyhow::anyhow!("MCP response missing 'result': {payload}"))?
            .clone();
        Ok(result)
    }
}

/// Reduce a configured endpoint URL to `scheme://host[:port]` for
/// safe logging. Anything that doesn't parse as a recognisable
/// http(s) URL is reported as `<redacted>` rather than echoed.
fn redact_endpoint(raw: &str) -> String {
    let trimmed = raw.trim();
    let (scheme, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        ("http", r)
    } else {
        return "<redacted>".into();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return "<redacted>".into();
    }
    format!("{scheme}://{authority}")
}

/// Parse the first `data: {…}` line from a Streamable-HTTP SSE
/// response. The GitBook server emits exactly one frame per JSON-RPC
/// request, so we do not need a full SSE state machine.
fn parse_sse_message(body: &str) -> anyhow::Result<Value> {
    for line in body.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim_start();
            if data.is_empty() {
                continue;
            }
            return serde_json::from_str(data).map_err(|e| {
                anyhow::anyhow!("Failed to parse SSE data frame: {e} — line: {data}")
            });
        }
    }
    anyhow::bail!("No SSE data frame found in MCP response: {body}")
}

/// Render an MCP `tools/call` result into a single string for the
/// agent. MCP returns `{ content: [{ type, text }, …], isError? }`.
fn render_tool_result(result: &Value) -> ToolResult {
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut out = String::new();
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for block in content {
            if let Some(t) = block.get("text").and_then(Value::as_str) {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        out = result.to_string();
    }

    if is_error {
        ToolResult::error(out)
    } else {
        ToolResult::success(out)
    }
}

// ── Search ─────────────────────────────────────────────────────────

pub struct GitbooksSearchTool {
    client: Arc<McpHttpClient>,
}

impl GitbooksSearchTool {
    pub fn new(endpoint: String, timeout_secs: u64) -> Self {
        Self {
            client: Arc::new(McpHttpClient::new(endpoint, timeout_secs)),
        }
    }
}

#[async_trait]
impl Tool for GitbooksSearchTool {
    fn name(&self) -> &str {
        "gitbooks_search"
    }

    fn description(&self) -> &str {
        "Search the OpenHuman product documentation. Use this to answer questions about how \
        OpenHuman works, find features, look up configuration, or locate guides. Returns \
        excerpts with page titles and links — follow up with `gitbooks_get_page` for the \
        full markdown of a page."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language question or keyword query about OpenHuman."
                }
            },
            "required": ["query"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?;
        if query.trim().is_empty() {
            return Ok(ToolResult::error("query cannot be empty"));
        }

        match self
            .client
            .call_tool("searchDocumentation", json!({ "query": query }))
            .await
        {
            Ok(result) => Ok(render_tool_result(&result)),
            Err(e) => Ok(ToolResult::error(format!("gitbooks_search failed: {e}"))),
        }
    }
}

// ── Get page ──────────────────────────────────────────────────────

pub struct GitbooksGetPageTool {
    client: Arc<McpHttpClient>,
}

impl GitbooksGetPageTool {
    pub fn new(endpoint: String, timeout_secs: u64) -> Self {
        Self {
            client: Arc::new(McpHttpClient::new(endpoint, timeout_secs)),
        }
    }
}

#[async_trait]
impl Tool for GitbooksGetPageTool {
    fn name(&self) -> &str {
        "gitbooks_get_page"
    }

    fn description(&self) -> &str {
        "Fetch the full markdown of a specific OpenHuman documentation page by URL. Pair this \
        with `gitbooks_search` — search returns partial excerpts; use this to get the \
        complete page when more detail is needed."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The full URL of the OpenHuman documentation page (e.g. https://tinyhumans.gitbook.io/openhuman/getting-started)."
                }
            },
            "required": ["url"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;
        if url.trim().is_empty() {
            return Ok(ToolResult::error("url cannot be empty"));
        }

        match self
            .client
            .call_tool("getPage", json!({ "url": url }))
            .await
        {
            Ok(result) => Ok(render_tool_result(&result)),
            Err(e) => Ok(ToolResult::error(format!("gitbooks_get_page failed: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_endpoint_keeps_only_origin() {
        assert_eq!(
            redact_endpoint("https://tinyhumans.gitbook.io/openhuman/~gitbook/mcp"),
            "https://tinyhumans.gitbook.io"
        );
        assert_eq!(
            redact_endpoint("http://example.com:8080/path?token=secret"),
            "http://example.com:8080"
        );
    }

    #[test]
    fn redact_endpoint_rejects_userinfo_and_unknown_schemes() {
        assert_eq!(
            redact_endpoint("https://user:pass@example.com/x"),
            "<redacted>"
        );
        assert_eq!(redact_endpoint("ftp://example.com"), "<redacted>");
        assert_eq!(redact_endpoint(""), "<redacted>");
    }

    #[test]
    fn parse_sse_extracts_data_frame() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"x\":1}}\n\n";
        let v = parse_sse_message(body).unwrap();
        assert_eq!(v["result"]["x"], 1);
    }

    #[test]
    fn parse_sse_handles_crlf() {
        let body = "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\r\n\r\n";
        assert!(parse_sse_message(body).is_ok());
    }

    #[test]
    fn parse_sse_errors_when_no_data_line() {
        let body = "event: ping\n\n";
        assert!(parse_sse_message(body).is_err());
    }

    #[test]
    fn parse_sse_errors_on_invalid_json() {
        let body = "data: not-json\n\n";
        assert!(parse_sse_message(body).is_err());
    }

    #[test]
    fn render_tool_result_concatenates_text_blocks() {
        let r = json!({
            "content": [
                {"type": "text", "text": "first"},
                {"type": "text", "text": "second"}
            ]
        });
        let out = render_tool_result(&r);
        assert!(!out.is_error);
        assert!(out.output().contains("first"));
        assert!(out.output().contains("second"));
    }

    #[test]
    fn render_tool_result_marks_errors() {
        let r = json!({
            "content": [{"type": "text", "text": "boom"}],
            "isError": true
        });
        let out = render_tool_result(&r);
        assert!(out.is_error);
        assert!(out.output().contains("boom"));
    }

    #[test]
    fn render_tool_result_falls_back_to_raw_json() {
        let r = json!({"weird": "shape"});
        let out = render_tool_result(&r);
        assert!(out.output().contains("weird"));
    }

    #[tokio::test]
    async fn search_rejects_empty_query() {
        let t = GitbooksSearchTool::new("https://example.com/mcp".into(), 5);
        let result = t.execute(json!({"query": "   "})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("empty"));
    }

    #[tokio::test]
    async fn get_page_rejects_empty_url() {
        let t = GitbooksGetPageTool::new("https://example.com/mcp".into(), 5);
        let result = t.execute(json!({"url": ""})).await.unwrap();
        assert!(result.is_error);
    }

    /// Live integration test against the real GitBook MCP endpoint.
    /// Gated behind `OPENHUMAN_GITBOOKS_LIVE_TEST=1` so CI / offline
    /// builds don't depend on the public network.
    #[tokio::test]
    async fn live_search_smoke() {
        if std::env::var("OPENHUMAN_GITBOOKS_LIVE_TEST")
            .ok()
            .as_deref()
            != Some("1")
        {
            return;
        }
        let t = GitbooksSearchTool::new(
            "https://tinyhumans.gitbook.io/openhuman/~gitbook/mcp".into(),
            30,
        );
        let result = t
            .execute(json!({"query": "what is openhuman"}))
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "live search returned error: {}",
            result.output()
        );
        assert!(!result.output().is_empty());
    }
}
