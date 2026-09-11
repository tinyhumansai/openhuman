//! Seltz web search integration — direct API (not backend-proxied).
//!
//! **Scope**: Agent + CLI/RPC.
//!
//! **Endpoint**: `POST https://api.seltz.ai/v1/search`
//!
//! **Auth**: `x-api-key` header with user-provided API key.
//!
//! Seltz is an independent web search API optimized for AI agents, built on a
//! custom crawler/index with sub-200ms median latency. Unlike the Parallel
//! integration, this calls the Seltz API directly — no backend proxy needed.

use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use crate::openhuman::util::utf8_safe_prefix_at_byte_boundary;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

/// Default Seltz API base URL.
const DEFAULT_API_URL: &str = "https://api.seltz.ai/v1";

// ── Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct SeltzSearchResponse {
    #[serde(default)]
    pub documents: Vec<SeltzDocument>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SeltzDocument {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub published_date: Option<String>,
}

// ── SeltzSearchTool ─────────────────────────────────────────────────

/// Real-time web search via the Seltz API.
///
/// Requires a `SELTZ_API_KEY` (or `OPENHUMAN_SELTZ_API_KEY`) environment
/// variable or `seltz.api_key` config field. When the key is absent the tool
/// is still registered but returns a clear "not configured" error at call time
/// so the agent can fall back to other search tools.
pub struct SeltzSearchTool {
    api_key: Option<String>,
    api_url: String,
    max_results: usize,
    timeout_secs: u64,
    http_client: reqwest::Client,
}

impl SeltzSearchTool {
    pub fn new(
        api_key: Option<String>,
        api_url: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        let timeout = timeout_secs.max(1);
        // Platform-appropriate TLS backend — see [`crate::openhuman::util::tls`].
        let http_client = crate::openhuman::util::tls::tls_client_builder()
            .http1_only()
            .timeout(Duration::from_secs(timeout))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build Seltz HTTP client");

        Self {
            api_key,
            api_url: api_url.unwrap_or_else(|| DEFAULT_API_URL.to_string()),
            max_results: max_results.clamp(1, 20),
            timeout_secs: timeout,
            http_client,
        }
    }

    fn render_results_plain(&self, docs: &[SeltzDocument], query: &str) -> String {
        if docs.is_empty() {
            return format!("No results found for: {}", query);
        }

        let mut lines = vec![format!("Search results for: {} (via Seltz)", query)];

        for (i, doc) in docs.iter().take(self.max_results).enumerate() {
            let title = doc
                .title
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or("Untitled");
            let url = doc.url.trim();

            lines.push(format!("{}. {}", i + 1, title));
            lines.push(format!("   {}", url));

            if let Some(date) = doc.published_date.as_deref() {
                let date = date.trim();
                if !date.is_empty() {
                    lines.push(format!("   Published: {}", date));
                }
            }

            let content = doc.content.trim();
            if !content.is_empty() {
                let truncated = crate::openhuman::util::truncate_with_ellipsis(content, 500);
                lines.push(format!("   {}", truncated));
            }
        }

        lines.join("\n")
    }

    fn render_results_markdown(&self, docs: &[SeltzDocument], query: &str) -> String {
        if docs.is_empty() {
            return format!("_No results for `{query}`_ (via Seltz)");
        }

        // Carry the shared `(via <Provider>)` attribution marker the plain-text
        // renderer already emits, so the tool timeline can label the row
        // (#5136). Production shows the markdown rendering.
        let mut out = format!("# Search results — `{query}` (via Seltz)\n");
        for doc in docs.iter().take(self.max_results) {
            let title = doc
                .title
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or("Untitled");
            out.push_str(&format!("\n## [{title}]({})\n", doc.url.trim()));
            if let Some(date) = doc.published_date.as_deref() {
                let date = date.trim();
                if !date.is_empty() {
                    out.push_str(&format!("_Published: {date}_\n\n"));
                }
            }
            let content = doc.content.trim();
            if !content.is_empty() {
                let truncated = crate::openhuman::util::truncate_with_suffix(content, 500, "…");
                out.push_str(&format!("> {truncated}\n"));
            }
        }
        out
    }
}

