use super::{SearchResponse, SearchResultItem, SeltzSearchTool};
use crate::openhuman::integrations::IntegrationClient;
use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Provider the OpenHuman Managed search path resolves to today. Exa powers
/// the overwhelming majority of managed search traffic, so it is the labelled
/// default whenever the backend response does not name a provider. This is a
/// *fallback* label, not a hardcoded one: [`resolve_managed_provider`] prefers
/// the provider the backend actually reports, so a future routing change flows
/// through to the UI attribution ("Searched with …", #5136) with no code edit.
const MANAGED_DEFAULT_PROVIDER: &str = "Exa";

/// Resolve the provider name to attribute a managed search to. Uses the
/// backend-reported provider when present and non-empty, otherwise falls back
/// to [`MANAGED_DEFAULT_PROVIDER`]. Shared with the `tools.web_search` RPC so
/// both managed-search surfaces attribute a call the same way.
pub(crate) fn resolve_managed_provider(resp: &SearchResponse) -> &str {
    resp.provider
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or(MANAGED_DEFAULT_PROVIDER)
}

/// Web search tool backed by the server-side Parallel integration proxy.
pub struct WebSearchTool {
    client: Option<Arc<IntegrationClient>>,
    direct_search: Option<SeltzSearchTool>,
    max_results: usize,
    timeout_secs: u64,
}

impl WebSearchTool {
    pub fn new(
        client: Option<Arc<IntegrationClient>>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        Self {
            client,
            direct_search: None,
            max_results: max_results.clamp(1, 10),
            timeout_secs: timeout_secs.max(1),
        }
    }

    pub fn with_direct_search(mut self, direct_search: Option<SeltzSearchTool>) -> Self {
        self.direct_search = direct_search;
        self
    }

    fn parse_parallel_results(
        &self,
        results: &[SearchResultItem],
        query: &str,
        provider: &str,
    ) -> anyhow::Result<String> {
        if results.is_empty() {
            // Still attribute an empty search: the call completed, so the
            // timeline must not keep showing it as in-progress (#5136).
            return Ok(format!(
                "No results found for: {} (via {})",
                query, provider
            ));
        }

        let mut lines = vec![format!("Search results for: {} (via {})", query, provider)];

        for (i, result) in results.iter().take(self.max_results).enumerate() {
            let title = if result.title.trim().is_empty() {
                "No title"
            } else {
                result.title.trim()
            };
            let url = result.url.trim();

            lines.push(format!("{}. {}", i + 1, title));
            lines.push(format!("   {}", url));

            if let Some(date) = result.publish_date.as_deref() {
                let date = date.trim();
                if !date.is_empty() {
                    lines.push(format!("   Published: {}", date));
                }
            }

            if let Some(first) = result.excerpts.first() {
                let excerpt = first.trim();
                if !excerpt.is_empty() {
                    let truncated = crate::openhuman::util::truncate_with_ellipsis(excerpt, 500);
                    lines.push(format!("   {}", truncated));
                }
            }
        }

        Ok(lines.join("\n"))
    }

    fn render_results_markdown(
        &self,
        results: &[SearchResultItem],
        query: &str,
        provider: &str,
    ) -> String {
        if results.is_empty() {
            return format!("_No results for `{query}`_ (via {provider})");
        }
        let mut out = format!("# Search results — `{query}` (via {provider})\n");
        for r in results.iter().take(self.max_results) {
            let title = if r.title.trim().is_empty() {
                "Untitled"
            } else {
                r.title.trim()
            };
            out.push_str(&format!("\n## [{title}]({})\n", r.url.trim()));
            if let Some(date) = r.publish_date.as_deref() {
                let date = date.trim();
                if !date.is_empty() {
                    out.push_str(&format!("_Published: {date}_\n\n"));
                }
            }
            if let Some(first) = r.excerpts.first() {
                let excerpt = first.trim();
                if !excerpt.is_empty() {
                    let truncated = crate::openhuman::util::truncate_with_suffix(excerpt, 500, "…");
                    out.push_str(&format!("> {truncated}\n"));
                }
            }
        }
        out
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search_tool"
    }

