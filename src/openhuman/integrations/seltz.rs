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
        let http_client = reqwest::Client::builder()
            .use_rustls_tls()
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
            return format!("_No results for `{query}`._");
        }

        let mut out = format!("# Search results — `{query}`\n");
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
mod tests {
    use super::*;

    fn tool() -> SeltzSearchTool {
        SeltzSearchTool::new(None, None, 5, 15)
    }

    fn tool_with_key() -> SeltzSearchTool {
        SeltzSearchTool::new(Some("test-key".into()), None, 5, 15)
    }

    #[test]
    fn test_tool_name() {
        assert_eq!(tool().name(), "seltz_search");
    }

    #[test]
    fn test_tool_description() {
        assert!(tool().description().contains("Seltz"));
    }

    #[test]
    fn test_parameters_schema() {
        let schema = tool().parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["query"].is_object());
        assert!(schema["properties"]["include_domains"].is_object());
        assert!(schema["properties"]["scope"].is_object());
    }

    #[test]
    fn test_render_plain_empty() {
        let result = tool().render_results_plain(&[], "test query");
        assert!(result.contains("No results found"));
    }

    #[test]
    fn test_render_plain_with_data() {
        let docs = vec![
            SeltzDocument {
                url: "https://example.com/a".into(),
                content: "First result content.".into(),
                title: Some("First Result".into()),
                published_date: Some("2026-01-15".into()),
            },
            SeltzDocument {
                url: "https://example.com/b".into(),
                content: "Second result content.".into(),
                title: None,
                published_date: None,
            },
        ];

        let result = tool().render_results_plain(&docs, "test");
        assert!(result.contains("via Seltz"));
        assert!(result.contains("First Result"));
        assert!(result.contains("https://example.com/a"));
        assert!(result.contains("Published: 2026-01-15"));
        assert!(result.contains("First result content."));
        assert!(result.contains("Untitled"));
    }

    #[test]
    fn test_render_plain_respects_max_results() {
        let tool = SeltzSearchTool::new(None, None, 1, 15);
        let docs = vec![
            SeltzDocument {
                url: "https://a.com".into(),
                content: "A".into(),
                title: Some("A".into()),
                published_date: None,
            },
            SeltzDocument {
                url: "https://b.com".into(),
                content: "B".into(),
                title: Some("B".into()),
                published_date: None,
            },
        ];
        let result = tool.render_results_plain(&docs, "q");
        assert!(result.contains("https://a.com"));
        assert!(!result.contains("https://b.com"));
    }

    #[test]
    fn test_render_plain_truncates_long_content() {
        let long_content = "x".repeat(600);
        let docs = vec![SeltzDocument {
            url: "https://t.com".into(),
            content: long_content,
            title: Some("T".into()),
            published_date: None,
        }];
        let result = tool().render_results_plain(&docs, "q");
        assert!(result.contains("..."));
        let content_line = result.lines().find(|l| l.trim().starts_with('x')).unwrap();
        assert!(content_line.trim().len() <= 503);
    }

    #[test]
    fn test_render_markdown_empty() {
        let result = tool().render_results_markdown(&[], "test");
        assert!(result.contains("No results"));
    }

    #[test]
    fn test_render_markdown_with_data() {
        let docs = vec![SeltzDocument {
            url: "https://example.com".into(),
            content: "Some content.".into(),
            title: Some("Example".into()),
            published_date: Some("2026-01-01".into()),
        }];
        let result = tool().render_results_markdown(&docs, "test");
        assert!(result.contains("[Example](https://example.com)"));
        assert!(result.contains("Published: 2026-01-01"));
        assert!(result.contains("> Some content."));
    }

    #[tokio::test]
    async fn test_execute_missing_query() {
        let result = tool_with_key().execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_empty_query() {
        let result = tool_with_key().execute(json!({"query": ""})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_without_api_key() {
        let result = tool().execute(json!({"query": "test"})).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no API key configured"));
    }

    #[tokio::test]
    async fn test_execute_posts_to_seltz_and_renders_results() {
        use axum::{extract::Json, routing::post, Router};
        use serde_json::Value;

        let app = Router::new().route(
            "/search",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["query"], "test query");
                Json(json!({
                    "documents": [
                        {
                            "url": "https://example.com/result",
                            "title": "Seltz Result",
                            "content": "Content from Seltz search.",
                            "published_date": "2026-05-01"
                        }
                    ]
                }))
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        let tool = SeltzSearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
        let result = tool
            .execute(json!({"query": "test query"}))
            .await
            .expect("execute() should succeed");

        assert!(result.output().contains("Seltz Result"));
        assert!(result.output().contains("https://example.com/result"));
        assert!(result.output().contains("Content from Seltz search."));
    }

    #[test]
    fn test_max_results_clamped() {
        let tool = SeltzSearchTool::new(None, None, 100, 15);
        assert_eq!(tool.max_results, 20);
        let tool = SeltzSearchTool::new(None, None, 0, 15);
        assert_eq!(tool.max_results, 1);
    }
}
