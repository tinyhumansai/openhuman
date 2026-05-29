//! Querit web search integration -- direct API (not backend-proxied).
//!
//! **Scope**: Agent + CLI/RPC.
//!
//! **Endpoint**: `POST https://api.querit.ai/v1/search`
//!
//! **Auth**: `Authorization: Bearer <api key>`.
//!
//! Querit exposes an AI-oriented web search endpoint with optional site,
//! time-range, country, and language filters. This integration calls the
//! Querit API directly using the user's configured API key.

use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::time::Duration;

const DEFAULT_API_URL: &str = "https://api.querit.ai/v1";

#[derive(Debug, Deserialize, Serialize)]
pub struct QueritSearchResponse {
    #[serde(default)]
    pub took: String,
    #[serde(default)]
    pub error_code: i64,
    #[serde(default)]
    pub error_msg: String,
    #[serde(default)]
    pub search_id: i64,
    #[serde(default)]
    pub query_context: QueritQueryContext,
    #[serde(default)]
    pub results: QueritResults,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct QueritQueryContext {
    #[serde(default)]
    pub query: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct QueritResults {
    #[serde(default)]
    pub result: Vec<QueritResultItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QueritResultItem {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub page_age: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub snippet: Option<String>,
    #[serde(default)]
    pub site_name: Option<String>,
    #[serde(default)]
    pub site_icon: Option<String>,
    #[serde(default)]
    pub sentence: Vec<String>,
}

/// Real-time web search via the Querit API.
pub struct QueritSearchTool {
    tool_name: &'static str,
    api_key: Option<String>,
    api_url: String,
    max_results: usize,
    timeout_secs: u64,
    http_client: reqwest::Client,
}

impl QueritSearchTool {
    pub fn new(
        api_key: Option<String>,
        api_url: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        Self::with_name("querit_search", api_key, api_url, max_results, timeout_secs)
    }

    pub fn new_web_search_tool(
        api_key: Option<String>,
        api_url: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        Self::with_name(
            "web_search_tool",
            api_key,
            api_url,
            max_results,
            timeout_secs,
        )
    }

    fn with_name(
        tool_name: &'static str,
        api_key: Option<String>,
        api_url: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        let timeout = timeout_secs.max(1);
        let http_client = crate::openhuman::tls::tls_client_builder()
            .http1_only()
            .timeout(Duration::from_secs(timeout))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build Querit HTTP client");

        Self {
            tool_name,
            api_key,
            api_url: api_url.unwrap_or_else(|| DEFAULT_API_URL.to_string()),
            max_results: max_results.clamp(1, 20),
            timeout_secs: timeout,
            http_client,
        }
    }

    fn render_results_plain(&self, results: &[QueritResultItem], query: &str) -> String {
        if results.is_empty() {
            return format!("No results found for: {}", query);
        }

        let mut lines = vec![format!("Search results for: {} (via Querit)", query)];
        for (i, item) in results.iter().take(self.max_results).enumerate() {
            let title = item
                .title
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or("Untitled");
            lines.push(format!("{}. {}", i + 1, title));
            lines.push(format!("   {}", item.url.trim()));

            if let Some(age) = item.page_age.as_deref() {
                let age = age.trim();
                if !age.is_empty() {
                    lines.push(format!("   Page age: {}", age));
                }
            }
            if let Some(site) = item.site_name.as_deref() {
                let site = site.trim();
                if !site.is_empty() {
                    lines.push(format!("   Site: {}", site));
                }
            }
            if let Some(snippet) = item.snippet_text() {
                let truncated = crate::openhuman::util::truncate_with_ellipsis(&snippet, 500);
                lines.push(format!("   {}", truncated));
            }
        }

        lines.join("\n")
    }

    fn render_results_markdown(&self, results: &[QueritResultItem], query: &str) -> String {
        if results.is_empty() {
            return format!("_No results for `{query}`._");
        }

        let mut out = format!("# Search results -- `{query}`\n");
        for item in results.iter().take(self.max_results) {
            let title = item
                .title
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or("Untitled");
            out.push_str(&format!("\n## [{title}]({})\n", item.url.trim()));
            if let Some(age) = item.page_age.as_deref() {
                let age = age.trim();
                if !age.is_empty() {
                    out.push_str(&format!("_Page age: {age}_\n\n"));
                }
            }
            if let Some(site) = item.site_name.as_deref() {
                let site = site.trim();
                if !site.is_empty() {
                    out.push_str(&format!("_Site: {site}_\n\n"));
                }
            }
            if let Some(snippet) = item.snippet_text() {
                let truncated = crate::openhuman::util::truncate_with_suffix(&snippet, 500, "...");
                out.push_str(&format!("> {truncated}\n"));
            }
        }
        out
    }

    fn insert_array_filter(args: &Value, key: &str, target: &mut Map<String, Value>) {
        if let Some(value) = args.get(key).filter(|v| v.is_array()) {
            target.insert("include".to_string(), value.clone());
        }
    }

    fn object_or_include_map(value: Option<Value>) -> Map<String, Value> {
        match value {
            Some(Value::Object(map)) => map,
            Some(Value::Array(items)) => {
                let mut map = Map::new();
                map.insert("include".to_string(), Value::Array(items));
                map
            }
            _ => Map::new(),
        }
    }

    fn build_filters(args: &Value) -> Option<Value> {
        let mut filters = args
            .get("filters")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        let mut sites = Self::object_or_include_map(filters.remove("sites"));
        if let Some(include) = args.get("include_domains").filter(|v| v.is_array()) {
            sites.insert("include".to_string(), include.clone());
        }
        if let Some(exclude) = args.get("exclude_domains").filter(|v| v.is_array()) {
            sites.insert("exclude".to_string(), exclude.clone());
        }
        if !sites.is_empty() {
            filters.insert("sites".to_string(), Value::Object(sites));
        }

        let existing_time_range = filters
            .remove("timeRange")
            .or_else(|| filters.remove("time_range"));
        let mut time_range_obj = match existing_time_range.clone() {
            Some(Value::Object(map)) => map,
            Some(Value::String(date)) if !date.trim().is_empty() => {
                let mut map = Map::new();
                map.insert("date".to_string(), json!(date.trim()));
                map
            }
            _ => Map::new(),
        };
        let time_range = args
            .get("time_range")
            .or_else(|| args.get("date"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                let from = args
                    .get("from_date")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let to = args
                    .get("to_date")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                match (from, to) {
                    (Some(from), Some(to)) => Some(format!("{from}to{to}")),
                    (Some(day), None) | (None, Some(day)) => Some(day.to_string()),
                    (None, None) => None,
                }
            });
        if let Some(date) = time_range {
            time_range_obj.insert("date".to_string(), json!(date));
            filters.insert("timeRange".to_string(), Value::Object(time_range_obj));
        } else if !time_range_obj.is_empty() {
            filters.insert("timeRange".to_string(), Value::Object(time_range_obj));
        } else if let Some(other) = existing_time_range {
            filters.insert("timeRange".to_string(), other);
        }

        let mut geo = match filters.remove("geo") {
            Some(Value::Object(map)) => map,
            _ => Map::new(),
        };
        let mut countries = Self::object_or_include_map(geo.remove("countries"));
        Self::insert_array_filter(args, "countries", &mut countries);
        if !countries.is_empty() {
            geo.insert("countries".to_string(), Value::Object(countries));
        }
        if !geo.is_empty() {
            filters.insert("geo".to_string(), Value::Object(geo));
        }

        let mut languages = Self::object_or_include_map(filters.remove("languages"));
        Self::insert_array_filter(args, "languages", &mut languages);
        if !languages.is_empty() {
            filters.insert("languages".to_string(), Value::Object(languages));
        }

        if filters.is_empty() {
            None
        } else {
            Some(Value::Object(filters))
        }
    }

    fn decode_response(value: Value) -> anyhow::Result<QueritSearchResponse> {
        let payload = value
            .get("response_data")
            .and_then(|v| v.get("aiapi_res"))
            .or_else(|| value.get("aiapi_res"))
            .cloned()
            .unwrap_or(value);
        serde_json::from_value(payload).map_err(|e| {
            tracing::warn!("[querit] failed to parse response: {e}");
            anyhow::anyhow!("Failed to parse Querit response: {e}")
        })
    }
}

impl QueritResultItem {
    fn snippet_text(&self) -> Option<String> {
        self.snippet
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                let joined = self
                    .sentence
                    .iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                if joined.is_empty() {
                    None
                } else {
                    Some(joined)
                }
            })
    }
}

#[async_trait]
impl Tool for QueritSearchTool {
    fn name(&self) -> &str {
        self.tool_name
    }

