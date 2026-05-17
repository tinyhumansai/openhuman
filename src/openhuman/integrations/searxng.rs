//! SearXNG search integration for self-hosted, private web search.
//!
//! SearXNG exposes JSON results from `GET /search?format=json`. This wrapper
//! keeps the output shape small and stable for agents and MCP clients:
//! `{ title, url, snippet, source }`.

use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolCategory, ToolResult};
use crate::openhuman::util::utf8_safe_prefix_at_byte_boundary;
use anyhow::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

const DEFAULT_SOURCE: &str = "searxng";
const MAX_RESULTS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearxngSearchArgs {
    pub query: String,
    pub categories: Vec<String>,
    pub language: Option<String>,
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearxngSearchResponse {
    pub query: String,
    pub results: Vec<SearxngSearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearxngSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: String,
}

#[derive(Debug, Deserialize)]
struct RawSearxngResponse {
    #[serde(default)]
    results: Vec<RawSearxngResult>,
}

#[derive(Debug, Deserialize)]
struct RawSearxngResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
    #[serde(default)]
    engine: Option<String>,
    #[serde(default)]
    engines: Vec<String>,
}

pub struct SearxngSearchTool {
    base_url: String,
    max_results: usize,
    default_language: String,
    timeout_secs: u64,
    http_client: reqwest::Client,
}

impl SearxngSearchTool {
    pub fn new(
        base_url: String,
        max_results: usize,
        default_language: String,
        timeout_secs: u64,
    ) -> Self {
        let timeout = timeout_secs.max(1);
        let http_client = reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(Duration::from_secs(timeout))
            .connect_timeout(Duration::from_secs(timeout.min(10)))
            .build()
            .expect("failed to build SearXNG HTTP client");

        Self {
            base_url,
            max_results: max_results.clamp(1, MAX_RESULTS),
            default_language,
            timeout_secs: timeout,
            http_client,
        }
    }

    pub async fn search(&self, args: SearxngSearchArgs) -> anyhow::Result<SearxngSearchResponse> {
        let query = args.query.trim();
        if query.is_empty() {
            anyhow::bail!("SearXNG search query cannot be empty");
        }

        let max_results = args
            .max_results
            .unwrap_or(self.max_results)
            .clamp(1, MAX_RESULTS);
        let categories = normalize_categories(args.categories)?;
        let language = args
            .language
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.default_language.trim());

        let mut url = self.search_endpoint_url()?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", query);
            pairs.append_pair("format", "json");
            if !categories.is_empty() {
                pairs.append_pair("categories", &categories.join(","));
            }
            if !language.is_empty() {
                pairs.append_pair("language", language);
            }
        }

        tracing::debug!(
            query_len = query.chars().count(),
            max_results,
            categories = ?categories,
            timeout_secs = self.timeout_secs,
            "[searxng] GET /search"
        );

        let response = self
            .http_client
            .get(url.clone())
            .send()
            .await
            .map_err(|err| {
                tracing::warn!(error = %err, "[searxng] request failed");
                anyhow::anyhow!("SearXNG request failed: {err}")
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail = utf8_safe_prefix_at_byte_boundary(&body, 500);
            tracing::warn!(status = %status, "[searxng] non-2xx response: {detail}");
            anyhow::bail!("SearXNG returned {status}: {detail}");
        }

        let raw: RawSearxngResponse = response.json().await.map_err(|err| {
            tracing::warn!(error = %err, "[searxng] failed to parse JSON response");
            anyhow::anyhow!("Failed to parse SearXNG response: {err}")
        })?;

        let results = normalize_results(raw, max_results);
        tracing::debug!(result_count = results.len(), "[searxng] search complete");

        Ok(SearxngSearchResponse {
            query: query.to_string(),
            results,
        })
    }

    fn search_endpoint_url(&self) -> anyhow::Result<reqwest::Url> {
        let base = self.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            anyhow::bail!("SearXNG base_url cannot be empty");
        }
        let endpoint = if base.ends_with("/search") {
            base.to_string()
        } else {
            format!("{base}/search")
        };
        reqwest::Url::parse(&endpoint)
            .with_context(|| format!("invalid SearXNG base_url `{}`", self.base_url))
    }

    fn render_markdown(response: &SearxngSearchResponse) -> String {
        if response.results.is_empty() {
            return format!("No SearXNG results for `{}`.", response.query);
        }

        let mut out = format!("# SearXNG results for `{}`\n", response.query);
        for (index, result) in response.results.iter().enumerate() {
            out.push_str(&format!(
                "\n{}. [{}]({})\n",
                index + 1,
                result.title,
                result.url
            ));
            if !result.snippet.is_empty() {
                out.push_str(&format!("   {}\n", result.snippet));
            }
            out.push_str(&format!("   Source: {}\n", result.source));
        }
        out
    }
}

#[async_trait]
impl Tool for SearxngSearchTool {
    fn name(&self) -> &str {
        "searxng_search"
    }

