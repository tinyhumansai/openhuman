use crate::openhuman::integrations::parallel::{SearchResponse, SearchResultItem};
use crate::openhuman::integrations::IntegrationClient;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Web search tool backed by the server-side Parallel integration proxy.
pub struct WebSearchTool {
    client: Option<Arc<IntegrationClient>>,
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
            max_results: max_results.clamp(1, 10),
            timeout_secs: timeout_secs.max(1),
        }
    }

    fn parse_parallel_results(
        &self,
        results: &[SearchResultItem],
        query: &str,
    ) -> anyhow::Result<String> {
        if results.is_empty() {
            return Ok(format!("No results found for: {}", query));
        }

        let mut lines = vec![format!(
            "Search results for: {} (via backend Parallel)",
            query
        )];

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
                    let truncated = if let Some((idx, _)) = excerpt.char_indices().nth(500) {
                        format!("{}...", &excerpt[..idx])
                    } else {
                        excerpt.to_string()
                    };
                    lines.push(format!("   {}", truncated));
                }
            }
        }

        Ok(lines.join("\n"))
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search_tool"
    }

    fn description(&self) -> &str {
        "Search the web for information via the backend search proxy. Returns relevant search results with titles, URLs, and descriptions."
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
        let query = args
            .get("query")
            .and_then(|q| q.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?;

        if query.trim().is_empty() {
            anyhow::bail!("Search query cannot be empty");
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
            "objective": query,
            "searchQueries": [query],
            "mode": "fast",
            "excerpts": {
                "maxResults": self.max_results,
                "maxCharsPerResult": 500
            }
        });

        let resp = client
            .post::<SearchResponse>("/agent-integrations/parallel/search", &body)
            .await?;

        Ok(ToolResult::success(
            self.parse_parallel_results(&resp.results, query)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, routing::post, Json, Router};
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
        let result = tool().parse_parallel_results(&[], "test query").unwrap();
        assert!(result.contains("No results found"));
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
            .parse_parallel_results(&results, "parallel ai")
            .unwrap();
        assert!(result.contains("via backend Parallel"));
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
        let result = tool.parse_parallel_results(&results, "q").unwrap();
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
        let result = tool().parse_parallel_results(&results, "q").unwrap();
        assert!(result.contains("..."));
        let excerpt_line = result.lines().find(|l| l.trim().starts_with('x')).unwrap();
        assert!(excerpt_line.trim().len() <= 503);
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
    }
}
