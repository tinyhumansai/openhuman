use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use regex::Regex;
use serde_json::json;
use std::time::Duration;

/// Web search tool for searching the internet.
/// Supports multiple providers: DuckDuckGo (free), Brave (requires API key), Parallel (requires API key).
pub struct WebSearchTool {
    provider: String,
    brave_api_key: Option<String>,
    parallel_api_key: Option<String>,
    max_results: usize,
    timeout_secs: u64,
}

impl WebSearchTool {
    pub fn new(
        provider: String,
        brave_api_key: Option<String>,
        parallel_api_key: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        Self {
            provider: provider.trim().to_lowercase(),
            brave_api_key,
            parallel_api_key,
            max_results: max_results.clamp(1, 10),
            timeout_secs: timeout_secs.max(1),
        }
    }

    async fn search_duckduckgo(&self, query: &str) -> anyhow::Result<String> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("https://html.duckduckgo.com/html/?q={}", encoded_query);

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()?;

        let response = client.get(&search_url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!(
                "DuckDuckGo search failed with status: {}",
                response.status()
            );
        }

        let html = response.text().await?;
        self.parse_duckduckgo_results(&html, query)
    }

    fn parse_duckduckgo_results(&self, html: &str, query: &str) -> anyhow::Result<String> {
        // Extract result links: <a class="result__a" href="...">Title</a>
        let link_regex = Regex::new(
            r#"<a[^>]*class="[^"]*result__a[^"]*"[^>]*href="([^"]+)"[^>]*>([\s\S]*?)</a>"#,
        )?;

        // Extract snippets: <a class="result__snippet">...</a>
        let snippet_regex = Regex::new(r#"<a class="result__snippet[^"]*"[^>]*>([\s\S]*?)</a>"#)?;

        let link_matches: Vec<_> = link_regex
            .captures_iter(html)
            .take(self.max_results + 2)
            .collect();

        let snippet_matches: Vec<_> = snippet_regex
            .captures_iter(html)
            .take(self.max_results + 2)
            .collect();

        if link_matches.is_empty() {
            return Ok(format!("No results found for: {}", query));
        }

        let mut lines = vec![format!("Search results for: {} (via DuckDuckGo)", query)];

        let count = link_matches.len().min(self.max_results);

        for i in 0..count {
            let caps = &link_matches[i];
            let url_str = decode_ddg_redirect_url(&caps[1]);
            let title = strip_tags(&caps[2]);

            lines.push(format!("{}. {}", i + 1, title.trim()));
            lines.push(format!("   {}", url_str.trim()));

            // Add snippet if available
            if i < snippet_matches.len() {
                let snippet = strip_tags(&snippet_matches[i][1]);
                let snippet = snippet.trim();
                if !snippet.is_empty() {
                    lines.push(format!("   {}", snippet));
                }
            }
        }

        Ok(lines.join("\n"))
    }

    async fn search_brave(&self, query: &str) -> anyhow::Result<String> {
        let api_key = self
            .brave_api_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Brave API key not configured"))?;

        let encoded_query = urlencoding::encode(query);
        let search_url = format!(
            "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
            encoded_query, self.max_results
        );

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .build()?;

        let response = client
            .get(&search_url)
            .header("Accept", "application/json")
            .header("X-Subscription-Token", api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Brave search failed with status: {}", response.status());
        }

        let json: serde_json::Value = response.json().await?;
        self.parse_brave_results(&json, query)
    }

    async fn search_parallel(&self, query: &str) -> anyhow::Result<String> {
        let api_key = self
            .parallel_api_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Parallel API key not configured"))?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .build()?;

        let body = json!({
            "objective": query,
            "search_queries": [query],
            "mode": "fast",
            "excerpts": {
                "max_chars_per_result": 10000
            }
        });

        tracing::debug!(
            "[web_search] parallel: POST /v1beta/search for query={:?}",
            query
        );

        let response = client
            .post("https://api.parallel.ai/v1beta/search")
            .header("Content-Type", "application/json")
            .header("x-api-key", api_key)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            // Consume body without logging it — it may echo auth metadata.
            let _ = response.bytes().await;
            anyhow::bail!("Parallel search failed with status: {}", status);
        }

        let json: serde_json::Value = response.json().await?;
        self.parse_parallel_results(&json, query)
    }

    fn parse_parallel_results(
        &self,
        json: &serde_json::Value,
        query: &str,
    ) -> anyhow::Result<String> {
        let results = json
            .get("results")
            .and_then(|r| r.as_array())
            .ok_or_else(|| anyhow::anyhow!("Invalid Parallel API response: missing 'results'"))?;

        if results.is_empty() {
            return Ok(format!("No results found for: {}", query));
        }

        let mut lines = vec![format!("Search results for: {} (via Parallel)", query)];

        for (i, result) in results.iter().take(self.max_results).enumerate() {
            let title = result
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("No title");
            let url = result.get("url").and_then(|u| u.as_str()).unwrap_or("");

            lines.push(format!("{}. {}", i + 1, title));
            lines.push(format!("   {}", url));

            // Include the first excerpt (evidence-oriented text, LLM-ready).
            if let Some(excerpts) = result.get("excerpts").and_then(|e| e.as_array()) {
                if let Some(first) = excerpts.first().and_then(|e| e.as_str()) {
                    let excerpt = first.trim();
                    if !excerpt.is_empty() {
                        // Truncate very long excerpts to keep tool output reasonable.
                        let truncated = if excerpt.len() > 500 {
                            format!("{}...", &excerpt[..500])
                        } else {
                            excerpt.to_string()
                        };
                        lines.push(format!("   {}", truncated));
                    }
                }
            }
        }

        Ok(lines.join("\n"))
    }

    fn parse_brave_results(&self, json: &serde_json::Value, query: &str) -> anyhow::Result<String> {
        let results = json
            .get("web")
            .and_then(|w| w.get("results"))
            .and_then(|r| r.as_array())
            .ok_or_else(|| anyhow::anyhow!("Invalid Brave API response"))?;

        if results.is_empty() {
            return Ok(format!("No results found for: {}", query));
        }

        let mut lines = vec![format!("Search results for: {} (via Brave)", query)];

        for (i, result) in results.iter().take(self.max_results).enumerate() {
            let title = result
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("No title");
            let url = result.get("url").and_then(|u| u.as_str()).unwrap_or("");
            let description = result
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");

            lines.push(format!("{}. {}", i + 1, title));
            lines.push(format!("   {}", url));
            if !description.is_empty() {
                lines.push(format!("   {}", description));
            }
        }

        Ok(lines.join("\n"))
    }
}