    fn description(&self) -> &str {
        "Search the web in real time using Querit. Returns current results with URLs, \
         snippets, site names, and page age. Supports site include/exclude filters, \
         time ranges, countries, and languages."
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
                "count": {
                    "type": "integer",
                    "description": "Querit-native alias for max_results."
                },
                "filters": {
                    "type": "object",
                    "description": "Querit-native filters object with sites, timeRange, geo, and languages."
                },
                "include_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only fetch results from these domains."
                },
                "exclude_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Exclude results from these domains."
                },
                "time_range": {
                    "type": "string",
                    "description": "Querit date filter: d7, w2, m6, y1, or YYYY-MM-DDtoYYYY-MM-DD."
                },
                "from_date": {
                    "type": "string",
                    "description": "Start date for a Querit date-range filter (YYYY-MM-DD)."
                },
                "to_date": {
                    "type": "string",
                    "description": "End date for a Querit date-range filter (YYYY-MM-DD)."
                },
                "countries": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Country filters, e.g. [\"united states\", \"japan\"]."
                },
                "languages": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Language filters, e.g. [\"english\", \"japanese\"]."
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
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?;

        let api_key = self.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Querit search unavailable: no API key configured. \
                 Set QUERIT_API_KEY or OPENHUMAN_QUERIT_API_KEY, \
                 or add search.querit.api_key to config.toml."
            )
        })?;

        let max_results = args
            .get("max_results")
            .or_else(|| args.get("count"))
            .and_then(Value::as_u64)
            .map(|n| n.clamp(1, 20) as usize)
            .unwrap_or(self.max_results);

        let mut body = json!({
            "query": query,
            "count": max_results,
        });
        if let Some(filters) = Self::build_filters(&args) {
            body.as_object_mut()
                .expect("querit request body object")
                .insert("filters".to_string(), filters);
        }

        let url = format!("{}/search", self.api_url.trim_end_matches('/'));
        tracing::debug!(
            query_len = query.chars().count(),
            max_results,
            timeout_secs = self.timeout_secs,
            "[querit] POST {url}"
        );

        let resp = self
            .http_client
            .post(&url)
            .bearer_auth(api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                tracing::warn!("[querit] request failed: {e}");
                anyhow::anyhow!("Querit search request failed: {e}")
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                status = %status,
                body_len = body_text.len(),
                "[querit] non-2xx response from Querit"
            );
            anyhow::bail!("Querit returned non-2xx status {status}");
        }

        let search_resp = Self::decode_response(resp.json().await.map_err(|e| {
            tracing::warn!("[querit] failed to read response JSON: {e}");
            anyhow::anyhow!("Failed to read Querit response JSON: {e}")
        })?)?;

        if search_resp.error_code != 0 && search_resp.error_code != 200 {
            tracing::warn!(
                error_code = search_resp.error_code,
                error_msg_len = search_resp.error_msg.chars().count(),
                "[querit] application-level error from Querit"
            );
            anyhow::bail!("Querit returned error_code {}", search_resp.error_code);
        }

        tracing::debug!(
            result_count = search_resp.results.result.len(),
            search_id = search_resp.search_id,
            "[querit] search complete"
        );

        let mut result =
            ToolResult::success(self.render_results_plain(&search_resp.results.result, query));
        if options.prefer_markdown {
            result.markdown_formatted =
                Some(self.render_results_markdown(&search_resp.results.result, query));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> QueritSearchTool {
        QueritSearchTool::new(None, None, 5, 15)
    }

    fn tool_with_key() -> QueritSearchTool {
        QueritSearchTool::new(Some("test-key".into()), None, 5, 15)
    }

    #[test]
    fn test_tool_name() {
        assert_eq!(tool().name(), "querit_search");
        assert_eq!(
            QueritSearchTool::new_web_search_tool(None, None, 5, 15).name(),
            "web_search_tool"
        );
    }

    #[test]
    fn test_parameters_schema() {
        let schema = tool().parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["query"].is_object());
        assert!(schema["properties"]["time_range"].is_object());
        assert!(schema["properties"]["countries"].is_object());
    }

    #[test]
    fn test_render_plain_with_data() {
        let results = vec![QueritResultItem {
            url: "https://example.com/a".into(),
            page_age: Some("2026-05-01 00:00:00".into()),
            title: Some("First Result".into()),
            snippet: Some("First result snippet.".into()),
            site_name: Some("Example".into()),
            site_icon: None,
            sentence: vec![],
        }];

        let result = tool().render_results_plain(&results, "test");
        assert!(result.contains("via Querit"));
        assert!(result.contains("First Result"));
        assert!(result.contains("https://example.com/a"));
        assert!(result.contains("Page age: 2026-05-01"));
        assert!(result.contains("Site: Example"));
        assert!(result.contains("First result snippet."));
    }

    #[test]
    fn test_build_filters_maps_supported_fields() {
        let filters = QueritSearchTool::build_filters(&json!({
            "include_domains": ["example.com"],
            "exclude_domains": ["spam.test"],
            "time_range": "d7",
            "countries": ["united states"],
            "languages": ["english"]
        }))
        .expect("filters");

        assert_eq!(filters["sites"]["include"][0], "example.com");
        assert_eq!(filters["sites"]["exclude"][0], "spam.test");
        assert_eq!(filters["timeRange"]["date"], "d7");
        assert_eq!(filters["geo"]["countries"]["include"][0], "united states");
        assert_eq!(filters["languages"]["include"][0], "english");
    }

    #[test]
    fn test_build_filters_preserves_native_filters_payload() {
        let filters = QueritSearchTool::build_filters(&json!({
            "filters": {
                "sites": {
                    "include": ["techcrunch.com"]
                },
                "timeRange": {
                    "date": "m3"
                },
                "geo": {
                    "countries": {
                        "include": ["united states"]
                    }
                },
                "languages": {
                    "include": ["english"]
                }
            }
        }))
        .expect("filters");

        assert_eq!(filters["sites"]["include"][0], "techcrunch.com");
        assert_eq!(filters["timeRange"]["date"], "m3");
        assert_eq!(filters["geo"]["countries"]["include"][0], "united states");
        assert_eq!(filters["languages"]["include"][0], "english");
    }

    #[test]
    fn test_build_filters_normalizes_native_shorthand_values() {
        let filters = QueritSearchTool::build_filters(&json!({
            "filters": {
                "sites": ["example.com"],
                "time_range": "m3",
                "geo": {
                    "countries": ["united states"]
                },
                "languages": ["english"]
            }
        }))
        .expect("filters");

        assert_eq!(filters["sites"]["include"][0], "example.com");
        assert_eq!(filters["timeRange"]["date"], "m3");
        assert_eq!(filters["geo"]["countries"]["include"][0], "united states");
        assert_eq!(filters["languages"]["include"][0], "english");
    }

    #[test]
    fn test_build_filters_combines_date_range() {
        let filters = QueritSearchTool::build_filters(&json!({
            "from_date": "2026-01-01",
            "to_date": "2026-01-31"
        }))
        .expect("filters");
        assert_eq!(filters["timeRange"]["date"], "2026-01-01to2026-01-31");
    }

    #[test]
    fn test_decode_response_accepts_wrapped_aiapi_shape_and_sentence() {
        let parsed = QueritSearchTool::decode_response(json!({
            "response_data": {
                "aiapi_res": {
                    "error_code": 0,
                    "search_id": 42,
                    "results": {
                        "result": [
                            {
                                "url": "https://example.com",
                                "title": "Wrapped",
                                "sentence": ["Sentence excerpt."]
                            }
                        ]
                    }
                }
            }
        }))
        .expect("wrapped response");

        assert_eq!(parsed.results.result[0].title.as_deref(), Some("Wrapped"));
        assert_eq!(
            parsed.results.result[0].snippet_text().as_deref(),
            Some("Sentence excerpt.")
        );
    }

    #[tokio::test]
    async fn test_execute_missing_query() {
        let result = tool_with_key().execute(json!({})).await;
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
    async fn test_execute_posts_to_querit_and_renders_results() {
        use axum::{extract::Json, routing::post, Router};
        use serde_json::Value;

        let app = Router::new().route(
            "/search",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["query"], "test query");
                assert_eq!(body["count"], 3);
                assert_eq!(body["filters"]["sites"]["include"][0], "example.com");
                Json(json!({
                    "took": "12ms",
                    "error_code": 200,
                    "error_msg": "",
                    "search_id": 42,
                    "query_context": { "query": "test query" },
                    "results": {
                        "result": [
                            {
                                "url": "https://example.com/result",
                                "title": "Querit Result",
                                "snippet": "Content from Querit search.",
                                "page_age": "2026-05-01 00:00:00",
                                "site_name": "Example"
                            }
                        ]
                    }
                }))
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        let tool = QueritSearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
        let result = tool
            .execute(json!({
                "query": "test query",
                "max_results": 3,
                "include_domains": ["example.com"]
            }))
            .await
            .expect("execute() should succeed");

        assert!(result.output().contains("Querit Result"));
        assert!(result.output().contains("https://example.com/result"));
        assert!(result.output().contains("Content from Querit search."));
    }

    #[tokio::test]
    async fn test_execute_non_success_status_does_not_expose_response_body() {
        use axum::{http::StatusCode, routing::post, Router};

        let app = Router::new().route(
            "/search",
            post(|| async {
                (
                    StatusCode::BAD_REQUEST,
                    "sensitive query context should stay private",
                )
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        let tool = QueritSearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
        let err = tool
            .execute(json!({
                "query": "private search",
                "max_results": 3
            }))
            .await
            .expect_err("non-2xx responses should fail");
        let message = err.to_string();

        assert!(message.contains("Querit returned non-2xx status 400 Bad Request"));
        assert!(!message.contains("sensitive query context"));
    }

    #[tokio::test]
    async fn test_execute_app_error_does_not_expose_error_msg() {
        use axum::{extract::Json, routing::post, Router};
        use serde_json::Value;

        let app = Router::new().route(
            "/search",
            post(|Json(_body): Json<Value>| async move {
                Json(json!({
                    "took": "3ms",
                    "error_code": 400,
                    "error_msg": "validation failed for sensitive query context",
                    "search_id": 42,
                    "query_context": { "query": "sensitive query context" },
                    "results": { "result": [] }
                }))
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        let tool = QueritSearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
        let err = tool
            .execute(json!({
                "query": "sensitive query context",
                "max_results": 3
            }))
            .await
            .expect_err("application-level errors should fail");
        let message = err.to_string();

        assert_eq!(message, "Querit returned error_code 400");
        assert!(!message.contains("validation failed"));
        assert!(!message.contains("sensitive query context"));
    }
}