    fn description(&self) -> &str {
        "Search a user-configured SearXNG instance. Returns private, self-hosted web search results normalized as title, URL, snippet, and source."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string."
                },
                "categories": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["web", "general", "news", "images"]
                    },
                    "description": "Optional SearXNG categories. `web` maps to SearXNG `general`."
                },
                "language": {
                    "type": "string",
                    "description": "Optional language code, e.g. `en`, `zh-CN`, `fr`."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_RESULTS,
                    "description": "Maximum number of results to return."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
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
        let search_args = parse_search_args(args)?;
        let response = self.search(search_args).await?;
        let payload = json!({
            "query": response.query,
            "results": response.results,
        });
        let markdown = Self::render_markdown(&response);
        let mut result = ToolResult::json(payload);
        if options.prefer_markdown {
            result.markdown_formatted = Some(markdown);
        }
        Ok(result)
    }
}

pub fn normalize_categories(categories: Vec<String>) -> anyhow::Result<Vec<String>> {
    let mut normalized = Vec::new();
    for category in categories {
        let trimmed = category.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mapped = match trimmed.to_ascii_lowercase().as_str() {
            "web" | "general" => "general",
            "news" => "news",
            "images" => "images",
            other => anyhow::bail!(
                "unsupported SearXNG category `{other}`; expected web, news, or images"
            ),
        };
        if !normalized.iter().any(|existing| existing == mapped) {
            normalized.push(mapped.to_string());
        }
    }
    Ok(normalized)
}

fn normalize_results(raw: RawSearxngResponse, max_results: usize) -> Vec<SearxngSearchResult> {
    raw.results
        .into_iter()
        .filter_map(|item| {
            let url = item.url.unwrap_or_default().trim().to_string();
            if url.is_empty() {
                return None;
            }
            let title = item.title.unwrap_or_default().trim().to_string();
            let snippet = item
                .content
                .or(item.snippet)
                .unwrap_or_default()
                .trim()
                .to_string();
            let source = item
                .engine
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    item.engines
                        .into_iter()
                        .find(|value| !value.trim().is_empty())
                })
                .unwrap_or_else(|| DEFAULT_SOURCE.to_string())
                .trim()
                .to_string();
            Some(SearxngSearchResult {
                title: if title.is_empty() { url.clone() } else { title },
                url,
                snippet,
                source,
            })
        })
        .take(max_results)
        .collect()
}

fn parse_search_args(args: serde_json::Value) -> anyhow::Result<SearxngSearchArgs> {
    let object = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("SearXNG arguments must be an object"))?;
    let query = object
        .get("query")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?
        .to_string();
    let categories = match object.get("categories") {
        Some(value) if value.is_null() => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("categories must be an array of strings"))?
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("categories must contain only strings"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    let language = object
        .get("language")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let max_results = object
        .get("max_results")
        .and_then(|value| value.as_u64())
        .map(|value| value.clamp(1, MAX_RESULTS as u64) as usize);

    Ok(SearxngSearchArgs {
        query,
        categories,
        language,
        max_results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(base_url: String) -> SearxngSearchTool {
        SearxngSearchTool::new(base_url, 10, "en".into(), 5)
    }

    #[test]
    fn normalizes_categories_and_maps_web_to_general() {
        let categories = normalize_categories(vec![
            "web".into(),
            "news".into(),
            "general".into(),
            " images ".into(),
        ])
        .expect("categories");
        assert_eq!(categories, vec!["general", "news", "images"]);
    }

    #[test]
    fn rejects_unknown_category() {
        let err = normalize_categories(vec!["videos".into()]).expect_err("must reject");
        assert!(err.to_string().contains("unsupported SearXNG category"));
    }

    #[test]
    fn parameters_schema_includes_mcp_expected_fields() {
        let schema = tool("http://localhost:8080".into()).parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["query"].is_object());
        assert!(schema["properties"]["categories"].is_object());
        assert!(schema["properties"]["language"].is_object());
        assert!(schema["properties"]["max_results"].is_object());
    }

    #[tokio::test]
    async fn search_calls_json_endpoint_and_normalizes_results() {
        use axum::{extract::Query, routing::get, Json, Router};
        use std::collections::HashMap;

        let app = Router::new().route(
            "/search",
            get(|Query(params): Query<HashMap<String, String>>| async move {
                assert_eq!(params.get("q").map(String::as_str), Some("test query"));
                assert_eq!(params.get("format").map(String::as_str), Some("json"));
                assert_eq!(
                    params.get("categories").map(String::as_str),
                    Some("general,news")
                );
                assert_eq!(params.get("language").map(String::as_str), Some("en"));
                Json(json!({
                    "results": [
                        {
                            "title": "First result",
                            "url": "https://example.com/one",
                            "content": "A useful snippet.",
                            "engine": "duckduckgo"
                        },
                        {
                            "title": "Missing URL should be skipped",
                            "content": "No URL"
                        },
                        {
                            "url": "https://example.com/two",
                            "snippet": "Fallback snippet.",
                            "engines": ["brave"]
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

        let response = tool(format!("http://127.0.0.1:{}", addr.port()))
            .search(SearxngSearchArgs {
                query: " test query ".into(),
                categories: vec!["web".into(), "news".into()],
                language: None,
                max_results: Some(5),
            })
            .await
            .expect("search");

        assert_eq!(response.query, "test query");
        assert_eq!(response.results.len(), 2);
        assert_eq!(response.results[0].title, "First result");
        assert_eq!(response.results[0].source, "duckduckgo");
        assert_eq!(response.results[1].title, "https://example.com/two");
        assert_eq!(response.results[1].snippet, "Fallback snippet.");
        assert_eq!(response.results[1].source, "brave");
    }
}