#[async_trait]
impl Tool for SeltzSearchTool {
    fn name(&self) -> &str {
        "seltz_search"
    }

    fn description(&self) -> &str {
        "Search the web in real time using Seltz. Returns current information from trusted \
         sources with URLs and extracted content. Supports domain filtering, date ranges, \
         and news scope. Fast (<200ms) and optimized for AI agent workflows."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query. Use concise keywords for best results."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default from config, max 20)."
                },
                "include_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Restrict results to these domains (e.g. [\"bbc.com\", \"reuters.com\"])."
                },
                "exclude_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Exclude results from these domains."
                },
                "from_date": {
                    "type": "string",
                    "description": "Only include results published on or after this date (YYYY-MM-DD)."
                },
                "to_date": {
                    "type": "string",
                    "description": "Only include results published on or before this date (YYYY-MM-DD)."
                },
                "scope": {
                    "type": "string",
                    "description": "Restrict to a specific scope. Currently supported: \"news\"."
                }
            },
            "required": ["query"]
        })
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: serde_json::Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|q| q.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?;

        if query.trim().is_empty() {
            anyhow::bail!("Search query cannot be empty");
        }

        let api_key = self.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Seltz search unavailable: no API key configured. \
                 Set SELTZ_API_KEY or OPENHUMAN_SELTZ_API_KEY, \
                 or add seltz.api_key to config.toml."
            )
        })?;

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| n.clamp(1, 20) as usize)
            .unwrap_or(self.max_results);

        // Build request body — only include optional fields when set.
        let mut body = json!({
            "query": query,
            "max_results": max_results,
        });
        let body_map = body.as_object_mut().unwrap();

        if let Some(include) = args.get("include_domains") {
            if include.is_array() {
                body_map.insert("include_domains".to_string(), include.clone());
            }
        }
        if let Some(exclude) = args.get("exclude_domains") {
            if exclude.is_array() {
                body_map.insert("exclude_domains".to_string(), exclude.clone());
            }
        }
        if let Some(from) = args.get("from_date").and_then(|v| v.as_str()) {
            if !from.is_empty() {
                body_map.insert("from_date".to_string(), json!(from));
            }
        }
        if let Some(to) = args.get("to_date").and_then(|v| v.as_str()) {
            if !to.is_empty() {
                body_map.insert("to_date".to_string(), json!(to));
            }
        }
        if let Some(scope) = args.get("scope").and_then(|v| v.as_str()) {
            if !scope.is_empty() {
                body_map.insert("scope".to_string(), json!(scope));
            }
        }

        let url = format!("{}/search", self.api_url);

        tracing::debug!(
            query_len = query.chars().count(),
            max_results,
            timeout_secs = self.timeout_secs,
            "[seltz] POST {url}"
        );

        let resp = self
            .http_client
            .post(&url)
            .header("x-api-key", api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                tracing::warn!("[seltz] request failed: {e}");
                anyhow::anyhow!("Seltz search request failed: {e}")
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let detail = utf8_safe_prefix_at_byte_boundary(&body_text, 500);
            tracing::warn!(
                status = %status,
                "[seltz] non-2xx response: {detail}"
            );
            anyhow::bail!("Seltz returned {status}: {detail}");
        }

        let search_resp: SeltzSearchResponse = resp.json().await.map_err(|e| {
            tracing::warn!("[seltz] failed to parse response: {e}");
            anyhow::anyhow!("Failed to parse Seltz response: {e}")
        })?;

        tracing::debug!(
            doc_count = search_resp.documents.len(),
            "[seltz] search complete"
        );

        let mut result =
            ToolResult::success(self.render_results_plain(&search_resp.documents, query));
        if options.prefer_markdown {
            result.markdown_formatted =
                Some(self.render_results_markdown(&search_resp.documents, query));
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "seltz_tests.rs"]
mod tests;
