//! Parallel web search and content extraction integration tools.
//!
//! **Scope**: All (agent loop + CLI/RPC).
//!
//! **Endpoints**:
//!   - `POST /agent-integrations/parallel/search`
//!   - `POST /agent-integrations/parallel/extract`
//!
//! **Pricing** (fetched from backend):
//!   - Search:  ~$0.01/request
//!   - Extract: ~$0.002/URL
//!
//! The backend handles Parallel API keys, billing, and rate limiting.

use super::IntegrationClient;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// UTF-8 safe truncation: returns the truncated slice and whether it was truncated.
fn truncate_chars(s: &str, max_chars: usize) -> (&str, bool) {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => (&s[..byte_idx], true),
        None => (s, false),
    }
}

// ── Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct SearchResponse {
    #[serde(rename = "searchId")]
    #[allow(dead_code)]
    pub(crate) search_id: String,
    pub(crate) results: Vec<SearchResultItem>,
    #[serde(rename = "costUsd")]
    pub(crate) cost_usd: f64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchResultItem {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) publish_date: Option<String>,
    pub(crate) excerpts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExtractResponse {
    #[serde(rename = "extractId", default)]
    #[allow(dead_code)]
    extract_id: String,
    #[serde(default)]
    results: Vec<ExtractResultItem>,
    #[serde(default)]
    errors: Vec<ExtractError>,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

#[derive(Debug, Deserialize)]
struct ExtractResultItem {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    excerpts: Vec<String>,
    #[serde(default)]
    full_content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExtractError {
    #[serde(default)]
    url: String,
    #[serde(default)]
    error: String,
}

// ── ParallelSearchTool ──────────────────────────────────────────────

/// AI-powered web search via the Parallel API.
pub struct ParallelSearchTool {
    client: Arc<IntegrationClient>,
}

impl ParallelSearchTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ParallelSearchTool {
    fn name(&self) -> &str {
        "parallel_search"
    }

    fn description(&self) -> &str {
        "AI-powered web search via Parallel. Provide an objective and one or more search \
         queries. Returns relevant results with titles, URLs, and excerpts. \
         Supports modes: 'fast' (quickest), 'one-shot' (balanced), 'agentic' (most thorough). \
         Cost is per request, billed by the backend."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "objective": {
                    "type": "string",
                    "description": "What you are trying to find or learn"
                },
                "search_queries": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "One or more search queries (1-10)",
                    "minItems": 1,
                    "maxItems": 10
                },
                "mode": {
                    "type": "string",
                    "enum": ["fast", "one-shot", "agentic"],
                    "description": "Search mode (default: fast)",
                    "default": "fast"
                },
                "num_results": {
                    "type": "integer",
                    "description": "Number of results per query (1-50, default 10)",
                    "minimum": 1,
                    "maximum": 50,
                    "default": 10
                },
                "max_characters_per_excerpt": {
                    "type": "integer",
                    "description": "Max characters per excerpt (100-10000, default 500)",
                    "minimum": 100,
                    "maximum": 10000,
                    "default": 500
                }
            },
            "required": ["objective", "search_queries"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let objective = args
            .get("objective")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: objective"))?;

        if objective.trim().is_empty() {
            return Ok(ToolResult::error("objective cannot be empty"));
        }

        let search_queries = args
            .get("search_queries")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: search_queries"))?;

        if search_queries.is_empty() {
            return Ok(ToolResult::error(
                "search_queries must contain at least one query",
            ));
        }

        let mut queries: Vec<&str> = Vec::with_capacity(search_queries.len());
        for (i, v) in search_queries.iter().enumerate() {
            match v.as_str() {
                Some(s) if !s.trim().is_empty() => queries.push(s),
                Some(_) => {
                    return Ok(ToolResult::error(format!(
                        "search_queries[{i}] is an empty string"
                    )));
                }
                None => {
                    return Ok(ToolResult::error(format!(
                        "search_queries[{i}] is not a string"
                    )));
                }
            }
        }

        let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("fast");

        let mut body = json!({
            "objective": objective,
            "searchQueries": queries,
            "mode": mode,
        });

        // Build excerpts config if custom values provided
        let num_results = args.get("num_results").and_then(|v| v.as_u64());
        let max_chars = args
            .get("max_characters_per_excerpt")
            .and_then(|v| v.as_u64());

        if num_results.is_some() || max_chars.is_some() {
            let mut excerpts = json!({});
            if let Some(n) = num_results {
                excerpts["numResults"] = json!(n.clamp(1, 50));
            }
            if let Some(c) = max_chars {
                excerpts["maxCharactersPerExcerpt"] = json!(c.clamp(100, 10000));
            }
            body["excerpts"] = excerpts;
        }

        tracing::info!("[parallel_search] queries={}", queries.len());
        tracing::debug!("[parallel_search] objective={:?}", objective);

        match self
            .client
            .post::<SearchResponse>("/agent-integrations/parallel/search", &body)
            .await
        {
            Ok(resp) => {
                if resp.results.is_empty() {
                    return Ok(ToolResult::success(format!(
                        "No results found for: {}",
                        objective
                    )));
                }

                let mut lines = vec![format!("Search results ({} found):", resp.results.len())];

                for (i, item) in resp.results.iter().enumerate() {
                    lines.push(format!("\n{}. {}", i + 1, item.title));
                    lines.push(format!("   {}", item.url));
                    if let Some(ref date) = item.publish_date {
                        lines.push(format!("   Published: {}", date));
                    }
                    if let Some(excerpt) = item.excerpts.first() {
                        let text = excerpt.trim();
                        if !text.is_empty() {
                            let (slice, was_truncated) = truncate_chars(text, 500);
                            let truncated = if was_truncated {
                                format!("{slice}...")
                            } else {
                                slice.to_string()
                            };
                            lines.push(format!("   {}", truncated));
                        }
                    }
                }

                lines.push(format!("\nCost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!("Parallel search failed: {e}"))),
        }
    }
}

// ── ParallelExtractTool ─────────────────────────────────────────────

/// Extract content from web pages via the Parallel API.
pub struct ParallelExtractTool {
    client: Arc<IntegrationClient>,
}

impl ParallelExtractTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

/// Maximum characters of full_content to include per URL in tool output.
const MAX_CONTENT_CHARS: usize = 5000;

#[async_trait]
impl Tool for ParallelExtractTool {
    fn name(&self) -> &str {
        "parallel_extract"
    }

    fn description(&self) -> &str {
        "Extract content from one or more web pages using the Parallel API. \
         Returns page titles, excerpts, and optionally full content. \
         Useful for reading articles, documentation, or structured data from URLs. \
         Cost is per URL, billed by the backend."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "urls": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "URLs to extract content from (1-20)",
                    "minItems": 1,
                    "maxItems": 20
                },
                "objective": {
                    "type": "string",
                    "description": "What information to focus on when extracting"
                },
                "excerpts": {
                    "type": "boolean",
                    "description": "Include relevant excerpts (default true)",
                    "default": true
                },
                "full_content": {
                    "type": "boolean",
                    "description": "Include full page content (default false)",
                    "default": false
                }
            },
            "required": ["urls"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let urls = args
            .get("urls")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: urls"))?;

        if urls.is_empty() {
            return Ok(ToolResult::error("urls must contain at least one URL"));
        }

        let mut url_strings: Vec<&str> = Vec::with_capacity(urls.len());
        for (i, v) in urls.iter().enumerate() {
            match v.as_str() {
                Some(s) if !s.trim().is_empty() => url_strings.push(s),
                Some(_) => {
                    return Ok(ToolResult::error(format!("urls[{i}] is an empty string")));
                }
                None => {
                    return Ok(ToolResult::error(format!("urls[{i}] is not a string")));
                }
            }
        }

        let objective = args.get("objective").and_then(|v| v.as_str());
        let excerpts = args
            .get("excerpts")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let full_content = args
            .get("full_content")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut body = json!({
            "urls": url_strings,
            "excerpts": excerpts,
            "fullContent": full_content,
        });

        if let Some(obj) = objective {
            body["objective"] = json!(obj);
        }

        tracing::info!("[parallel_extract] urls={}", url_strings.len());

        match self
            .client
            .post::<ExtractResponse>("/agent-integrations/parallel/extract", &body)
            .await
        {
            Ok(resp) => {
                let mut lines = Vec::new();

                for (i, item) in resp.results.iter().enumerate() {
                    let title = item.title.as_deref().unwrap_or("(no title)");
                    lines.push(format!("\n{}. {} — {}", i + 1, title, item.url));

                    for excerpt in &item.excerpts {
                        let text = excerpt.trim();
                        if !text.is_empty() {
                            let (slice, was_truncated) = truncate_chars(text, 500);
                            let truncated = if was_truncated {
                                format!("{slice}...")
                            } else {
                                slice.to_string()
                            };
                            lines.push(format!("   {}", truncated));
                        }
                    }

                    if let Some(ref content) = item.full_content {
                        let content = content.trim();
                        if !content.is_empty() {
                            let (slice, was_truncated) = truncate_chars(content, MAX_CONTENT_CHARS);
                            let truncated = if was_truncated {
                                format!(
                                    "{}... [truncated, {} chars total]",
                                    slice,
                                    content.chars().count()
                                )
                            } else {
                                slice.to_string()
                            };
                            lines.push(format!("   Content:\n   {}", truncated));
                        }
                    }
                }

                if !resp.errors.is_empty() {
                    lines.push("\nErrors:".to_string());
                    for err in &resp.errors {
                        lines.push(format!("  {} — {}", err.url, err.error));
                    }
                }

                if lines.is_empty() {
                    Ok(ToolResult::success(format!(
                        "No content extracted from the provided URLs.\nCost: ${:.4}",
                        resp.cost_usd
                    )))
                } else {
                    lines.push(format!("\nCost: ${:.4}", resp.cost_usd));
                    Ok(ToolResult::success(lines.join("\n")))
                }
            }
            Err(e) => Ok(ToolResult::error(format!("Parallel extract failed: {e}"))),
        }
    }
}

#[cfg(test)]
#[path = "parallel_tests.rs"]
mod tests;
