//! `gitbooks` — answer questions about OpenHuman by talking to the
//! GitBook MCP server through the shared `openhuman::mcp::http_client` path.

use crate::openhuman::mcp::http_client::{redact_endpoint, McpHttpClient};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct GitbooksSearchTool {
    client: Arc<McpHttpClient>,
}

impl GitbooksSearchTool {
    /// Builds the tool.
    ///
    /// Fallible because the client is: a malformed proxy URL or an unusable
    /// TLS setting stops one being built. It used to panic on that; a
    /// documentation tool taking the process down over a proxy setting is not
    /// a trade anyone would choose.
    ///
    /// # Errors
    ///
    /// Returns the reason the client could not be built.
    pub fn new(endpoint: String, timeout_secs: u64) -> Result<Self, String> {
        Ok(Self {
            client: Arc::new(
                McpHttpClient::new(endpoint, timeout_secs).map_err(|error| error.to_string())?,
            ),
        })
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

        tracing::debug!(
            target: "[gitbooks]",
            endpoint = %redact_endpoint(self.client.endpoint()),
            tool = "searchDocumentation",
            "dispatching via shared MCP client"
        );

        match self
            .client
            .call_tool("searchDocumentation", json!({ "query": query }))
            .await
        {
            Ok(result) => Ok(crate::openhuman::skills::types::tool_result_from_mcp(
                result.rendered,
            )),
            Err(e) => Ok(ToolResult::error(format!("gitbooks_search failed: {e}"))),
        }
    }
}

pub struct GitbooksGetPageTool {
    client: Arc<McpHttpClient>,
}

impl GitbooksGetPageTool {
    /// Builds the tool.
    ///
    /// Fallible because the client is: a malformed proxy URL or an unusable
    /// TLS setting stops one being built. It used to panic on that; a
    /// documentation tool taking the process down over a proxy setting is not
    /// a trade anyone would choose.
    ///
    /// # Errors
    ///
    /// Returns the reason the client could not be built.
    pub fn new(endpoint: String, timeout_secs: u64) -> Result<Self, String> {
        Ok(Self {
            client: Arc::new(
                McpHttpClient::new(endpoint, timeout_secs).map_err(|error| error.to_string())?,
            ),
        })
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

        tracing::debug!(
            target: "[gitbooks]",
            endpoint = %redact_endpoint(self.client.endpoint()),
            tool = "getPage",
            "dispatching via shared MCP client"
        );

        match self
            .client
            .call_tool("getPage", json!({ "url": url }))
            .await
        {
            Ok(result) => Ok(crate::openhuman::skills::types::tool_result_from_mcp(
                result.rendered,
            )),
            Err(e) => Ok(ToolResult::error(format!("gitbooks_get_page failed: {e}"))),
        }
    }
}

#[cfg(test)]
#[path = "gitbooks_tests.rs"]
mod tests;