fn decode_ddg_redirect_url(raw_url: &str) -> String {
    if let Some(index) = raw_url.find("uddg=") {
        let encoded = &raw_url[index + 5..];
        let encoded = encoded.split('&').next().unwrap_or(encoded);
        if let Ok(decoded) = urlencoding::decode(encoded) {
            return decoded.into_owned();
        }
    }

    raw_url.to_string()
}

fn strip_tags(content: &str) -> String {
    let re = Regex::new(r"<[^>]+>").unwrap();
    re.replace_all(content, "").to_string()
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search_tool"
    }

    fn description(&self) -> &str {
        "Search the web for information. Returns relevant search results with titles, URLs, and descriptions. Use this to find current information, news, or research topics."
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

        tracing::info!("Searching web for: {}", query);

        let result = match self.provider.as_str() {
            "duckduckgo" | "ddg" => self.search_duckduckgo(query).await?,
            "brave" => self.search_brave(query).await?,
            "parallel" => self.search_parallel(query).await?,
            _ => anyhow::bail!("Unknown search provider: {}", self.provider),
        };

        Ok(ToolResult::success(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ddg_tool() -> WebSearchTool {
        WebSearchTool::new("duckduckgo".to_string(), None, None, 5, 15)
    }

    #[test]
    fn test_tool_name() {
        assert_eq!(ddg_tool().name(), "web_search_tool");
    }

    #[test]
    fn test_tool_description() {
        assert!(ddg_tool().description().contains("Search the web"));
    }

    #[test]
    fn test_parameters_schema() {
        let schema = ddg_tool().parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["query"].is_object());
    }

    #[test]
    fn test_strip_tags() {
        let html = "<b>Hello</b> <i>World</i>";
        assert_eq!(strip_tags(html), "Hello World");
    }

    #[test]
    fn test_parse_duckduckgo_results_empty() {
        let result = ddg_tool()
            .parse_duckduckgo_results("<html>No results here</html>", "test")
            .unwrap();
        assert!(result.contains("No results found"));
    }

    #[test]
    fn test_parse_duckduckgo_results_with_data() {
        let html = r#"
            <a class="result__a" href="https://example.com">Example Title</a>
            <a class="result__snippet">This is a description</a>
        "#;
        let result = ddg_tool().parse_duckduckgo_results(html, "test").unwrap();
        assert!(result.contains("Example Title"));
        assert!(result.contains("https://example.com"));
    }

    #[test]
    fn test_parse_duckduckgo_results_decodes_redirect_url() {
        let html = r#"
            <a class="result__a" href="https://duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpath%3Fa%3D1&amp;rut=test">Example Title</a>
            <a class="result__snippet">This is a description</a>
        "#;
        let result = ddg_tool().parse_duckduckgo_results(html, "test").unwrap();
        assert!(result.contains("https://example.com/path?a=1"));
        assert!(!result.contains("rut=test"));
    }

    #[test]
    fn test_constructor_clamps_web_search_limits() {
        let tool = WebSearchTool::new("duckduckgo".to_string(), None, None, 0, 0);
        let html = r#"
            <a class="result__a" href="https://example.com">Example Title</a>
            <a class="result__snippet">This is a description</a>
        "#;
        let result = tool.parse_duckduckgo_results(html, "test").unwrap();
        assert!(result.contains("Example Title"));
    }

    #[tokio::test]
    async fn test_execute_missing_query() {
        let result = ddg_tool().execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_empty_query() {
        let result = ddg_tool().execute(json!({"query": ""})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_brave_without_api_key() {
        let tool = WebSearchTool::new("brave".to_string(), None, None, 5, 15);
        let result = tool.execute(json!({"query": "test"})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("API key"));
    }

    // --- Parallel provider tests (mocked) ---

    #[tokio::test]
    async fn test_execute_parallel_without_api_key() {
        let tool = WebSearchTool::new("parallel".to_string(), None, None, 5, 15);
        let result = tool.execute(json!({"query": "test"})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("API key"));
    }

    #[test]
    fn test_parse_parallel_results_empty() {
        let tool = WebSearchTool::new("parallel".to_string(), None, Some("key".to_string()), 5, 15);
        let json = serde_json::json!({ "results": [] });
        let result = tool.parse_parallel_results(&json, "test query").unwrap();
        assert!(result.contains("No results found"));
    }

    #[test]
    fn test_parse_parallel_results_with_data() {
        let tool = WebSearchTool::new("parallel".to_string(), None, Some("key".to_string()), 5, 15);
        let json = serde_json::json!({
            "search_id": "abc-123",
            "results": [
                {
                    "title": "Parallel AI Docs",
                    "url": "https://docs.parallel.ai/home",
                    "publish_date": null,
                    "excerpts": ["Parallel provides infrastructure for AI web search."]
                },
                {
                    "title": "Parallel Search Quickstart",
                    "url": "https://docs.parallel.ai/search",
                    "publish_date": "2024-01-01",
                    "excerpts": ["Use POST /v1beta/search to retrieve results."]
                }
            ],
            "warnings": null,
            "usage": [{ "name": "search", "count": 1 }]
        });
        let result = tool.parse_parallel_results(&json, "parallel ai").unwrap();
        assert!(result.contains("via Parallel"));
        assert!(result.contains("Parallel AI Docs"));
        assert!(result.contains("https://docs.parallel.ai/home"));
        assert!(result.contains("infrastructure for AI web search"));
        assert!(result.contains("Parallel Search Quickstart"));
    }

    #[test]
    fn test_parse_parallel_results_respects_max_results() {
        let tool = WebSearchTool::new(
            "parallel".to_string(),
            None,
            Some("key".to_string()),
            2, // max_results = 2
            15,
        );
        let json = serde_json::json!({
            "results": [
                { "title": "Result 1", "url": "https://a.com", "excerpts": [] },
                { "title": "Result 2", "url": "https://b.com", "excerpts": [] },
                { "title": "Result 3", "url": "https://c.com", "excerpts": [] }
            ]
        });
        let result = tool.parse_parallel_results(&json, "q").unwrap();
        assert!(result.contains("Result 1"));
        assert!(result.contains("Result 2"));
        assert!(!result.contains("Result 3"));
    }

    #[test]
    fn test_parse_parallel_results_missing_results_field() {
        let tool = WebSearchTool::new("parallel".to_string(), None, Some("key".to_string()), 5, 15);
        let json = serde_json::json!({ "search_id": "abc" });
        let result = tool.parse_parallel_results(&json, "q");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("missing 'results'"));
    }

    #[test]
    fn test_parse_parallel_results_truncates_long_excerpt() {
        let tool = WebSearchTool::new("parallel".to_string(), None, Some("key".to_string()), 5, 15);
        let long_excerpt = "x".repeat(600);
        let json = serde_json::json!({
            "results": [{
                "title": "T",
                "url": "https://t.com",
                "excerpts": [long_excerpt]
            }]
        });
        let result = tool.parse_parallel_results(&json, "q").unwrap();
        // Should contain truncated text ending with "..."
        assert!(result.contains("..."));
        // The excerpt portion in the output should not exceed 503 chars ("x"*500 + "...")
        let excerpt_line = result.lines().find(|l| l.trim().starts_with('x')).unwrap();
        assert!(excerpt_line.trim().len() <= 503);
    }
}