    fn description(&self) -> &str {
        "Search the web for information via a configured direct search API or the backend search proxy. Returns relevant search results with titles, URLs, and descriptions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query. Be specific for better results."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    fn supports_markdown(&self) -> bool {
        true
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
        let query = query.trim().to_string();

        if let Some(direct_search) = &self.direct_search {
            tracing::debug!(
                query_len = query.chars().count(),
                max_results = self.max_results,
                timeout_secs = self.timeout_secs,
                "[web_search] direct Seltz search"
            );
            let mut normalized_args = args;
            if let Some(obj) = normalized_args.as_object_mut() {
                obj.insert("query".to_string(), Value::String(query.clone()));
            }
            return direct_search
                .execute_with_options(normalized_args, options)
                .await;
        }

        let client = self.client.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "Web search unavailable: no backend session token. Sign in first so the server can proxy search."
            )
        })?;

        let query_fingerprint = hex::encode(Sha256::digest(query.as_bytes()));
        tracing::debug!(
            query_len = query.chars().count(),
            query_fingerprint = %query_fingerprint[..16],
            max_results = self.max_results,
            timeout_secs = self.timeout_secs,
            "[web_search] backend parallel search"
        );

        // Body matches `parallelSearchSchema` in backend-2. The legacy
        // `numResults` / `maxCharactersPerExcerpt` aliases still work, but
        // current fields are `maxResults` / `maxCharsPerResult`. Also dropping
        // `timeoutSecs` — the validator does not declare it and Parallel's
        // per-mode deadlines drive timing on the upstream side.
        let _ = self.timeout_secs;
        let body = json!({
            "objective": query.clone(),
            "searchQueries": [query.clone()],
            "mode": "fast",
            "excerpts": {
                "maxResults": self.max_results,
                "maxCharsPerResult": 500
            }
        });

        let resp = client
            .post::<SearchResponse>("/agent-integrations/parallel/search", &body)
            .await?;

        // Attribute the search to the provider the managed backend resolved to
        // (Exa by default). The provider name is echoed in the result text so
        // the UI can surface "Searched with <provider>" without a hardcode.
        let provider = resolve_managed_provider(&resp);
        tracing::debug!(provider, "[web_search] managed search resolved provider");

        let mut result =
            ToolResult::success(self.parse_parallel_results(&resp.results, &query, provider)?);
        if options.prefer_markdown {
            result.markdown_formatted =
                Some(self.render_results_markdown(&resp.results, &query, provider));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
    use serde_json::Value;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn tool() -> WebSearchTool {
        WebSearchTool::new(None, 5, 15)
    }

    async fn start_mock_backend(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    #[test]
    fn test_tool_name() {
        assert_eq!(tool().name(), "web_search_tool");
    }

    #[test]
    fn test_tool_description() {
        assert!(tool().description().contains("backend search proxy"));
    }

    #[test]
    fn test_parameters_schema() {
        let schema = tool().parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["query"].is_object());
    }

    #[test]
    fn test_parse_parallel_results_empty() {
        let result = tool()
            .parse_parallel_results(&[], "test query", "Exa")
            .unwrap();
        assert!(result.contains("No results found"));
        // A completed empty search is still attributed, so the timeline labels
        // the row instead of leaving it as in-progress (#5136).
        assert!(result.trim_end().ends_with("(via Exa)"));
    }

    #[test]
    fn test_render_markdown_empty_carries_provider() {
        // The markdown rendering is what production shows, so its empty form
        // needs the marker too — and it must sit at the end of the line, where
        // the timeline parser looks for it.
        let result = tool().render_results_markdown(&[], "test query", "Exa");
        assert!(result.contains("No results"));
        assert!(result.trim_end().ends_with("(via Exa)"));
    }

    /// A minimal `SearchResponse` carrying only the provider under test, so
    /// the resolution cases read without result/cost noise.
    fn response_with_provider(provider: Option<&str>) -> SearchResponse {
        SearchResponse {
            search_id: "search-1".into(),
            results: vec![],
            cost_usd: 0.0,
            provider: provider.map(str::to_string),
        }
    }

    #[test]
    fn test_resolve_managed_provider_defaults_to_exa() {
        // Backend omits the provider → fall back to the managed default.
        assert_eq!(
            resolve_managed_provider(&response_with_provider(None)),
            "Exa"
        );
        // Blank / whitespace-only provider is treated as absent.
        assert_eq!(
            resolve_managed_provider(&response_with_provider(Some("   "))),
            "Exa"
        );
    }

    #[test]
    fn test_resolve_managed_provider_uses_backend_value() {
        // A provider named by the backend wins over the default and is trimmed,
        // so a future routing change surfaces without a code edit.
        assert_eq!(
            resolve_managed_provider(&response_with_provider(Some("  Brave  "))),
            "Brave"
        );
    }

    #[test]
    fn test_parse_parallel_results_attribution_is_dynamic() {
        let results = vec![SearchResultItem {
            title: "T".into(),
            url: "https://t.com".into(),
            publish_date: None,
            excerpts: vec![],
        }];
        let exa = tool().parse_parallel_results(&results, "q", "Exa").unwrap();
        assert!(exa.contains("(via Exa)"));
        assert!(!exa.contains("via backend Parallel"));
        let brave = tool()
            .parse_parallel_results(&results, "q", "Brave")
            .unwrap();
        assert!(brave.contains("(via Brave)"));
    }

    #[test]
    fn test_parse_parallel_results_with_data() {
        let results = vec![
            SearchResultItem {
                title: "Parallel AI Docs".into(),
                url: "https://docs.parallel.ai/home".into(),
                publish_date: None,
                excerpts: vec!["Parallel provides infrastructure for AI web search.".into()],
            },
            SearchResultItem {
                title: "Parallel Search Quickstart".into(),
                url: "https://docs.parallel.ai/search".into(),
                publish_date: Some("2024-01-01".into()),
                excerpts: vec!["Use POST /v1beta/search to retrieve results.".into()],
            },
        ];

        let result = tool()
            .parse_parallel_results(&results, "parallel ai", "Exa")
            .unwrap();
        assert!(result.contains("(via Exa)"));
        assert!(result.contains("Parallel AI Docs"));
        assert!(result.contains("https://docs.parallel.ai/home"));
        assert!(result.contains("Parallel Search Quickstart"));
        assert!(result.contains("Published: 2024-01-01"));
    }

    #[test]
    fn test_parse_parallel_results_respects_max_results() {
        let tool = WebSearchTool::new(None, 2, 15);
        let results = vec![
            SearchResultItem {
                title: "Result 1".into(),
                url: "https://a.com".into(),
                publish_date: None,
                excerpts: vec![],
            },
            SearchResultItem {
                title: "Result 2".into(),
                url: "https://b.com".into(),
                publish_date: None,
                excerpts: vec![],
            },
            SearchResultItem {
                title: "Result 3".into(),
                url: "https://c.com".into(),
                publish_date: None,
                excerpts: vec![],
            },
        ];
        let result = tool.parse_parallel_results(&results, "q", "Exa").unwrap();
        assert!(result.contains("Result 1"));
        assert!(result.contains("Result 2"));
        assert!(!result.contains("Result 3"));
    }

    #[test]
    fn test_parse_parallel_results_truncates_long_excerpt() {
        let long_excerpt = "x".repeat(600);
        let results = vec![SearchResultItem {
            title: "T".into(),
            url: "https://t.com".into(),
            publish_date: None,
            excerpts: vec![long_excerpt],
        }];
        let result = tool().parse_parallel_results(&results, "q", "Exa").unwrap();
        assert!(result.contains("..."));
        let excerpt_line = result.lines().find(|l| l.trim().starts_with('x')).unwrap();
        assert!(excerpt_line.trim().len() <= 503);
    }

    #[test]
    fn test_web_search_truncation_utf8() {
        let excerpt = "🦀".repeat(600);
        let results = vec![SearchResultItem {
            title: "T".into(),
            url: "https://t.com".into(),
            publish_date: None,
            excerpts: vec![excerpt],
        }];
        let result = tool().parse_parallel_results(&results, "q", "Exa").unwrap();
        assert!(result.contains("..."));
        // Should have 500 crabs + "..."
        let excerpt_line = result.lines().find(|l| l.contains('🦀')).unwrap();
        assert_eq!(
            excerpt_line.trim().chars().filter(|c| *c == '🦀').count(),
            500
        );
    }

    #[tokio::test]
    async fn test_execute_missing_query() {
        let result = tool().execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_empty_query() {
        let result = tool().execute(json!({"query": ""})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_without_backend_client() {
        let result = tool().execute(json!({"query": "test"})).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("backend session token"));
    }

    #[tokio::test]
    async fn test_execute_posts_to_backend_and_renders_results() {
        #[derive(Clone)]
        struct MockState {
            called: Arc<AtomicBool>,
        }

        let state = MockState {
            called: Arc::new(AtomicBool::new(false)),
        };
        let called = Arc::clone(&state.called);
        let app = Router::new()
            .route(
                "/agent-integrations/parallel/search",
                post(
                    |State(state): State<MockState>, Json(body): Json<Value>| async move {
                        state.called.store(true, Ordering::SeqCst);
                        assert_eq!(body["objective"], "test success");
                        assert_eq!(body["searchQueries"][0], "test success");
                        Json(json!({
                            "success": true,
                            "data": {
                                "searchId": "search-123",
                                "results": [
                                    {
                                        "url": "https://example.com/result",
                                        "title": "Backend Search Result",
                                        "publish_date": "2026-04-20",
                                        "excerpts": ["Rendered excerpt from backend search."]
                                    }
                                ],
                                "costUsd": 0.01
                            }
                        }))
                    },
                ),
            )
            .with_state(state);

        let base_url = start_mock_backend(app).await;
        let client = Arc::new(IntegrationClient::new(base_url, "test-token".into()));
        let result = WebSearchTool::new(Some(client), 5, 15)
            .execute(json!({"query": "test success"}))
            .await
            .expect("execute() should return rendered backend results");

        assert!(called.load(Ordering::SeqCst));
        assert!(result.output().contains("Backend Search Result"));
        assert!(result.output().contains("https://example.com/result"));
        assert!(result
            .output()
            .contains("Rendered excerpt from backend search."));
        // Backend omitted a provider → attribution falls back to the managed
        // default (Exa) rather than the legacy "backend Parallel" wording.
        assert!(result.output().contains("(via Exa)"));
        assert!(!result.output().contains("backend Parallel"));
    }

    #[tokio::test]
    async fn test_execute_attributes_backend_reported_provider() {
        // When the backend names the resolved provider, the tool result echoes
        // it verbatim — the attribution is dynamic, not a hardcoded "Exa".
        let app = Router::new().route(
            "/agent-integrations/parallel/search",
            post(|Json(_body): Json<Value>| async move {
                Json(json!({
                    "success": true,
                    "data": {
                        "searchId": "search-xyz",
                        "provider": "Brave",
                        "results": [
                            {
                                "url": "https://example.com/r",
                                "title": "Result",
                                "excerpts": ["Excerpt."]
                            }
                        ],
                        "costUsd": 0.01
                    }
                }))
            }),
        );

        let base_url = start_mock_backend(app).await;
        let client = Arc::new(IntegrationClient::new(base_url, "test-token".into()));
        let result = WebSearchTool::new(Some(client), 5, 15)
            .execute(json!({"query": "anything"}))
            .await
            .expect("execute() should render backend results");

        assert!(result.output().contains("(via Brave)"));
        assert!(!result.output().contains("(via Exa)"));
    }

    #[tokio::test]
    async fn test_execute_uses_direct_search_api_when_configured() {
        #[derive(Clone)]
        struct MockState {
            called: Arc<AtomicBool>,
        }

        let state = MockState {
            called: Arc::new(AtomicBool::new(false)),
        };
        let called = Arc::clone(&state.called);
        let app = Router::new()
            .route(
                "/search",
                post(
                    |State(state): State<MockState>,
                     headers: HeaderMap,
                     Json(body): Json<Value>| async move {
                        state.called.store(true, Ordering::SeqCst);
                        assert_eq!(
                            headers.get("x-api-key").and_then(|v| v.to_str().ok()),
                            Some("test-key")
                        );
                        assert_eq!(body["query"], "direct search");
                        Json(json!({
                            "documents": [
                                {
                                    "url": "https://example.com/direct",
                                    "title": "Direct Search Result",
                                    "content": "Rendered excerpt from direct search.",
                                    "published_date": "2026-04-21"
                                }
                            ]
                        }))
                    },
                ),
            )
            .with_state(state);

        let base_url = start_mock_backend(app).await;
        let result = WebSearchTool::new(None, 5, 15)
            .with_direct_search(Some(SeltzSearchTool::new(
                Some("test-key".into()),
                Some(base_url),
                5,
                15,
            )))
            .execute(json!({"query": "direct search"}))
            .await
            .expect("execute() should return rendered direct search results");

        assert!(called.load(Ordering::SeqCst));
        assert!(result.output().contains("via Seltz"));
        assert!(result.output().contains("Direct Search Result"));
        assert!(result.output().contains("https://example.com/direct"));
    }
}
